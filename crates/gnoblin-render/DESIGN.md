# The window node: the node that is the window

Status: design, not implemented. The scene tree in this crate holds the window node, so the rationale lives here; open
work is in Beads.

Owner decision (Doop choice 47, restated): shells must say exactly what a window is. "Make it so they need to wrap in the
Window/Client parent node, and this is what we use to determine what a window is. Anything inside that node means it's a
window."

Terms. The "window node" is the tree node of class `window`. The Rust SDK calls its handle `Window`; the object model calls
it `Client` ("a toplevel window the compositor manages, as a handle over its tree node",
crates/gnoblin-core-impl/src/model/client.rs:1). One node, three names.

## 1. The model in one page

1. Every managed toplevel (xdg or X11) has exactly one window node. The core makes it, when the window first has content.
   The shell never makes, destroys or re-classes one.
2. The window node has one compositor-made child, the toplevel surface node (class `surface`, role `toplevel`): the
   client's pixels. The client's subsurfaces and popups hang from it, as they hang from their parents in the protocol.
3. The window is the window node's subtree: the node, its own `effects`, the toplevel surface node and everything under
   it, and every node the shell inserted anywhere under the window node. Frame, titlebar, shadow, border, glow, badge,
   resize handles: if the shell wants it counted as the window, it inserts it under the window node. Anything not under
   the window node is not the window, however it is placed on screen.
4. Capture of a window (ext-image-copy-capture on an ext-foreign-toplevel source, and the portal's window cast) copies
   exactly that subtree, drawn on its own: no output crop, no covering window, whatever output(s) the window is on.
5. A core-only shell writes nothing extra. The core makes the window node and puts the client inside it; a window with no
   shell decoration is a complete window.

New in the core: the toplevel surface becomes a node; the subtree is drawn and captured offscreen; three refusals make the
rule unbreakable. New in std: nothing structural (std already parents frames under the window node). Removed: the
output-crop window capture, the window-geometry crop hacks, and the `gnoblin_view_source_v1` Wayland protocol.

## 2. What the code does today (evidence)

### 2.1 The window node and who makes it

- Class `window` derives from `node` (crates/gnoblin-schema/schema/tree/window.toml:3-20). Its `x/y/width/height` are the
  window geometry, "the part of its surface the user sees as the window"; a shadow or title bar the client draws outside
  it "is not the window" (window.toml:6-12).
- The compositor makes the node in `Composer::sync_with`: a `NodeSpec` with `parent: None`, `kind: Visual`
  (crates/gnoblin-backend/src/render/compose.rs:3562-3566), class `window` (compose.rs:3577). It is made when the window
  first has content, not at map (compose.rs:1100-1101 comment; 3504-3516, where no content drops the node). It starts
  unparented and the shell places it (crates/gnoblin-render/src/transaction.rs:329; scene_ids.rs:12-19: the compositor
  "creates the windows", the shell "may destroy only the nodes it created").
- Hole: `new_node` checks only `has_class` (transaction.rs:632-651), and a read-only key may be set in the creating
  transaction (`check_writable`, transaction.rs:739-755). So today a shell can make a fake `window` node, or a fake
  `surface` with a chosen `role`. Not checked: whether the IPC host refuses it earlier.
- The compositor keeps its own `WindowNode` record per window (compose.rs:366-460), keyed by `WindowId`; `window_of(node)`
  is a linear search on `entry.node` (compose.rs:2088-2092).

### 2.2 The client surface is not a node

- The window node IS the toplevel surface: the buffer is the node's own content, drawn at the node's origin less
  `surface_offset` (compose.rs:366-376; `push_window`, 3630-3688). The node's rectangle is the window geometry; a buffer
  larger than it (CSD shadow) is drawn through `draw_extent` and hit through `hit_extent` (`apply_extents`,
  compose.rs:1172-1204; `own_placement`, 1160-1167; `Scene::set_draw_extent`, crates/gnoblin-render/src/scene.rs:1002).
- Only popups and subsurfaces are child nodes: "children of the window's node that the compositor draws ... the shell cannot
  reparent or destroy them" (surface.toml:10-14). `sync_tree` hangs a surface with no parent surface from the window node
  (compose.rs:2698-2716). The schema calls `surface` "a client surface that is not a toplevel" (surface.toml:4).
- The "cannot reparent" rule works: `Scene::attach` returns `NotOwner` for the shell when the node was
  `parented_by == Compositor` (scene.rs:724-726; `insert_node`, transaction.rs:657-679).

### 2.3 Where std puts frames, titlebars, shadows, borders

- Frames are children of the window node. `window_frame::outlet` makes an outlet canvas and calls `ui.mount(cid, node)` with
  the window's node (crates/gnoblin-std-impl/src/frame/window_frame.rs:186-190); `mount` is `insert_into(parent)`
  (std-impl/src/ui/tree.rs:240-249). The outlets are "stacked in the window's children" (frame/mod.rs:3-9).
- Titlebar, edges and corners are canvases with `placement` `frame` (default), `over` or `under`
  (schema/tree/canvas.toml:19-26). `window_shape` unions the window rectangle with drawn `frame` canvas children
  (compose.rs:3264-3285); `under` canvases paint before the window (`canvases_under_windows`, compose/canvases.rs:121-146).
- Shadow, border and blur are not nodes: they are entries of the window node's `effects` (node.toml:167-180), written by
  the std style cascade (std-impl/src/style/cascade.rs:9-62). The border ring is a layer over the whole frame
  (`push_ring`, compose.rs:3292). `corner_radius` rounds content, decoration, borders and shadow as one shape
  (node.toml:98-115).
- Std places the window node itself under the root for stacking (std-impl/src/policy/stack.rs:66-116;
  frame/actions.rs:253-268). That stays legal.
- No std or core code puts a window node under another window node (every `insert_into` in std-impl and core-impl was
  read). Yet docs/xr.md:112 ("the dialog is a child of the parent window") and data/js/gnoblin-std/host/xr_bridge.ts:71-74
  (`parentKind(client) === "window"`) assume it. A latent inconsistency.

### 2.4 How compose.rs draws a window and a View

- Output frame: `compose` walks `self.order` and calls `push_window` per window, interleaving canvases, border rings and Views
  by paint rank (compose.rs:3072-3225, call at 3149). A shell node outside the window is drawn by the same walk, in scene
  order.
- A View has `source` (a node) and `camera` (view.toml:4-20). `render_views` redraws it only when it or a node of its source
  subtree is dirty (compose/views.rs:299-374); `draw_view` renders the subtree through `view_rig` (views.rs:376-441), on the
  CPU, or as a `ViewScene` the GPU draws (views.rs:444-527).
- A View does NOT draw a window as the output does. (a) `ViewEmit::layer` builds layers with `corner_radius: 0.0` and
  `effects: Effects::new()` (views.rs:138-150): no rounding, shadow, border. (b) The buffer is cropped to the geometry
  (`view_crop`, `geometry_crop`, views.rs:163-215, used at 559-575 and 601-616): "The buffer lies wholly outside the window
  geometry: nothing of it is the window" (views.rs:567). (c) The default camera puts the source's local origin at the View's
  top-left (views.rs:649-662), so a titlebar at negative `y` is cut.
- Secure: `find_secure` takes every window marked secure and every node whose `secure` is true, "and everything below
  either" (compose.rs:976-996; `set_window_secure`, 1838-1850). A View's capture picture replaces secure nodes
  (views.rs:40-52, 398-440).

### 2.5 How window capture works now

- A foreign-toplevel source resolves the published identifier `gnoblin-<16 hex>` of a `WindowId`
  (protocols/ext-foreign-toplevel-list-v1/src/lib.rs:37-45; protocols/ext-image-capture-source-v1/src/lib.rs:65-68) and
  records `Target::Window` (gnoblin-wayland/src/glue/ext_image_capture_source_v1.rs:41-53).
- `Target::Window` is a crop of the last output frame. `capture_targets` gives each window an output, an origin
  (`shown - monitor`, scaled) and the size of its geometry (gnoblin-wayland/src/protocols/capture_shared.rs:901-930; a window
  with no output is skipped, 916). `Pixels::copy` reads `frames[output]` at that origin (capture_shared.rs:526-557); the
  PipeWire path does the same (`PlanePixels::copy_into`, 575-600); portal casts reach it through `CastSource::Window`
  (gnoblin-compositor/src/ipc/consent.rs:250).
- So: any window above the target is in the copy; a titlebar above the client, a frame shadow and a CSD shadow are cut; a
  window on two outputs shows the part on one (`Targets::output`, capture_shared.rs:173-180).
- The lock embargo is asked at every frame; a granted stream continues through a lock only for an output, and a window
  stream is refused (gnoblin-capture/src/table.rs:375-391).

### 2.6 What XR consumes

- The extension is a separate process (docs/xr.md:34-38). Its connection has role `view` and sees only `wl_shm`,
  `ext_image_copy_capture_manager_v1` and `gnoblin_view_source_manager_v1` (crates/protocols/api/src/access.rs:226-249); the
  two ext capture-source managers are hidden from it on purpose (access.rs:222-228).
- The stock shell's room holds one `View` per window with `source` = the window node, size = the window size, `input =
"forward"` (data/js/gnoblin-std/extensions/xr/room.ts:108-119; docs/xr.md:98-115). The program calls
  `gnoblin_view_source_manager_v1.create_source(node_hi, node_lo)` for each View (extensions/gnoblin-xr/crates/xr-capture/
  src/wayland.rs:338-343; protocols/gnoblin-view-source-v1.xml; gnoblin-wayland/src/protocols/gnoblin_view_source_v1.rs) and
  copies the View's picture: a geometry-cropped window (2.4).
- `auto`/`layers` composition: flat View quads. `projection`: each panel View gets the head camera so the compositor draws
  a stereo picture shown as a projection layer (xr/viewport.ts:27, 90-96; docs/xr.md:154-160).
- Ray input does not use the picture: the scene forwards it through `through_view` to `Landing::Window`
  (compose/views.rs:824-858).
- The View stays for other uses: `View:capture(path)` stills (gnoblin-wayland/src/view_capture.rs:1-8) and portal
  `CastSource::View` (consent.rs:249). Only the Wayland View-source protocol goes.

## 3. The design

### 3.1 Tree shape

```
root
  window                  core-made; rect = window geometry; the shell may place and transform it
    surface role=toplevel core-made; the client buffer, rect = whole buffer, placed at -surface_offset
      surface role=subsurface ...   core-made, children of their parent surface; `behind` as today
      surface role=popup ...        core-made
    canvas "titlebar"     shell-made, placement frame
    canvas "backdrop"     shell-made, placement under
    canvas "badge"        shell-made, placement over
    (window.effects: shadow, border; window.corner_radius)
```

Draw order is today's: `under` canvases, the toplevel surface and its subsurfaces, `frame`/`over` canvases, the border
ring. `placement` keeps its meaning; the core reads `under` as "sorts before the toplevel surface node".

### 3.2 Rules the core enforces

- R1. The core makes the window node and its toplevel surface node together, and destroys them together. One window node
  per toplevel; one toplevel surface node per window node.
- R2. A shell cannot make a `window` or a `surface` node: `new window` and `new surface` are refused `not-owner` (new check
  in `new_node`, transaction.rs:632). Closes the hole in 2.1.
- R3. A shell cannot move or destroy the toplevel surface node, a subsurface or a popup. Already `not-owner` through
  `parented_by == Compositor` (scene.rs:724) and the creator rule; the new node is created that way. So the client surface
  cannot be reparented out of its window node.
- R4. A window node cannot be inside another window's subtree and cannot contain one. An `insert` that would put a `window`
  under a node with a `window` ancestor, or a node with a `window` descendant under a `window`, is refused with a new code
  `window-nested` (check in `insert_node`, transaction.rs:657-679; depth is bounded by MAX_DEPTH, scene_ids.rs:36). Every
  node therefore has at most one window answer. Dialogs are their own windows (question 2).
- R5. The shell may hide, fade or transform the toplevel surface node (existing node/surface properties) but not set its
  `x/y/width/height`, which are the buffer's place. Hidden is hidden in captures too.
- R6. The core never inspects shell nodes to decide what a window is. Identity is the parent chain. There is no
  "represents" property and no marker.

### 3.3 Schema members added (all appended; ABI in 3.9)

- `surface.role` gains `toplevel`; the class comment loses "not a toplevel" (surface.toml:4).
- `window.toplevel_id` (string, read-only): the ext-foreign-toplevel identifier (`gnoblin-<hex>`). It lets an out-of-process
  tree reader (XR) pair a node with its foreign-toplevel handle; the tree exposes no `WindowId` today.
- `window.extent` (list<double> [x, y, width, height], read-only): the rectangle of the window's drawn subtree in the
  window node's own space (3.5). Written when it changes, at most once a frame.
- No change to `view.toml`. A View whose `source` is a window now draws it correctly (3.6).

### 3.4 What a core-only shell writes

Nothing. A core-only shell places and focuses windows (examples/01-minimal-core-shell/src/lib.rs:1-14, 25-45). The core
already makes the window node; after this change it also makes the client's node inside it, so the window (geometry rectangle,
client picture) is complete, and capture and XR work with zero shell lines.

A core-only shell that wants a frame makes one canvas and inserts it under the window node: `new canvas`, set `content`,
`insert_into(window)` (the call std's `mount` uses, ui/tree.rs:246-248). A sibling of the window under the root is not the
window, and the docs say so in one sentence.

### 3.5 Eye candy inside and outside the client rectangle

- Inside or outside the client rectangle: a child of the window node or of any node under it. Negative coordinates are fine.
  Nothing clips it to the geometry unless the shell sets `clip` on an ancestor (node.toml:122-126) or `corner_radius` on
  the window.
- The window node's own `effects` belong to the window.
- The capture area is `extent`: the union, in window-node space, of the window rectangle and the draw rectangle of every
  drawn node of the subtree, grown by effect padding (the scene already computes this, `visual_local`, scene.rs:282-286),
  honouring ancestors' `clip`. It includes the client's own CSD margin (the toplevel surface node is the whole buffer; no
  `view_crop`) and, by the owner's rule, popups (question 3).
- The picture's top-left is the extent's top-left, so the geometry sits at `(-extent.x, -extent.y)`. A consumer that wants
  only the geometry crops.
- The window node's own `visible`, `opacity` and placement transform (workspace fade, minimise, shell scale) are NOT applied:
  a capture is the window as it is, not as the shell now shows it. Descendants' `visible`, `opacity`, `transform`, `clip` and
  `effects` are applied. A window that has no node (the compositor drops it when it is not shown: compose.rs:3504-3516) has
  no picture: its session gets no frames and is not stopped until the window is gone.
- `blur` (backdrop) is omitted: there is no backdrop. Shadow, border and corner radius are drawn as far as the renderer
  draws them (node.toml:176-180: GLES and Vulkan every kind, software every kind but `shader`); `skipped` already says
  what is not.

### 3.6 How the compositor changes

1. Toplevel surface node: `sync_with` makes it with the window node (compose.rs:3552-3600) and `WindowNode` records its id.
   `surface_offset` becomes its transform (it sits at `-surface_offset`; its rectangle is the buffer), so the buffer no longer
   needs the window node's `draw_extent`/`hit_extent` (compose.rs:1172-1204), and `push_window` (3630) draws that node.
2. `window_of` and `window_of_node` (compose.rs:2088, 2506) also map the toplevel surface node to its window, so the pick
   and `Landing::Window` (views.rs:824-837) keep working. `pointer_input` still takes the window node
   (transaction.rs:707-735) and adds the surface offset.
3. `sync_tree` hangs parent-less client surfaces from the toplevel surface node (compose.rs:2698-2716). `behind` ordering is
   unchanged (`push_children`, compose.rs:3690-3730; scene.rs:54-57).
4. Window picture: a compositor-private picture per window with at least one capture session, using the View machinery
   (`draw_view`, `view_scene`, `emit_view_node`) with `source` = the window node, an orthographic rig at the `extent` origin,
   size = extent x scale (3.8), under a private key (not a scene node, not shown to the shell). Needs: `draw_view` to take a
   source and rig instead of reading them from a View node (views.rs:376-441); `ViewEmit::layer` to carry the node's
   `corner_radius`, `silhouette`, `effects` (views.rs:138-150); `emit_view_node` to ignore the root's own
   `visible/opacity/transform`.
5. Delete `view_crop`, `geometry_crop`, `ViewCrop` and the crop option of `view_sources`/`window_pictures` (views.rs:152-215,
   540-616). A View of a window now shows the CSD margin too (3.7, section 8).
6. The output frame changes only in that the toplevel surface is a child node.

### 3.7 Capture: which standard protocols serve it

No private protocol.

- `ext_foreign_toplevel_list_v1` gives a window's handle; `ext_foreign_toplevel_image_capture_source_manager_v1` makes a
  source from it; `ext_image_copy_capture_manager_v1` copies it. All three exist (docs/wayland-protocols.md:55-59).
- `Target::Window` is re-implemented over the window picture. `Targets::bounds` returns the picture size and `copy` reads the
  picture, the way `Target::View` already does (capture_shared.rs:526-545, 575-590). Delete `CapturedPlacement`, the window
  origin/output lookup and `capture_layout`'s window arm (capture_shared.rs:140-200, 705-717, 901-930). Portal casts
  (consent.rs:250) and the PipeWire export (gnoblin-pipewire-export/src/lib.rs:246) take the fix with no code of their own.
- Sessions see `buffer_size` when the extent changes (native to the protocol). Damage is the whole picture per redraw, as a
  View's is today (capture_shared.rs:784); finer damage later.
- Lock and secure. The embargo is per frame and refuses window targets under any lock (table.rs:375-391). The picture is
  drawn in its capture variant only (views.rs:398-440), so `secure` on the window node masks the surface, the frame and every
  popup together, and `secure` on one node inside masks just that one. A secure shell node outside the window is not in the
  picture at all. The foreign-toplevel list still publishes a secure window's title and app id; unchanged.

### 3.8 Multi-output, scale, performance

- Multi-output: the picture is drawn once in window space, so a window spanning two outputs is whole. Scale is the largest
  scale of the outputs the window is on (the `canvas_scale` rule, canvases.rs:60), else one. A window with no output is now
  capturable.
- Cost: the picture exists only while a session or cast is open, is redrawn only when a node of the subtree is dirty (the
  View rule, views.rs:299-374) and serves every session on that window. On the GPU path it is a texture read back only when
  a frame is copied (`view_awaits_readback`, capture_image_copy.rs:123-130). A still window costs nothing; a moving one costs
  a subtree draw per presented frame per captured window, linear in captured windows, not in output area. XR's 12 panels
  already cost 12 subtree draws as Views (2.6). Memory counts against the GPU budget like Views.
- Pacing is the session's (table ticks); `fps` does not exist without a View node.

### 3.9 ABI impact

- `abi.lock` says `released none` (crates/gnoblin-core-abi/abi.lock:12): the ABI is open until the release commit, so even a
  re-meaning is allowed now and not after. The tree is addressed by schema names, not function slots; the additions are a new
  property value, two new properties and a new error code, all allowed in a minor (docs/abi-compatibility.md, table of
  compatible changes). No function-table field or export changes.
- The one re-meaning: "the window node has the client's pixels" becomes "the window node has the client's surface as a
  child". It changes what a snapshot holds (one more node per window), so it must land before `released` is set.
- A shell built earlier keeps loading. One that made `window`/`surface` nodes is refused (R2); none in the repo does.

### 3.10 Reload

- The window node and the toplevel surface node are compositor-made and survive every reload (ARCHITECTURE.md:129). Capture
  sessions are bound to the window, not to a shell node, so they continue.
- A shell node inside a window follows the existing rule: a node a generation made persists; one found by `adopt_node` is
  the same node in the next generation; an `ephemeral` one it does not adopt is destroyed (core-impl/src/session.rs:745-822;
  docs/rust-shell.md:208). Membership is the parent link, so an adopted node is still inside its window with no
  re-declaration. What the new generation does not recreate or adopt leaves the picture; the worst case is the bare client.
  The client is never hidden by a reload, which is the main reason for rejecting the explicit option.
- std rebuilds frames from its own reconcile turn (frame/decorations.rs:1-8). No change.

### 3.11 Popups, subsurfaces, layer surfaces

- Subsurfaces and popups are children of the toplevel surface node or of their protocol parent. They are in the window
  picture. They keep `placement_override` and `stack` (surface.toml:51-66).
- A menu opening grows `extent`; sessions get a `buffer_size` update and XR remakes a swapchain (xr surfaces.rs:1-2;
  docs/xr.md:60). One remake per size change.
- Layer-shell surfaces are `surface` role `layer`, unparented (surface.toml:10-14): not windows, not window-capturable.
  Section 4.

### 3.12 What std changes

- Frames already mount under the window node (window_frame.rs:186-190). No change.
- Remove the window-in-window assumption: `parentKind(...) === "window"` in xr_bridge.ts:71-74, runtime.ts:165-168 and the
  `dialog` argument of `room.reconcile` (room.ts:130-141).
- A std test walks the nodes std makes for a window (frame, style, menu) and asserts the window is in the ancestor chain.
  None is known to be a sibling (2.3).
- Document the contract in docs/js-window-frame.md: what you insert under the window node is the window.

## 4. Shell-drawn parts that are not windows (bars, docks, overlays, the room)

Not windows, so not in a window capture. How XR gets them without a private protocol:

1. Client bars and docks are layer-shell surfaces, not toplevels; ext-foreign-toplevel cannot name them. The standard route
   is to put them on an output and capture the output (`ext_output_image_capture_source_manager_v1`). XR's HUD is the virtual
   output attached to the viewer (docs/xr.md:429-443, which today throws `unsupported`). That is a separate small
   decision about virtual-output backend support, independent of the window node.
2. Shell-drawn canvases (a std panel, a shell overlay) are drawn on the output, so the same output carries them.
3. The stereo `projection` composition (viewport.ts:27, 90-96) needs the compositor to render a perspective picture of a View.
   No standard source can ask for that and the window model does not serve it. Question 1.

## 5. XR port

- Per panel the program reads, from the tree, the View (placement, `input`) and the window node it shows, plus the window's
  `toplevel_id` and `extent`. It binds `ext_foreign_toplevel_list_v1`, finds the handle with that identifier, makes a source
  with `ext_foreign_toplevel_image_capture_source_manager_v1.create_source`, and copies with the existing session code
  (xr-capture/src/wayland.rs:338-496). The quad is the capture buffer size, offset from the View's top-left by `extent.x/y`,
  so it lines up with the ray's window coordinates (views.rs:844-857).
- `VIEW_INTERFACES` becomes `wl_shm`, `ext_image_copy_capture_manager_v1`, `ext_foreign_toplevel_list_v1`,
  `ext_foreign_toplevel_image_capture_source_manager_v1` (access.rs:226-249; the tests at access.rs:626-680 change).
- Delete `Global::ViewSource`, protocols/gnoblin-view-source-v1.xml, gnoblin-wayland/src/protocols/gnoblin_view_source_v1.rs,
  tests/view_source.rs, tests/view_capture_e2e.rs, the xr-capture bindings and the xtask/dupes-allow.toml entry, and fix the
  comment at view_capture.rs:5. ARCHITECTURE.md:117 ("No custom Wayland protocols") becomes true.
- The `view` role then sees every toplevel's title and app id; it already reads the same fields in the tree snapshot
  (docs/xr.md:37-38), so nothing widens.
- A secure window: XR receives the masked or omitted picture. The room still skips `client.secure` (room.ts:140).
- A dialog is its own toplevel and capture; XR anchors its panel to the parent's using `parent_window` (window.toml).

## 6. Alternatives and why not

- (b) Explicit: the shell marks node N as client C's window and the client stays hidden until it does. A core-only shell then
  needs boilerplate to see any window (the spec forbids it); every reload and every shell crash hides every client
  (ARCHITECTURE.md:129: compositor-owned state "never lived in the shell"); and each shell invents its own node, so "outside the
  node is not the window" becomes a convention.
- (c) Hybrid: the core node by default, the shell may re-point it. Two answers to "what is the window" bring the ambiguity
  back, and capture needs the explicit path anyway.
- (a) The window node keeps the client pixels as its own content, with no toplevel surface node. Cheapest and closest to
  today, but the pixels stay invisible to the tree: a shell cannot put a node between client and frame, cannot hide or clip
  the client alone, popups hang from a non-node, and `own_placement`/`draw_extent`/`view_crop` stay. If the surface-node
  refactor proves too big, (a) is the fallback: rules R1, R2, R4-R6 and the capture design hold; only 3.1 and 3.6 steps 1-3, 5
  change.
- Keep the output crop and hide other windows during capture: cannot show frame parts outside the geometry and disturbs
  the display.
- A private window-picture protocol for XR: rejected by ARCHITECTURE.md:117 and by the owner.

## 7. Lanes, sizes, order

Rough working days for one person who knows the area.

| Lane                        | Work                                                                                                                                                                                                                                                                                                                                                                            | Size                                                         |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| core (schema, scene)        | `surface.role=toplevel`, `window.toplevel_id`, `window.extent`, `window-nested`; R2 and R4 in transaction.rs; extent computation over a subtree with clips; schema, abi.lock, catalog hash, SDK regeneration                                                                                                                                                                    | M (3-4)                                                      |
| backend / compose           | toplevel surface node and its sync; `window_of`; `sync_tree` parent; buffer placement off the window node; pick and `Landing`; delete crop code; publish `extent`; window picture (`draw_view` on an explicit source and rig, effects and radius in `ViewEmit::layer`, ignore root visible/opacity/transform, per-session lifetime) on the CPU, GLES, software and Vulkan paths | L (8-12)                                                     |
| protocols (gnoblin-wayland) | `Target::Window` over the window picture; drop `CapturedPlacement`; `buffer_size` on extent change; GPU readback for windows; delete the view-source protocol, its access entry, tests and dupes entry                                                                                                                                                                          | M (4-5)                                                      |
| std                         | the ancestor-chain test; remove `parentKind === "window"`; contract docs                                                                                                                                                                                                                                                                                                        | S (1-2)                                                      |
| XR port                     | bind toplevel list and source manager; capture by identifier; quad offset from `extent`; role globals; drop the View-source client; `projection` per question 1                                                                                                                                                                                                                 | M-L (5-8, plus about 5 if projection moves into the program) |
| docs                        | docs/xr.md, docs/js-window-frame.md, docs/shell-api, docs/wayland-protocols.md, window.toml and surface.toml comments, CHANGELOG                                                                                                                                                                                                                                                | S (2)                                                        |
| tests                       | below                                                                                                                                                                                                                                                                                                                                                                           | M (4-5)                                                      |

Order: 1 core schema and refusals (blocks the rest). 2 backend toplevel surface node and picks. 3 window picture, then the
protocol rewire. 4 std, in parallel with 3. 5 XR port after the rewire, with the role globals. 6 delete the view-source
protocol last, when XR no longer binds it. Docs and tests travel with each lane.

Tests, each stating what must be true:

- Core: `new window` and `new surface` from a shell are `not-owner`; moving the toplevel surface is `not-owner`; `window-nested`
  both ways; one window node and one toplevel surface node per window, both gone with it.
- Compose: a decorated window (titlebar above, shadow canvas, CSD margin) captured has size = extent and the titlebar pixels;
  a window stacked over it is not in the picture; a window on two outputs is whole. Bases: decorated_window_tests.rs,
  surface_tree_tests.rs, view_secure_tests.rs.
- Secure and lock: marking the window, a node inside it and a node outside it mask what they should; a window session is
  stopped under a lock; nothing leaks in the first frame after unlock.
- Protocol: foreign-toplevel source plus ext-image-copy-capture gives the window picture; a portal window cast gives the same
  (bases: gnoblin-wayland/tests/capture.rs, dmabuf_capture.rs).
- Reload: a window with a std frame is captured across a reload; frame before, bare client at worst during, frame after, never
  blank.
- XR: the panel quad lines up with the ray hit using `extent`; the `view` role sees exactly the four interfaces.

## 8. What breaks

- Anything that counted a window's children or read the window node's pixels as its own sees one more node per window.
  `window > canvas` selectors still match; `window > surface` is new.
- Window captures (portal, XR) get bigger pictures: the CSD margin and frame parts outside the geometry are in. A consumer
  that wants the geometry crops with `extent`.
- A popup opening resizes the capture buffer (3.11).
- A View of a window shows rounding, shadow, border and the CSD margin it used to crop.
- The window-in-window assumption in std XR code and docs/xr.md:112 goes.
- XR `projection` composition moves into the XR program (decision 1), so the compositor's stereo View picture goes.

## 9. Decisions on the open points

Decided by the agent on 2026-11-26 (the owner asked not to be asked low-level questions); recorded on Doop.

1. Stereo `projection` in XR: the XR program projects the flat window pictures itself (it owns the GPU copy path in
   xr-gpu). The compositor-rendered stereo View leaves the XR path, so no private View source stays.
2. Dialogs: a dialog is its own window and its own capture. XR shows it as its own panel, anchored to the parent's panel by
   `parent_window`. docs/xr.md:112 changes with the XR lane.
3. Popups are inside the window and count towards `extent`. The capture buffer resizes when a menu opens, so a menu past the
   window edge shows in XR.
4. Bars, docks and overlays (section 4): a separate decision about virtual-output backend support, tracked in Beads, not part
   of the window-node lanes.

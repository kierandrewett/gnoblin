# JS runtime change log: UI toolkit, stock shell UI, style and transitions

The finished work of the UI toolkit (P7), the stock shell UI, window style (P8a) and transitions. Part of the [JS/TS
runtime design](js-runtime.md).

## P7 UI toolkit status

The shell UI toolkit (lane JsUi): `Canvas`, `Box`, `Text`, `Icon`, `Grid` (`data/js/gnoblin/ui.ts`) and the stock
`Titlebar` and `Decorations` (`data/js/gnoblin/titlebar.ts`), over the host functions of `crates/gnoblin-js/src/ui.rs`
(`__gnoblinHost.ui`). User page: `docs/js-ui.md`. Ordinary ES classes and fields: no class registry, no bound
properties.

- **Frame.**

    After the layouts, `JsShell::frame` calls the module's `ui` entry (`runUiFrame`). It takes the answers to measurements
    and the pointer events parked for the toolkit.

    Then it updates every canvas tree that has no canvas parent: `refresh()` (read live state), `arrange()` (a `Box` row or
    column, or a `Grid`), then the node's geometry and paint list are written when they differ from the last write.

    A change is therefore one frame of latency and a frame that changed nothing sends nothing
    (`nothing_is_written_again_for_a_frame_that_changed_nothing`). The Lua shell re-evaluated a binding when a property it
    read changed; the polling here costs one `draw()` per canvas per frame and needs no change tracking.

- **Nodes.** A canvas makes its node when it is constructed (`ui.create`: `New` in the frame's transaction, the id made
  from the attach epoch like `gnoblin-lua`'s) and is destroyed with `destroy()`.

    A script can write only nodes the toolkit made (`not-owned` otherwise); the geometry keys are `x`, `y`, `width`,
    `height`, `opacity` and the flags `visible`, `input`, `clip`, and `content` is the paint list.

    When the compositor destroys a window its canvases go with it, and the frame's destroyed list tells the toolkit
    (`uiNodeGone`); a reload's `retire` destroys the old generation's canvases (`cancelAll` calls `uiReleaseAll`).

- **Paint list.** `encodePaintList` writes the bytes the Lua encoder wrote (`"GNPL"`, version, op count, tag and fields,
  little endian). The ops are TypeScript objects (`{ op: "rect", x, y, width, height, radius, fill, stroke,
strokeWidth }`, `line`, `text`, `icon`, `pushClip`, `popClip`, `pushOpacity`, `popOpacity`), not the Lua tables.
- **Text.** `Text` asks the compositor to measure its font, size and text (`ui.measure` is the `measure` call of the
  canvas class: the size as the bits of an `f64`, the text `font\ntext`). `JsShell` keeps the answers of its own tokens
  (`TreeIncoming::Answer`) for the toolkit; any other answer is still dropped. Until the answer a text is sized by the
  Lua estimate, and a late answer for text that has since changed is ignored.
- **Titlebar.** A `Box` row of a drag area (holding the title), minimise, maximise and close buttons, mounted under the
  window's node (decision 2a). Every number and colour is a field of `TitlebarStyle`. `startDecorations()` attaches a
  `Decorations` listener to `gnoblin.clients`: one titlebar per client, there at attach or arriving later.

    The stock default calls it; an `init.ts` that does not has no titlebars. The tree has no server-side decoration flag a
    window could be asked for (a window's decoration mode is always client-side today, `zxdg_decoration_v1.rs`), so every
    client is decorated. `Client` gains `maximised` and `minimised` (writable) for the buttons.

- **Proven against the fake link** (`crates/gnoblin-js/tests/ui.rs`, 24 tests): row, column, grow, nesting and grid
  rectangles (derived from `Box:arrange`; the Lua corpus has no layout case); the measure call's node, size and text;
  the estimate, the answer, a late answer and a refused measure;

    the Lua test's `GNPL` bytes for a box, a text, an icon and a squeezed text; one list of every op equal to
    `gnoblin_paint::encode` and decoded by `gnoblin_paint::decode`; a bad op named and the content left alone; the
    titlebar's geometry and colours from its style; the buttons, drag and double click through pointer events;

    a window gets exactly one titlebar, it follows the title, and it is gone with the window; one bar per window including
    those at boot; no titlebar without the key; the close button closes the window; `retire` destroys the bars.

- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`,
  `a_script_draws_a_box_and_a_titlebar_and_the_compositor_holds_what_it_painted`): a script's `Box` is a canvas node of
  120 by 40 whose `content` the compositor accepted and the observer reads back, and a mapped window has exactly one
  titlebar canvas under it with its title and the close icon painted in the tree.

    A pixel is read by drawing the decoded list with `gnoblin-paint`'s painter (the one the compositor links), not from
    the compositor's output: a `View` capture was not tried. Real pointer events on a titlebar are not exercised, so the
    drag is proven against the fake link only.

- **Not done.** The keyboard Move and Resize modes of the window menu (`reposition.lua`); a fade for a window its client
  closes itself (the node is gone from the compositor at once); dragging a titlebar with a real pointer.

## Stock shell UI status

Lane JsStockShellUi: `Entry`, `Clock` and `Bar` (`data/js/gnoblin/widgets.ts`), the window menu (`menus/`), the
not-responding alert (`alerts.ts`), `toast` (`panel.ts`), the overview (`overview.ts`), the launcher (`launcher.ts`), and
the titlebar's gestures, fade and reserved room. User page: `docs/js-ui.md`.

- **Host.** The toolkit's events now include a `key` sent to the canvas that holds the seat's focus (`Canvas.focus()`,
  `ui.focus`, written as the seat's `focus`) and a `request` made of any window (`menu`, with its anchor);

    a key event is never printed. `ui.top` (`Canvas.mountTop`) parents the windows the compositor left without a parent to
    the root, as `Client.raise` does (`adopt_windows`, shared), and puts a canvas last, over them.

    The host reads the seat's `pointer_x`, `pointer_y`, `double_click`, `middle_click` and `right_click`, whether the lock
    is engaged or its `state` is not `not-locked`, and the primary screen's size. A canvas may be `secure`.

    `Window` gains `unresponsive`, `kill` (callable now) and the host's `lower` (`Client.lower()`).

- **Menu.** The pure parts are `menuHeight`, `placeCard`, `MenuNavigator` and `menuKeyAction`; the controller is
  `windowMenu`.

    It answers `menu` requests, opens one menu at a time on a layer over the screen, keeps the keyboard where it was and
    closes with its window and on a lock. A script that does not call `startWindowMenu()` never starts it: the compositor
    still sends the requests, and nothing answers them.

    The compositor does not wait for the shell, so there is no timeout to add. Always on top is kept by the menu
    controller (a pinned window is raised again when another window comes over it), not by the compositor.

- **Alert.** `AlertTracker` is the state machine. The alert reads `unresponsive` a few times a second (the property is
  written, not signalled) and its Force quit calls `Client.kill()`.
- **Titlebar room.** Decision: the compositor's decoration mode is client-side and the tree has no server-side flag, so
  decoration cannot be opted into per client; the stock library keeps the Lua behaviour (every window gets a bar) and
  reserves the room instead of drawing over the content. `client.bounds` is the content, `client.frameInset` the bar,
  layouts place frames; see `docs/js-ui.md`, "The room above the client".
- **Launcher.** With no `launcher` command and no key bound to `Super+space`, the host binds the stock entry
  (`planStockLauncher`; `plan.bindings` is unchanged).
- **Proven against the fake link** (`crates/gnoblin-js/tests/stock_ui.rs`, the cases of `menus_test.lua`,
  `alerts_test.lua` and `ui_test.lua`; the fake compositor moved to `tests/common`): menu opening, anchor and clamping,
  keyboard navigation, pointer selection, submenu, Restore, Minimise, Close, Always on top, secure, lock, replacing
  `items` and `show`, a throwing entry, `menus: false`;

    the alert tracker, Wait, Force quit and its arming, the alert closing on recovery, a closed window and a lock,
    `alerts: false`; `EntryModel`, `Entry` keys, `formatTime`, `Clock`, `Bar`, toast, overview, launcher;

    the titlebar's seat actions, the content below its bar, `frameInset`.

- **Proven on the real compositor** (`crates/gnoblin/tests/js_shell.rs`, the last two tests): a mapped decorated
  window's content starts where its bar ends, with the bar on screen above it and no overlap; a right click on the bar
  through the session's input path opens the menu, which the compositor holds as painted canvases, and Escape closes it.

    A client that answers no ping gets the alert, Escape (Wait) closes it, and Force quit (armed) closes it: the
    compositor refuses `kill` for its own process, which is this test's, so the kill itself is proven against the fake
    link only.

## P8a style status

Lane JsStyle. The style model of owner-spec section 6 (`design/js-research/owner-spec.md`) over the existing node
`effects` and `opacity`, with no renderer feature added.

Code: `data/js/gnoblin/style/` (the classes, the cascade, the mapping to effects; no host import), the style section of
`data/js/gnoblin/host.ts` (scopes, `Client.style`, the frame pass), `effects` and `skipped` in
`crates/gnoblin-js/src/bindings/props.rs`.

Tests: `crates/gnoblin-js/tests/style.rs` over fakes, `a_global_style_becomes_the_effect_list_...` in
`crates/gnoblin/tests/js_shell.rs` against the real compositor. User page: `docs/js-style.md`.

- **Classes.** `Style`, `ClientStyle`, `Border`, `Shadow`, `BlurFilter`, `SaturateFilter`, `BrightnessFilter` and
  `ContrastFilter` (the spec's cookbook uses `{ contrast }`) are ordinary ES classes exported from `gnoblin`. A literal
  in a list is normalised to its class; `{ blur: 18 }` in a filter list is a `BlurFilter` of radius 18.
- **Scopes and the cascade.** `Collection.style` is the scope's defaults: `gnoblin.clients.style` (global), a
  workspace's `clients.style`, a `where()` selection's `.style` (`where(query, { priority })`), and `client.style` (the
  client's own overrides).

    A scope holds only what was set on it, so `client.style.cornerRadius` is `undefined` when the client does not override
    it. Precedence, lowest first: built-in defaults, global, workspace, selections by priority and then by the order their
    style was first used (later wins), client. A selection of `workspace.clients` is ranked as a selection, which answers
    spec-review 1.12.

    `computedStyle` is resolved on every read from the live scopes: a frozen object with the value of each property and
    `origin` naming the scope it came from. `Node.style` (spec-review ST2 `(+)`) is not implemented.

- **Unset, `null`, `[]`.** Each property of a scope is a value or unset; `undefined` is unset, so assigning `undefined`
  and `unset(name)` are the same, and `reset()` unsets all. `null` is a value (`background = null` hides what the scopes
  above set), and `[]` is an explicit empty list.
- **Copy on write (decision for spec-review 1.13, owner question 2).** Reading a list a scope does not override returns
  a copy of the inherited list whose elements know they are copies.

    The first write to one of them makes the copy the scope's own list, then applies the write, so nothing leaks to the
    scope the list came from or to other clients. The client detaches from later edits of the inherited list.

    A copy handed out earlier and written later commits as it was read. The cost is that `client.style.shadows[0]` is
    readable without an override (the review's other option made it throw).

- **Observable changes, no Proxy.** A field of an element and a scalar of a style is an accessor: it checks the value,
  throws with the property path (`Client#42.style.borders[2].width: must be finite and >= 0`) and leaves the previous
  value, bumps one module counter (the style epoch) and, on a copy, commits it.

    An array a script changed in place (`push`, `splice`, `list[0] = {...}`) cannot be intercepted: the frame pass
    (`syncStyleLists`) compares each array with its entries at the last check, validates new entries, makes literals
    elements, and puts the old entries back and reports an error for a bad change.

    Assigning a list copies it: `style.shadows = array` does not adopt `array`.

- **Limits.** `MAX_STYLE_LIST` is 8 per list and 8 effects per node (the backdrop blur and the outer shadows). Numbers
  take the renderer's ranges: blur and shadow blur 0 to 100, spread -100 to 100, grades 0 to 2, opacity 0 to 1.
- **Application.** `applyStyles` runs at the end of `layout()` each frame: it checks in-place changes, then for each
  client compares the style epoch and the names of the scopes it sits under (a `where()` it joined or left, a workspace
  move) with the last application, and only a client that moved is resolved and written.

    Writes happen only when the value changed: `effects` as JSON against the last write, `opacity` against the last
    resolved value.

    The first application of a client compares with what the node holds (a window an earlier generation styled), writing
    an empty list when the style has no effects and the node has some, and the opacity when it differs: that is how a
    style line removed on reload takes its effect away.

    `client.opacity` stays the raw node property (animations and the overview write it); a style opacity is written only
    when it changes, so a later animation of the node is not undone by an unrelated style change.

- **What a style becomes.** `opacity` is the node's `opacity`.

    Outer `shadows` are `shadow` effects (`x`, `y` to `offset_x`, `offset_y`; `blur` to `blur_radius`; `spread`; the
    colour as `#rrggbbaa`), listed last to first so the first shadow is on top (paint order is first to last).

    `backdropFilter` is one `blur` effect (documented defaults, `downscale` 2, `passes` 3) when a `BlurFilter` has a
    radius above 0, listed before the shadows; the chain's saturate, brightness and contrast multiply into its
    `saturation`, `brightness` and `contrast`.

- **Not applied, and said so.** The tree has no corner radius, border, background or content filter, and an inset shadow
  has no effect kind; the blur radius is fixed at 12 by the compositor (`BLUR_RADIUS`), so a radius only switches the
  blur on; a grade above 2 is drawn as 2;

    a saturate, brightness or contrast without a blur has nothing to draw. Each stays in `computedStyle`, becomes a line
    of `client.styleWarnings`, and is reported once per generation as a diagnostic `not applied: style.<property>:
<reason>` (`takeNotices`, read by `collect_reports`).

    The compositor's own list of effects it skipped (`skipped` in `node.toml`; the task called it `effects_unsupported`)
    is appended to `client.styleWarnings`. GLES draws blur and shadow; the software and Vulkan renderers draw none.

- **Config.** A script sets the global style with `gnoblin.clients.style` and a selection's with
  `gnoblin.clients.where(...).style`; a client's own style is the highest layer and beats every scope. (The window rules
  engine that once set a matched client's style was removed; design/js-spec-native.md, 3.11.)
- **Proven on the real compositor.** A script that sets `gnoblin.clients.style` opacity and shadows ends as the
  `effects` list and `opacity` of three mapped windows' nodes in the scene (one with a `where()` selection's own
  shadow), and a reload without the lines takes both off.

    **Proven against the fake link only:** the cascade (ST1 to ST5, ST7), copy on write, the mapping and the notices and
    the 8-effect cap. **Not proven:** that a renderer draws the effects (GLES only, not exercised here), `ST6` (secure
    masking).

- **Not done.** `Node.style` and `workspace.style`; content filters, inset shadows, a blur radius, a background and
  per-corner radii (a per-corner radius is drawn as the largest of the four); a titlebar restore animation still ends
  at opacity 1, not the style's opacity.
- **Native corner radius, borders and frame shadow (lane NativeStyle).** The renderer now draws what the style says.

    The node has `corner_radius` (`node.toml`, animated) and the `effects` list gains `border` (`width`, `offset`,
    `inset`, `colour`) and a `frame` flag on `shadow` (`effect.toml`); a node holds at most 8 borders and 8 shadows
    (`too-large`).

    The choice is a node property for the radius, not an effect kind, because the radius clips content, decoration, border
    and shadow together; a per-corner list was not cheap (every renderer's shader takes one radius) and is not drawn.

    For a window, the shape everything shares is its frame: the content's rectangle united with its decoration children (a
    titlebar canvas above it). The composer gives each layer of the frame the same `silhouette` (`gnoblin-render-frame`'s
    `Surface` and `Solid`), so the content and the bar are cut from the frame's corners and not each from their own.

    The shadow with `frame` follows the silhouette, which is what puts it under the bar. Borders are drawn as one ring
    layer after the window's subtree, so the stroke runs over the titlebar. The scene's damage for a window with a frame
    shadow or a border includes the decoration children plus the padding.

    Which renderer draws what: GLES draws `corner_radius`, `shadow`, `border` and `blur` (one SDF fragment program each:
    `border.wgsl`, the shared `gnoblin_corner`); the software renderer draws `corner_radius`, `shadow` and `border` as
    spans (hard edges, a stepped shadow falloff) and skips `blur`;

    Vulkan draws `corner_radius` (it shares the shaders) and reports every effect kind skipped. A node placed in 3D draws
    no shadow, border or blur (`unsupported-effect-on-3d`). `seat.effect_kinds` is `blur, shadow, border` on GLES and
    `shadow, border` on software.

    A capture draws what the output draws; a secure layer keeps its silhouette in the mask and loses its effects, and no
    backdrop effect samples a secure layer. **Proven:** GLES probes on llvmpipe
    (`crates/gnoblin-render-gles/src/shape_tests.rs`), software probes (`shape.rs`), the composer's frame and pixels
    (`decorated_window_tests.rs`), scene damage, the wire and the JS mapping.

    **Also proven:** a real-compositor test (`a_corner_radius_a_border_and_a_shadow_reach_...` in
    `crates/gnoblin/tests/js_shell.rs`) and a live picture on llvmpipe GLES (`rounded` scenario of
    `scripts/js-shell-screenshots.sh`, `docs/images/js-shell/rounded.png`): the corners are cut from the frame, the border
    runs over the titlebar and round the arc, and the shadow surrounds the whole frame, titlebar included.

## Transitions status

Lane JsTransitions. `gnoblin.requestTransition(callback, options)` of owner-spec section 14 over the existing tween
player, with no native change.

Code:

- `data/js/gnoblin/transition/` (the staging, the engine, `Transition`; no host import);
- `crates/gnoblin-js/src/transition.rs` (`__gnoblinHost.check`, the tree's validation of a write without queuing it, the
  same `prepare` that a queued write goes through);
- the staging block at the top of `data/js/gnoblin/host.ts` (replaces `host.get/set/animate/call/raise/lower` and the
  spawn, kill and session verbs of `host.api`);
- `snapshotStyles` in `style/snapshot.ts`.

Tests: `crates/gnoblin-js/tests/transition.rs` over the fake link, `a_layout_change_in_a_transition_...` in
`crates/gnoblin/tests/js_shell.rs` against the real compositor. User page: `docs/js-transitions.md`.

- **Staging.** While the callback runs every node write is checked at once (so the setter that made it throws and names
  its own path) and kept; reads give the staged value.

    Then the layout and style passes run once against the staged state (`layout()` of `gnoblin.ts`, with a layout's error
    failing the transition and not just a report), and the staged writes are sent in the frame's one transaction, each as
    a timed write (`host.animate`) when the property is animatable and the node has a place, and as a plain write
    otherwise.

    The tree starts each track at the value the transform has, which is the drawn one, so a retargeted window does not
    jump.

- **Failure.** The staged writes are dropped, and `snapshotScriptState` puts back each workspace's layout, gaps, name and
  dirty flag, the own fields of the layout it had, and every style (`snapshotStyles`). A layout object the callback
  assigned keeps field changes made to it; an array edited in place is not seen (`fieldsChanged` is shallow).
- **Decisions.**

    Every duration, delay and timeout of the script API is in milliseconds, as the spec says (the one exception is a drag
    event's `deltaTime`, which the spec gives in seconds). That includes:

    - `Client.set`, `gnoblin.show`, `gnoblin.motion` and `Operation.timeout`;
    - the `timeout` of a spawned process;
    - the options of the stock modules.

    Easing names are the CSS names `linear`, `ease`, `ease-in`, `ease-out` and `ease-in-out` for `Client.set` and
    `requestTransition` alike; `crates/gnoblin-js/tests/time_units.rs` fails when an exported time is documented in
    another unit. `gnoblin.after` and `Client.animate` are gone: `gnoblin.timers.after(ms, fn)` is the one timer and
    `Client.set(values, { duration })` the one timed write of a client.

    The nested call, an async callback and a side effect (`call`, `raise`, `lower`, spawn, kill, a session verb) are
    errors. `set(props)` without options is atomic and immediate.

    `finished` is a promise that never rejects and `onFinished` a queued callback, both with `{ cancelled }`. The end is
    the compositor's `track_ended`, not the shell's clock.

- **Acceptance rows** (`spec-review.md`, "Animation"):

- **AN1.** Done: once, one transaction, a returned Promise refused (`tests/transition.rs`).
- **AN2.** Done at the player and the shell: the layout is computed once and every window gets timed writes by node id;
  golden poses at 0, 0.25, 0.5 and 1 for position, opacity, scale and rotation, and a size tween in whole pixels.
- **AN3.** Not done: there is no camera (native and spec work).
- **AN4.** Done: no JS runs between the commit and the end (the callback count stays 1 across frames); a scrub is one
  write and `JsShell::dispatches` stays put over three scrubs (fake link); against a real compositor a held transition
  is scrubbed 0, 0.4, 0.1, 1 and let go, and it ends on the compositor's `track_ended` (`js_shell.rs`).
- **AN5.** Done at the scene: while a track is seeked, `Scene::hit_test` uses the presented rectangle, position, scale
  and rotation, and a point inside only the target does not hit (`tween_scrub.rs`).
- **AN6.** Done over the fake link: `cancel` from running and from ended, `{ cancelled }` once; `progress`, `play()`,
  `pause()` and `reverse()` write the `tween`, a transition run back to its start settles cancelled and puts the start
  values back.
- **AN7.** Done: an invalid value, a stale (destroyed) client and a throwing layout abort with the previous state and
  the path in the error.
- **AN8.** Done: the newest owns a property and the older goes on with the rest, one with none left ends cancelled, a
  nested call is an error.

- **Native tweens** (lane TweenNative). `TweenPlayer` seeks to a point of a track, pauses, runs backwards and settles
  (`finish`: forward, back or cancel);

    a track interpolates `width` and `height` (whole pixels), and `rotation` and `scale` per axis as the node stores them;
    the timed write has `delay`;

    a `tween` node (`tween.toml`, `design/node-core.md`, "Tweens") holds the play state of the tracks that join it; the
    compositor sends `track_ended`, which `finished` settles on.

    `crates/gnoblin-render/tests/tween_scrub.rs` has the golden poses and the hit tests,
    `crates/gnoblin-js/tests/transition.rs` the scrub, reverse, delay and end over the fake link.

- **Not built**:

    - presented positions read from a running compositor (a tween's pose is not in the tree an observer reads, so the
      scrub and the end are proven another way):
        - `crates/gnoblin/tests/js_shell.rs` proves the scrub through the `tween` node and the end through `track_ended`;
        - `crates/gnoblin-wayland/tests/tree_live.rs` proves the poses on the composer with an injected clock;
    - tweening effects and corner radius;
    - `Camera2D`/`Camera3D`;
    - a tree surface for `TweenPlayer::finish`;
    - a quaternion rotation path.

    Spec examples 076 to 079 (`arcLayout.radius`, `camera3D`, `display.viewport`, a `View`) need objects the runtime does
    not have.

# JS runtime: live verification

The pictures of the shipped shell, the defects they showed and how each was fixed. Part of the [JS/TS runtime
design](runtime.md).

## Live verification

Lane JsLiveVerify. Until now the shell was proven by scene-tree assertions; nobody had looked at it. This section is what was
seen on pictures of the shipped binary, how to take them again, and what is wrong with them.

### How the pictures are taken

`scripts/js-shell-screenshots.sh [OUT_DIR [SCENARIO...]]` takes the pictures in three steps:

- It boots the release `gnoblin` as a session: the guardian, which starts the compositor on its headless backend with
  the software renderer, the services role and the runtime role that runs the JavaScript shell.
- It starts real Wayland clients (`foot`, running `gnoblinctl` commands so the windows show real output).
- It takes each picture with `grim` on the compositor's privileged socket (`gnoblin-wayland-privileged.sock`, the same
  path as `docs/troubleshooting.md`), which is the compositor's own screencopy.

No `init.ts` is present unless a scenario writes one, so the zero-config scenarios run the stock library.

Every picture is saved with three files, so a picture that looks wrong has its numbers beside it:

- `NAME.windows.json`: the compositor's window list.
- `NAME.clients.json`: the shell's `x`, `y`, `width`, `height`, `frameInset`, `tiled` and workspace for each client,
  from `gnoblinctl console --eval`.
- `NAME.log`: the session log.

It needs `grim`, `foot` and `dbus-daemon` (and `wtype` for the menu); `shellcheck` is the lint.

On an Agent37 instance (Debian 12, gVisor, no GPU):

```sh
apt-get install -y --no-install-recommends grim foot wtype dbus dbus-daemon fonts-dejavu-core libegl1 libgles2 libegl-mesa0 libgl1-mesa-dri
a37-gate.sh run ID TREE build --release -p gnoblin -p gnoblinctl
a37-gate.sh in ID TREE scripts/js-shell-screenshots.sh /home/node/shots
a37 pull ID /home/node/shots/master-stack-decorated.png master-stack-decorated.png
```

Any Linux box with the build works the same way: `GNOBLIN_BIN_DIR=target/release scripts/js-shell-screenshots.sh ./shots`.
Each scenario boots its own session over a fresh directory, so one can be run alone by name.

| Scenario                                      | What it shows                                                                                             |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `one-window`                                  | zero config, one window, its titlebar                                                                     |
| `cascade`                                     | zero config, three windows of the default size                                                            |
| `master-stack`, `dwindle`                     | three windows, then a reload to an `init.ts` that declares workspaces                                     |
| `master-stack-decorated`, `dwindle-decorated` | the same with `startDecorations()`                                                                        |
| `shadows-software`, `shadows-gles`            | `gnoblin.clients.style.shadows` on each renderer (the renderer is read at start, so the session restarts) |
| `csd-double`, `csd-own`, `csd-tiled`          | foot as it comes: the shell's one bar; with decorations off, foot's own bar (off screen)                  |
| `reload-tiled`, `transition-mid`              | a reload of three tiled windows (before and after compared); a frame mid-transition                       |
| `window-menu`                                 | the window menu, opened with `Alt+Space` sent by `wtype`                                                  |
| `lock`                                        | `gnoblinctl lock`, then a capture, which is refused                                                       |
| `rounded`                                     | decorated windows with `cornerRadius`, a border and a shadow on GLES                                      |
| `window-frame-buttons`, `-glow`, `-strip`     | the three examples of `docs/recipes/window-frame.md`, read from that page, on two windows                 |

The pictures (`/home/kieran/dev/lanes/JsLiveVerify-shots/` on the workstation of the lane) are not in the repository except
four that are clean product states and are in `docs/typescript-shell.md`, "What it looks like": `docs/images/js-shell/`.

### What was seen

- **Zero config, one window (`one-window`).** A 700 by 500 window under a 28 px titlebar (`#414559` while focused, title in
  `#c6d0f5`, minimise, maximise and close at the right), content directly below it, nothing over it, and black everywhere
  else: the stock shell draws no wallpaper and no bar. The window sits at the screen's top-left corner. Text is crisp and
  not clipped.
- **Master-stack and dwindle with decorations.** The bars lie above each tile, the focused bar is lighter than the others,
  the 12 px gap is exactly 12 px black, windows touch the screen's edges, and no tile overlaps another.
- **Window menu.** A card with rounded corners, two groups of entries and a submenu mark, text not clipped, opened at the
  top-left corner of the window's content because `Alt+Space` has no pointer to anchor to. The titlebar's right click is
  not drawn: a shipped session has no way to inject a pointer (its EIS socket stays shut until a remote desktop session is
  approved, and `gnoblin-wayland` offers no virtual pointer), so it is proved by the scene in
  `crates/gnoblin/tests/js_shell.rs`, `a_decorated_windows_content_lies_below_its_bar_and_a_titlebar_right_click_opens_the_window_menu`.
- **Shadows.** The software renderer draws none (as `docs/js-style.md` says). GLES draws them: the session was started
  with `compositor.renderer = "gles"` on Mesa's llvmpipe (surfaceless EGL works under gVisor), and the shadow is visible on
  the window below it.
- **The lock screen and the lock card (`lock`).** Not drawn. A locked session refuses every capture (`grim` says
  `failed to copy output HEADLESS-1`), by design: no pixel leaves a locked compositor. `docs/session-lock.md` says no test
  reads their pixels and that is still so; an in-process capture that skips the embargo would be a way around the rule
  being proved, so it was not added.

### Defects

- **1.** `master-stack.png`

    Diagnosis: A black column at x = 1023: `tile()` floored each width on its own, so a master of 607.2 and a stack of
    404.8 at 619.2 ended at 619 + 404 = 1023. Every tiled layout had it (grid with 3 columns on 1000 px ended at 999; the
    Lua shell had the same).

    State: Fixed in `data/js/gnoblin-std/layout.ts` (`tile` floors edges). Tests: `stock_layouts.rs`, real:
    `a_master_stack_workspace_with_a_ratio_and_a_gap_fills_the_screen_to_both_edges`. The Lua goldens that pinned the
    sliver are changed.

- **2.** `cascade.png` (before the fix)

    Diagnosis: Three windows of the default size: the second and the third landed at exactly (324, 268). `placeRect`
    pushes a cascade that found no free spot back inside the area, pinning every later window to the far corner, so the
    third hid the second.

    State: Fixed in `FloatingLayout.onClientAdded` (`apartFrom` steps back by the cascade step). `placeRect` and its
    golden table are unchanged. Tests: `stock_layouts.rs`, real:
    `windows_too_big_for_a_free_spot_cascade_and_never_land_exactly_on_each_other`.

- **3.** `master-stack.png`, `dwindle.png`

    Diagnosis: An `init.ts` that declares workspaces and does not call `startDecorations()` draws no titlebars. With foot
    as the client, foot then draws its own title bar, which is outside its window and off screen for the top row.

    State: Open, by design, owner's call: should an `init.ts` that leaves the call out get titlebars? An `init.ts`
    replaces the default, and the starter calls `startDecorations()`. The doc and the screenshots in `docs/` say what is.

- **4.** `csd-own.png`

    Diagnosis: Fixed: the window is its geometry, so foot's bar is on screen and the window is 700 by 500 (below, "The 52
    px, fixed").

    State: Fixed in the compositor: the window node is the window geometry. Test:
    `a_client_side_decorated_window_is_held_at_its_window_geometry_not_its_buffer_less_its_bar`, no longer ignored.

- **5.** `shadows-gles.png`

    Diagnosis: The shadow is the window node's, so it is cast by the content and not by the frame: it has nothing at the
    titlebar's side. The shadow effect has no rectangle of its own to grow.

    State: Open. Needs the effect to take an extent, in the compositor.

- **6.** `master-stack.log`

    Diagnosis: A reload logged 19 `a seat claim's chord is already bound for that device` and one `a seat gesture claim
was refused` at WARN. The new generation claimed each stock key while the old one still held it, so the compositor
    refused the duplicate, and the old generation's release then freed the chord: after the reload nobody held it and the
    key did nothing.

    State: Fixed . A staged generation's claims wait for the swap and go out behind the old
    generation's releases, in one transaction (["Hot reload"](runtime-reload.md#hot-reload), ["Seat claims across a
    reload"](runtime-reload.md#seat-claims-across-a-reload)). Tests: `crates/gnoblin-js/tests/input.rs`, real:
    `a_reload_hands_every_seat_claim_over_so_each_shortcut_fires_once_and_a_removed_one_is_released`.

- **7.** every `.log`

    Diagnosis: The guardian wrapped each INFO line a child prints on stderr as `WARN guardian/raw INFO ...`, so a healthy
    start was a page of warnings: `is_structured` looked for the bracketed form `INFO  [role] ...`, and the console prints
    `INFO   role/component  message`.

    State: Fixed in `crates/gnoblin-session/src/raw.rs`: a line the child formatted itself is passed
    on with its own level. Tests in the same file.

**Defect 4 (double title bars) is fixed** (`docs/js-ui.md`, "Who draws the frame").

The compositor now negotiates `xdg-decoration`: a client that asks for server-side is told so only while a shell that
holds the tree has written the main seat's `decorations_available` (the stock shell does when `decorations` is on, after
each generation starts, and takes it back when a generation retires; the compositor clears it when the shell's
connection drops), and is told client-side otherwise.

The window node publishes `decoration` and `decoration_wanted`; the stock `Decorations` mounts a `Titlebar`, and
`frameInset` reserves room, only on a `server` window. The answer is sent again to existing windows when the declaration
changes.

Proven by scene and protocol tests: `crates/gnoblin-wayland/tests/decoration.rs` (a real `xdg-decoration` client: no
shell, shell without declaration, declared, client-side ask, no preference, no manager, decorations off and on, shell
connection drop), `crates/gnoblin/tests/js_shell.rs` (a client that draws its own bar gets none, a reload toggling
decorations) and `crates/gnoblin-js/tests/stock_ui.rs`. It is re-taken on pixels below.

**The late bar is fixed** (`docs/js-ui.md`, "The room above the client"). The bar can arrive after the
window was placed: the window mapped before the shell declared decorations, a reload that turned them on, or a client
that changes its `decoration` later.

The floating layout places a window once, so the bar mounted above a window at y = 0 and was drawn at y < 0. The rule is
that the FRAME keeps its place.

The layout driver notes each window's bar height as it fits it (`fittedBar`), and when the bar differs the next pass
(`followBar`) moves the content by the difference, then clamps the frame into the work area.

A maximised window fills the area again, a snapped one tiles to its zone from the area less the bar, a tiled one is
placed again by its layout (the decoration change marks the workspace), and a fullscreen one is left and fitted when it
leaves.

The rectangles a window returns to (`windowState.kept`, the snap record, `floatingBounds`) move with it. `refitClient`
(an output change) fits frames too. The bar height crosses a reload in the workspace snapshot (`bar` in `KeptWindow`),
so a reload that turns decorations off lifts the content by the bar that went.

Tests: `stock_ui.rs` ("the frame keeps its place when the bar comes or goes", seven cases) and `js_shell.rs`
(`a_window_mapped_before_the_shell_declared_decorations_ends_with_its_bar_on_screen`; the decoration reload test checks
the bar is on screen in both directions; the three tests that waited for the declaration before mapping no longer wait).

**Parked windows keep their nodes** (same lane; `docs/shell-api/outputs.md`, "Unplugging").

With no output left the compositor used to hide the windows itself, so each node left the tree when the hide's fade
ended, and the next output returned them as windows the shell had never seen: a window the shell had moved to the first
workspace went back to its rule's workspace, hidden.

While a shell holds the tree the compositor now keeps the nodes (`remove_outputs_unless`); the shell hides the windows
of the parked desk itself, so the rule from the Lua shell holds: the windows come back where they were, on the workspace
the next output shows.

The real test `a_window_on_a_second_virtual_output_follows_to_the_first...` is back to both windows filling the new
output, and waits longer than the fade to prove the nodes stay.

### Re-verified on pixels (second run)

The release binary of the tree at `origin/rust-compositor` after SsdNegotiation, the reload claim hand-off, the console
global, tiling bindings, transitions, outputs and the idle fixes, on an Agent37 instance, every scenario of
`scripts/js-shell-screenshots.sh` (pictures in `/home/kieran/dev/lanes/JsLiveVerify-shots/`, four in `docs/images/js-shell/`):

- `one-window`, `csd-double`, `csd-tiled`: foot as it comes asks for server-side and gets exactly one bar, the shell's
  (28 px, above the content). `csd-tiled` (decorations and a dwindle workspace) fills the screen under one bar.
- `master-stack-decorated`, `dwindle-decorated`: one bar above each tile, 12 px gaps, edge to edge (the 1 px column of
  defect 1 stays fixed). `cascade`: the third window no longer hides the second (defect 2 stays fixed).
- `csd-own` (foot, `decorations: false`): **no bar is visible.** Foot draws its own, but its bar is a subsurface above its
  main surface and the window sits at y = 0, so the bar is off screen, and the window is 448 high, not 500. That is the
  height loss of defect 4's first run, which was still there on this run and is fixed since: see "The 52 px, fixed".
- `reload-tiled`: three tiled windows before and after `gnoblinctl config reload` are byte-identical PNGs.
- `transition-mid`: a `requestTransition` moving a window 600 px right over 6 s, captured 2 s in, shows the window at
  x = 212 (linear: 200), and `transition-end` shows it at 600. The script catches the middle frame by a long duration on
  the real clock; the injected clock is not reachable from a shipped session.
- `window-menu`: as before, opened with `Alt+Space`. Not run: the `Super`+drag swap and the titlebar right click, because a
  shipped session has no pointer injection (the EIS socket is shut until a remote desktop session is approved and no
  virtual pointer is offered); both are proved in `crates/gnoblin/tests/js_shell.rs`.
- `lock`: still refused (`failed to copy output HEADLESS-1`). The lock card cannot be read back on pixels from this tree:
  `ControlHandle::screenshot` reads the frame while locked (`crates/gnoblin-wayland/tests/session_lock.rs` does), but the
  card is drawn by the compositor crate's `LockCardService`, whose test `Session` runs `Compositor::run` and does not hand
  out the control handle. A test would need that handle exposed; the card's colours are therefore unproven.

### The 52 px, fixed

A client that draws its bar in a subsurface above its main surface (foot: `set_window_geometry(0, -26, 700, 500)`, a 700
by 474 buffer) has two rectangles, and the tree knew one. `publish_geometry`
(`crates/gnoblin-backend/src/render/compose.rs`) published the window node's `width` and `height` as the **buffer**
size, 700 by 474, and the node's position as the main surface's origin.

The shell writes a window's `width` and `height` back, and the compositor sends them as the `xdg_toplevel.configure`
size, which is a **window geometry** size: 474 asked of foot gave a 448 buffer, with its 26 px bar inside the 474.

Each round trip lost the bar's height, and a window placed at y = 0 had its bar at y = -26.

The fix is in the compositor, at the root: **the window node is the window geometry.**

- `x`, `y`, `width` and `height` of a window node are the `xdg_surface.set_window_geometry` rectangle in global
  coordinates, and the whole surface while the client set none (xdg-shell's default). `publish_geometry` publishes that,
  `window_node_size` (the size a configure asks for) is that size, and `window_frame` is that rectangle. A configure then
  asks for what the tree publishes, so the round trip is stable.
- The surface is drawn at the node's origin less the geometry's offset (`Drawn::local` in `push_surface`, 3D models
  included), a surface hung from it (a subsurface or a popup of the window's own surface) is placed in the node's plane
  (`tree_origin`), and what lies outside the node is the node's **draw extent**: it is drawn and damaged with the node
  (`Scene::set_draw_extent`; a commit damages the shadow) and is not the window.

    The node's hit extent is the surface tree's bounds in the same plane, so the pointer still reaches what the client's
    input region admits outside the geometry (a GTK resize border), and `surface_under` turns the node's point back into
    the surface's coordinates before the input region and the subsurfaces are asked.

    A popup is found from the surface's own origin, the frame less the geometry's offset (`window_popup_under`).

- Fallback placement, cascade, centring, tiling, maximise and fullscreen already worked in geometry (the policy's record is
  the geometry, adopted from `set_window_geometry`), so a maximised window fills the output with its geometry and the
  client's shadow lies past the screen's edge.
- A capture of a window (screencopy, toplevel capture, a cast) copies the output's pixels over the window geometry, so
  the shadow outside it is not in it; a View of a window draws its own surface cut to the geometry
  (`view_crop`, `geometry_crop`), and its subsurfaces and popups whole. `docs/shell-api/windows.md`, "Window geometry and
  capture", states the rule.

Proven by (each case fails on the tree before the fix, except the pixel cases of a shell-less session, which were already
right and are kept as guards):

- `crates/gnoblin-backend/src/render/surface_tree_tests.rs`: the published rectangle for geometry origins (0, 0),
  (0, -26) and (12, 12) with larger buffers, an unset geometry, a changed geometry republished, the surface drawn at the
  frame less the offset, damage of the shadow, a bar above the surface, a popup, the pick and its hit extent, maximise
  filling the output, and a View cut to the geometry.
- `crates/gnoblin-wayland/tests/window_geometry.rs` (real client, a shell that holds the tree): the node a shell reads for
  each origin, an unset geometry, a geometry committed later, five configure round trips that lose no bar, and the
  pointer in surface coordinates with and without an input region over the shadow; `tests/surface_tree.rs`: the pixels of
  a shadow margin, a bar above the surface and a popup anchored in the geometry, on both renderers.
- `crates/gnoblin/tests/js_shell.rs`, with the shipped shell: `a_client_side_decorated_window_is_held_at_its_window_geometry_not_its_buffer_less_its_bar`
  (no longer ignored), a bar, a shadow margin and no geometry, five round trips, and fullscreen filling the screen.

Live, `scripts/js-shell-screenshots.sh csd-own` on the fixed tree (release build on an Agent37 instance): foot's own bar
is on the screen at the top-left corner, and `csd-own.windows.json` and `csd-own.clients.json` say the window is 700 by
500 at (0, 0) (before: 700 by 448, its bar at y = -26).

`csd-double` is unchanged: the shell's 28 px bar, the window at (0, 28), 700 by 500. The picture is
`docs/images/js-shell/csd-own.png`.

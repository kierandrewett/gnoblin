# wlcs: Wayland protocol conformance for the Rust compositor

[wlcs](https://github.com/canonical/wlcs) is Canonical's Wayland Conformance Suite. It is the Phase 2 gate "wlcs core subset passes" (`design/rust-compositor.md`). wlcs does not start a compositor: it loads a shared library that exports `wlcs_server_integration`, and the library starts the compositor under test, hands out client connections and injects input. `crates/gnoblin-wlcs` is that library.

wlcs is GPL-2.0. It is cloned and built into `target/wlcs` by the script and is never vendored, linked or shipped.

## Run it

```sh
scripts/wlcs.sh            # build wlcs and the library, run the core subset, check it against tests/wlcs/expected.toml
scripts/wlcs.sh --gate     # only the tests expected.toml lists as passing (what CI runs)
scripts/wlcs.sh --all      # every wlcs test
scripts/wlcs.sh --update   # rewrite the `passing` list from this run
scripts/wlcs.sh 'XdgToplevelStableTest.*'   # run matching tests and print each one's output
```

It prints a pass/fail/skip/crash table per suite and exits 1 on a regression, a fixed known failure, or a failure with no entry. `WAYLAND_DEBUG=1` shows both sides of the wire.

- Build recipe on Debian or Ubuntu: `apt-get install git cmake g++ pkg-config libwayland-dev libgtest-dev libgmock-dev libboost-dev python3`.
- wlcs is pinned at `v1.8.1` (`WLCS_REF`).
- The script edits wlcs's own build files: it builds with `-Werror`, and CMake before 3.26 mangles `-DNAME=value` in `COMPILE_DEFINITIONS`.
- It applies the patches in `tests/wlcs/patches/`, which fix wlcs attaching a buffer before the initial configure is acknowledged. A compositor that enforces `xdg_surface.unconfigured_buffer` (Smithay does) closed the connection in about 320 tests before they tested anything.

## How the library runs the compositor

Each wlcs test gets a `ConformanceSession` (`crates/gnoblin-wlcs/src/harness.rs`): the real `gnoblin_wayland::Server` on a headless backend, with its one 1024x768 output and the software renderer, on a thread of its own.

- Clients are socket pairs adopted with `ControlHandle::adopt_client`.
- `position_window_absolute` finds the window by the client's descriptor and the surface's protocol id (`ControlHandle::place_surface`) and puts it at an exact position, with no work-area clamp.
- Input goes through `ControlHandle::inject_input`, followed by `ControlHandle::sync`, so a client's next round trip is answered after the event reached it.
- New windows take focus and motion is off, as the suite assumes.
- The descriptor lists exactly the globals a client sees; a test in the crate fails when the two differ.
- All `unsafe` is in `ffi.rs`.

Limits of the injection:

- Pointer and touch positions are whole pixels; wlcs passes `wl_fixed_t` and fractions are rounded.
- `touch.down_at` arrives as plain integers, because wlcs does not convert them.

## Bugs wlcs found and this change fixed

- A focused toplevel was never sent `xdg_toplevel.state.activated`. Every wlcs window waits for it.
- `wl_pointer.frame` was only sent after scroll. Enter, leave, motion and buttons were not closed by a frame, which clients at version 5 or later (GTK, Qt) rely on.
- `wl_surface.frame` callbacks of subsurfaces and popups were never answered.
- A popup's `wl_surface.frame` callback was never answered, so every popup test timed out waiting for the first frame of its menu. The callback is answered by the frame that follows the popup's commit, which draws it (`gnoblin-1whg`: a popup is a node under its window's, so its commit damages exactly where it is drawn).
- A popup took no pointer input: `State::surface_under` asked only the window the scene picked, and the scene picks by the window's bounds, so a popup hanging past its window's edge was unreachable. Popups are asked first, from the top of the window stack down to the picked window (`crates/gnoblin-wayland/src/popup.rs`), at the position the compositor last configured them at, which is also what the suite expects after `xdg_popup.reposition` and before the client's acknowledgement.
- `xdg_popup.grab` was logged and ignored. A grabbed popup now takes the keyboard, a press on another client dismisses the grab (child popups first) and a window that takes the keyboard ends it.
- A popup that was destroyed under the pointer left the client waiting for `wl_pointer.leave` (the suite's client fails on an enter without one): the pointer is moved on to what lies under it as the popup goes.
- A reactive popup (`xdg_positioner.set_reactive`) was not placed again when its window moved.
- Pointer focus went to the toplevel only: subsurfaces never took input, and `wl_surface.set_input_region` was ignored. `State::surface_under` now hit-tests the surface tree.

Each has a test in `crates/gnoblin-wayland` or `crates/gnoblin-wlcs`.

## Result

Core subset: 702 tests, 467 pass, 15 fail and 220 skipped (the `wl_shell` and `zxdg_shell_v6` variants, which the compositor does not serve). Every failure has a known-failure entry in `tests/wlcs/expected.toml` with a reason and the Beads issue that owns it:

- `gnoblin-wrns`: Smithay's zero-size positioner anchor rectangle.
- `gnoblin-ve4a`: protocol errors the suite expects: `xdg_wm_base.get_xdg_surface` is not validated (Smithay raises `role` and `invalid_surface_state` only at `get_toplevel` and at commit), the `wp_viewport` errors and the truncated shm file.
- `gnoblin-v3hw`: subsurface timing. Smithay applies `wl_subsurface.set_position` at the subsurface's own commit and `place_above` and `place_below` at once; the protocol applies both with the parent's state.
- `gnoblin-of0d`: later-phase protocols.

`gnoblin-f3cp`, `gnoblin-y3yk` and `gnoblin-jpbf` are resolved:

- A floating toplevel's initial configure suggests no size (0 by 0); the record takes the client's window-geometry size at the commit that maps it, so a later maximise returns to the size the client chose.
- `xdg_surface.set_window_geometry` places the window: the surface is drawn at the window's rectangle less the geometry's offset, so a point inside the geometry is the same point whatever the shadow width, and `xdg_toplevel.show_window_menu` anchors the same way.
- A window takes hits over the whole surface tree, so a subsurface that reaches past its parent receives pointer and touch input there (`Scene::set_hit_extent`).
- `xdg_toplevel.move` and `resize` start an interactive gesture when no shell holds the tree (`crates/gnoblin-wayland/src/interactive.rs`): a Smithay pointer or touch grab takes the device from the client, moves or resizes the window by how far the device went, and ends when the last button or the touch point is released. With a shell they stay node requests.
- The pointer follows the surface under it: a window or subsurface that moves, resizes, maps or closes under a pointer that has been placed gets `enter`, `leave` or `motion`.
- A touch point on a surface that is destroyed ends with `wl_touch.up`, closed by a frame, and not with `wl_touch.cancel`. wayland.xml defines `up` as "the touch point has disappeared" and `cancel` as "sent if the compositor decides the touch stream is a global gesture", which ends every point of the client; a window going away under a finger is the first. The wlcs test `sends_touch_up_on_surface_destroy` also asks for an end of the point (`touched_window()` is null afterwards), which only an `up` and its frame give: the suite's client has no `cancel` listener (libwayland aborts a process that gets an event with none), which is why the old cancel crashed the xdg variants. That is a gap in the suite, not a reason for the compositor's choice, and no patch is needed. `a_touch_point_on_a_destroyed_window_ends_with_an_up` in `crates/gnoblin-wayland/tests/protocols.rs` pins it; a device-level cancel still sends `cancel` (`a_cancelled_touch_ends_its_point_without_an_up`).

`tests/wlcs/patches/0003-xdg-toplevel-tests-ack-the-initial-configure.patch` changes wlcs tests: the interactive ones attached a buffer before acknowledging the initial configure, and `wm_capabilities_are_sent` waited for an event that xdg-shell sends with the first configure, which follows the initial commit, without committing.

The popup tests (`XdgPopupPositionerTest`, `XdgPopupStable`, `XdgPopupTest`) pass but for one; 88 more are skipped because the suite's popup of a layer surface and of a `zxdg_shell_v6` surface need globals the compositor does not serve.

The core subset therefore does not pass yet: the gate in `design/rust-compositor.md` stays open until the issues above are closed or their entries become ledger decisions. What is in place is the runner, the regression gate over the 467 passing tests, and the list of what is left.

## CI

`.github/workflows/wlcs.yml` runs `scripts/wlcs.sh --gate` on pushes and pull requests that touch the compositor crates, the library, the script or the expectations, and on demand with `scope` set to `core` or `all`.

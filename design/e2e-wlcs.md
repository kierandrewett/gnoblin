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
- A popup's `wl_surface.frame` callback was never answered, so every popup test timed out waiting for the first frame of its menu. A popup is not drawn (the scene holds one buffer per window), so its commit now repaints the outputs and the callback is answered by the frame that follows.
- A popup took no pointer input: `State::surface_under` asked only the window the scene picked, and the scene picks by the window's bounds, so a popup hanging past its window's edge was unreachable. Popups are asked first, from the top of the window stack down to the picked window (`crates/gnoblin-wayland/src/popup.rs`), at the position the compositor last configured them at, which is also what the suite expects after `xdg_popup.reposition` and before the client's acknowledgement.
- `xdg_popup.grab` was logged and ignored. A grabbed popup now takes the keyboard, a press on another client dismisses the grab (child popups first) and a window that takes the keyboard ends it.
- A popup that was destroyed under the pointer left the client waiting for `wl_pointer.leave` (the suite's client fails on an enter without one): the pointer is moved on to what lies under it as the popup goes.
- A reactive popup (`xdg_positioner.set_reactive`) was not placed again when its window moved.
- Pointer focus went to the toplevel only: subsurfaces never took input, and `wl_surface.set_input_region` was ignored. `State::surface_under` now hit-tests the surface tree.

Each has a test in `crates/gnoblin-wayland` or `crates/gnoblin-wlcs`.

## Result

Core subset: 702 tests, 310 pass, 170 fail, 220 skipped (the `wl_shell` and `zxdg_shell_v6` variants, which the compositor does not serve) and 2 crash (wlcs itself aborts on `wl_touch.frame`). Every failure has a known-failure entry in `tests/wlcs/expected.toml` with a reason and the Beads issue that owns it:

- `gnoblin-f3cp`: xdg window geometry.
- `gnoblin-6hac`: popups.
- `gnoblin-y3yk`: touch.
- `gnoblin-ve4a`: protocol errors the suite expects.
- `gnoblin-jpbf`: toplevel configuration.
- `gnoblin-v3hw`: subsurfaces and surface events.
- `gnoblin-of0d`: later-phase protocols.

The core subset therefore does not pass yet: the gate in `design/rust-compositor.md` stays open until those issues are closed or their entries become ledger decisions. What is in place is the runner, the regression gate over the 310 passing tests, and the list of what is left.

## CI

`.github/workflows/wlcs.yml` runs `scripts/wlcs.sh --gate` on pushes and pull requests that touch the compositor crates, the library, the script or the expectations, and on demand with `scope` set to `core` or `all`.

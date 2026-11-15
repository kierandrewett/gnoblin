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
- It applies the patches in `tests/wlcs/patches/`. Most fix wlcs attaching a buffer before the initial configure is acknowledged: a compositor that enforces `xdg_surface.unconfigured_buffer` (Smithay does) closed the connection in about 320 tests before they tested anything. `0005` fixes the second assertion of `place_above_simple` and `place_below_simple`, which demanded that a subsurface that is on top is not the pointer focus (the same failure is reported on Mir, canonical/wlcs#396). `0012` puts only the text input's own calls in the `InSequence` of two text-input tests (see "Later-phase protocols").

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

Default subset (the Phase 2 core and the later-phase protocol suites, see "Later-phase protocols" below): 746 tests, 520 pass, 6 fail and 220 skipped (the `wl_shell` and `zxdg_shell_v6` variants, which the compositor does not serve). Every failure has a known-failure entry in `tests/wlcs/expected.toml` with a reason and the Beads issue that owns it:

- `gnoblin-wrns`: Smithay's zero-size positioner anchor rectangle.
- `gnoblin-of0d`: later-phase protocols. Five tests remain and none is a missing protocol: all five are the selection rule below (see "Later-phase protocols").

`gnoblin-ve4a` and `gnoblin-v3hw` are resolved, with no change to Smithay. Seven of their tests passed earlier:

- `xdg_wm_base.get_xdg_surface` is checked before Smithay handles it (`request_guard.rs`, applied by `dispatch.rs` to every request): a `wl_surface` with a role other than an xdg one is `role`, and one with a buffer attached or committed is `invalid_surface_state`. Three `XdgSurfaceStableTest` tests.
- `wp_viewporter` is the compositor's own (`protocols/wp_viewporter.rs`). Smithay's raises `viewport_exists` on the `wl_surface`, installs its `bad_size` hook only for a surface that has no viewport data (a surface that has already committed a buffer has it, so the hook was never installed for the surfaces that matter) and checks `out_of_buffer` only when a commit brings a new buffer. The viewport state is still Smithay's `ViewportCachedState`, so the renderer is unchanged. Three `WpViewporterTest` tests.
- A `wl_shm` pool the client shrank under a buffer is `wl_shm.invalid_fd` on the buffer at the commit that attaches it (`wl_compositor.rs`, `gnoblin_render_frame::shm::probe`, one byte read inside Smithay's SIGBUS guard). The suite test did nothing in any build of ours: it wrote its `ftruncate()` inside an `assert()`, and `scripts/wlcs.sh` builds Release, which compiles the assert away. `tests/wlcs/patches/0004-bad-buffer-test-truncates-outside-the-assert.patch` runs the call. `BadBufferTest.test_truncated_shm_file`.

Seven more pass now:

- A buffer attached to an `xdg_surface` before its first configure is acknowledged is `xdg_surface.unconfigured_buffer` at the attach (`request_guard.rs`). Smithay raises it at the commit, so a client that attaches and does not commit was never told. The error has to be posted on the `xdg_surface`, which Smithay creates inside its `get_xdg_surface` handler and keeps in private state: the guard reads the new object's protocol id from the request (the only thing `New<XdgSurface>` gives out is its `Debug` form), lets Smithay's handler make the object, and finds it by that id afterwards (`Screened::ForwardThenTrack`). It keeps a weak reference on the `wl_surface` and asks the role's `last_acked` at each attach with a buffer. If the id cannot be read, the check falls back to Smithay's commit-time one and logs a warning. `XdgSurfaceStableTest.attaching_buffer_to_unconfigured_xdg_surface_is_an_error`; `crates/gnoblin-wayland/tests/xdg_surface_state.rs`.
- `wl_subsurface.set_position` is the parent's state and lands when the commit that applies the parent is applied (`crates/gnoblin-wayland/src/subsurface_position.rs`). Smithay keeps it in the subsurface's own cached state, so a desynchronized subsurface moved with its own commit, and a subsurface of a synchronized subsurface moved when only the grandparent committed, because Smithay's commit of a tree snapshots the pending state of every synchronized surface below it, committed by its client or not. The request is answered by `request_guard.rs` and never reaches Smithay; the parent's pre-commit hook moves it into a cached state of the parent, and the commit handler writes it into the subsurface when the parent's state is applied. Four `SubsurfaceTest` and `SubsurfaceMultilevelTest` tests; `crates/gnoblin-wayland/tests/surface_tree.rs` (`a_desynchronized_subsurface_moves_when_its_parent_commits_and_not_when_it_does`, `a_synchronized_subsurface_moves_with_the_parent_commit_alone`, `a_grandchild_moves_only_when_the_commits_above_it_apply_in_order`, `a_subsurface_that_is_made_again_starts_at_its_parents_origin`).
- `place_above_simple` and `place_below_simple` pass with `tests/wlcs/patches/0005-subsurface-restack-tests-expect-the-top-subsurface-focused.patch`. The restack was never the failure: the first assertion of each test, which checks it, passed. `place_above` and `place_below` still reorder at once, where the protocol applies the order with the parent's commit; Smithay has no public call to reorder later and no test of the suite looks at it.

`gnoblin-f3cp`, `gnoblin-y3yk` and `gnoblin-jpbf` are resolved:

- A floating toplevel's initial configure suggests no size (0 by 0); the record takes the client's window-geometry size at the commit that maps it, so a later maximise returns to the size the client chose.
- `xdg_surface.set_window_geometry` places the window: the surface is drawn at the window's rectangle less the geometry's offset, so a point inside the geometry is the same point whatever the shadow width, and `xdg_toplevel.show_window_menu` anchors the same way.
- A window takes hits over the whole surface tree, so a subsurface that reaches past its parent receives pointer and touch input there (`Scene::set_hit_extent`).
- `xdg_toplevel.move` and `resize` start an interactive gesture when no shell holds the tree (`crates/gnoblin-wayland/src/interactive.rs`): a Smithay pointer or touch grab takes the device from the client, moves or resizes the window by how far the device went, and ends when the last button or the touch point is released. With a shell they stay node requests.
- The pointer follows the surface under it: a window or subsurface that moves, resizes, maps or closes under a pointer that has been placed gets `enter`, `leave` or `motion`.
- A touch point on a surface that is destroyed ends with `wl_touch.up`, closed by a frame, and not with `wl_touch.cancel`. wayland.xml defines `up` as "the touch point has disappeared" and `cancel` as "sent if the compositor decides the touch stream is a global gesture", which ends every point of the client; a window going away under a finger is the first. The wlcs test `sends_touch_up_on_surface_destroy` also asks for an end of the point (`touched_window()` is null afterwards), which only an `up` and its frame give: the suite's client has no `cancel` listener (libwayland aborts a process that gets an event with none), which is why the old cancel crashed the xdg variants. That is a gap in the suite, not a reason for the compositor's choice, and no patch is needed. `a_touch_point_on_a_destroyed_window_ends_with_an_up` in `crates/gnoblin-wayland/tests/protocols.rs` pins it; a device-level cancel still sends `cancel` (`a_cancelled_touch_ends_its_point_without_an_up`).
- Multi-client touch is the compositor's own claim, not wlcs's: the suite's four `TouchTest` tests (`tests/touches.cpp` in wlcs v1.8.1) each use one `wlcs::Client` and one touch point, so the ledger neither passes nor skips anything for two clients touching at once. It is pinned in `crates/gnoblin-wayland/tests/protocols.rs` with two real clients, each with a window and a `wl_touch`, and two injected points. Each client gets the `down`, `motion`, `up` and `frame` of its own point only, with the device slot as the touch id, and a `frame` closes only its own client's batch. Destroying one client's window, or the client disconnecting, ends only its point (an `up` and a frame, never a `cancel`); the other client's point goes on and ends normally. Two windows of one client report each point on its own window. This was broken before and the test found it: Smithay's default implicit touch grab (`TouchDownGrab`) sent every later point to the first point's surface, so a second client's finger was delivered to the first client with a position on the first client's surface. `ImplicitTouchGrab` in `crates/gnoblin-wayland/src/protocols/wl_seat.rs` replaces it and gives each point the focus under it at `down`; it still ends when the last point lifts, which `xdg_toplevel.move` and `resize` rely on for their serial. An interactive gesture started by a touch still takes the grab, and while it runs a second finger is delivered to nobody (`TouchDrag`).

`tests/wlcs/patches/0003-xdg-toplevel-tests-ack-the-initial-configure.patch` changes wlcs tests: the interactive ones attached a buffer before acknowledging the initial configure, and `wm_capabilities_are_sent` waited for an event that xdg-shell sends with the first configure, which follows the initial commit, without committing.

The popup tests (`XdgPopupPositionerTest`, `XdgPopupStable`, `XdgPopupTest`) pass but for one; 88 more are skipped because the suite's popup of a layer surface and of a `zxdg_shell_v6` surface need globals the compositor does not serve.

The core subset therefore does not pass yet: the gate in `design/rust-compositor.md` stays open until the issues above are closed or their entries become ledger decisions. What is in place is the runner, the regression gate over the 520 passing tests, and the list of what is left.

## Later-phase protocols

Measured on the box before any change (`scripts/wlcs.sh --all`), the later-phase entries of the ledger were stale: 18 of the 44 tests in these suites already passed and 26 failed. 39 pass now, and what is left is decided, not missing:

- **Pointer constraints and relative pointer (`PointerConstraints`, `RelativePointer`; 7 failures fixed).** A mouse reports motion as a delta (`InputEvent::PointerMotionRelative`), and that path held no lock, no confinement and sent no relative motion, so a game played with a mouse had none of them. The delta path now follows the same rules as a position: a lock turns it into the relative vector, a confinement clamps it, and the vector is reported either way. A confinement with no region is the whole surface, is held to the surface that has the pointer (not the one under where the motion would land) and stops on the last pixel, as wlroots does. A constraint starts when its window has the pointer and the keyboard and the pointer is inside its region, and ends when any of that stops (`State::update_pointer_constraints`), so a persistent one comes back when its window is chosen again. The first absolute position a device gives the pointer places it and reports no relative vector. Real-client tests: `a_locked_pointer_reports_a_mouse_as_relative_motion`, `a_confined_pointer_stays_inside_its_surface`, `a_confined_pointer_stays_inside_its_region`, `a_pointer_constraint_waits_until_its_window_has_the_keyboard` in `crates/gnoblin-wayland/tests/protocols.rs`.
- **Fractional scale (1 failure fixed).** The manager is the compositor's own, so `fractional_scale_exists` is raised on the manager and not, as Smithay does, on the `wl_surface`. Real-client tests: `a_second_fractional_scale_object_is_an_error_on_the_manager`, `a_surface_may_have_a_fractional_scale_object_again_once_the_first_is_gone`.
- **xdg-decoration (4 failures fixed).** `orphaned` is raised, on the manager like Smithay's other two decoration errors, when a toplevel is destroyed before its decoration (`request_tap.rs`, `tests/decoration.rs`). The three `set_mode` and `unset_mode` tests asked for a configure before the client committed; the decoration mode travels with the toplevel's configure and xdg-shell sends the first one as the answer to the initial commit, so `tests/wlcs/patches/0011-xdg-decoration-tests-commit-before-expecting-configure.patch` makes the tests commit first.
- **Text input v3 (9 of 9 failures fixed).**
    - A held enable: Smithay discards a text input's requests while no input method is bound, and the suite's input method binds after the application's `enable` and `commit`, so an input method that starts or restarts after the application would never hear of an enabled field. The compositor keeps the newest request of each kind per text input and gives them to Smithay when an input method binds (`request_tap.rs`, `zwp_text_input_v3::hold` and `replay`). It drops them when the keyboard focus moves, because `leave` resets a text input.
    - A destroyed text input: one that is the active one deactivates the input method even when its client has another (`end_if_active`; Smithay only does so for a client's last).
    - Real-client tests: `a_text_field_enabled_before_the_input_method_binds_is_activated`, `..._is_not_activated_after_it_lost_focus` and `destroying_the_active_text_input_deactivates_the_input_method`.
    - Patch `0010-text-input-tests-ack-the-initial-configure.patch` acknowledges the initial configure in the three popup and child-window tests before they attach a buffer.
    - The two tests `text_input_enters_grabbing_popup` and `text_input_enters_parent_surface_after_child_destroyed` failed for two reasons, one of the compositor's and one of the suite's:
        - The compositor's defect: a text input was told `leave` and `enter` twice for one move of the keyboard when an input method was bound. Smithay sends both as the keyboard moves, and the compositor's own sweep for the case with no input method sent a second pair, whose `leave` named the surface that was arriving.
        - A second defect of the compositor's: a popup that took the keyboard with `xdg_popup.grab` moved it around the text input when no input method was bound, because the grab set the keyboard focus directly.
        - The fix: every move of the keyboard goes through `State::move_keyboard_focus`. It lets Smithay send the pair when an input method is bound and sends the pair itself when none is.
        - Real-client tests, each run with and without an input method: `a_text_input_hears_one_leave_and_one_enter_when_the_keyboard_changes_windows` and `a_grabbed_popup_takes_the_text_input_with_the_keyboard_before_it_is_configured`. `a_popup_without_a_grab_leaves_the_text_input_where_it_is` pins the popup that asks for no grab.
        - In the wlcs fixture the input method's `get_input_method` sits unflushed in its client's buffer until the test does a round trip on that client. The two popup tests never do, so they are the tests that run with no input method.
        - The suite's defect: both tests open with `InSequence seq;`, which puts every expectation the fixtures make afterwards into one sequence. Among them are the `configure` any-number expectations of `XdgSurfaceStable`, `XdgToplevelStable` and `XdgPopupStable`, created after `EXPECT_CALL(text_input, enter(parent))`. A configure that arrives before that `enter` breaks the sequence, though the tests say nothing about configures. Without patch 0012 the failure is the first `configure_bounds` of the parent: "Unexpected mock function call".
        - The decision on the order: the compositor's is right and the suite's is wrong. The text-input protocol says the text-input focus follows the keyboard focus and states no order between `enter` and a configure. A toplevel takes the keyboard when it maps, and xdg-shell lets a client map only after it acknowledged the initial configure, so a conforming client always sees that configure first. A grabbing popup takes the keyboard at the grab request, before its first configure: the same rule applied to its own request.
        - `tests/wlcs/patches/0012-text-input-popup-tests-order-only-the-text-input-calls.patch` puts only the text input's calls in a named sequence and leaves the expectations unchanged. A real client pins the compositor's order in `a_text_input_enters_a_window_when_it_maps_after_the_window_was_configured`.
- **Clipboard and primary selection (5 failures kept).** The suite sets the selection from a client that never has the keyboard (another client's window took it), with serial 0. The compositor ignores a selection set by a client without the keyboard, as Mutter has since 3.34, so a background program cannot swap what the user is about to paste; a clipboard manager is privileged and uses data-control. That is a ledger decision, not a gap: `a_selection_from_a_client_without_the_keyboard_is_ignored` in `tests/selection.rs` pins it.

## CI

`.github/workflows/wlcs.yml` runs `scripts/wlcs.sh --gate` on pushes and pull requests that touch the compositor crates, the library, the script or the expectations, and on demand with `scope` set to `core` or `all`.

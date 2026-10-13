# Recovery input routing regression

Run from the repository root with initialized Mutter and ImGui sources:

```sh
CC=clang CXX=clang++ python3 tests/recovery-input/run.py --sanitize \
  --compile-commands build/make/mutter/compile_commands.json
```

The compile database is optional. In a worktree with empty submodules, pass
`--mutter-repo /path/to/existing/mutter` and
`--imgui-source /path/to/existing/imgui`. Those directories are read only.
Requirements are a C and C++ compiler, Python, Git, and GLib/GObject development
files. Sanitizers require the compiler's ASan and UBSan runtimes.

Reproduce the failure by omitting the candidate routing patch:

```sh
python3 tests/recovery-input/run.py --baseline \
  --test /recovery/dismiss-through-native-route
```

This must fail: the recovery panel remains visible, neither the stage nor UI
handler sees the input, and the client receives both button events. The
candidate passes nine cases, including Dismiss, outside clicks, releases
outside the panel, releases owned by the client, no focused client, dismissed
panels, replay, embargoes, and callback removal.

## Boundary under test

The runner reconstructs four files from Mutter's revision in
`gnome-versions.json`, then applies the repository's patch series to a temporary
directory. It extracts complete production functions without changing their
branches:

- `clutter_stage_handle_event`
- `_clutter_event_process_filters`
- `event_callback` and `meta_display_handle_event`
- `meta_wayland_pointer_handle_event`

It also runs the production `gnoblin_plugin_ui_handle_event` and
`gnoblin_imgui_overlay_set_pointer` functions, the complete recovery panel
implementation, and the bundled ImGui input queue and layout code. A frame
separates motion, press, and release. Button coordinates come from ImGui layout.
No rendering window, compositor, VM, or host service starts.

Device and service operations are stubs. They model a focused client surface,
record client button delivery and seat updates, and supply lock/capture/replay
state. The routing decisions and recovery hit testing are production code.
The stage queue records whether a captured-event handler could run; it does
not initialize Clutter's actor graph. Console transport is stubbed and is not
covered. This fixture does not prove libinput, painting, native focus, or an
installed runtime. Native VM testing remains a separate integration step.

## Candidate

`0125-route-diagnostic-input-before-wayland.patch` adds a private callback to
Mutter's existing event filter. The callback runs after the existing session
lock, input capture, consumed-replay, keybinding, and stage-grab checks and
after the Wayland seat update. It precedes window click handling and Wayland
delivery. The plugin registers a borrowed callback record on its display and
removes it before freeing the UI. There is no exported symbol or shell API.

The old stage captured-event hook is removed. With a focused Wayland surface,
Mutter consumes pointer input before that hook; with no focused surface, a
second hook would double-deliver unconsumed hover events. Existing panel
bounds and press ownership still decide which input the recovery UI consumes.
The full-stage actor remains non-reactive.

The matching executable and Mutter library must be rebuilt together. The
manifest copies the new private header into
`subprojects/mutter/src/compositor/plugins/gnoblin-plugin-input.h`; Mutter's
`src/core/events.c` includes it there. The executable compiles the plugin and
UI directly from `GNOBLIN_SOURCE_ROOT/src/compositor/plugin`.

On a prepared disposable integration tree, the normal CMake `mutter` target
applies source changes, copies overlays, and builds both artifacts. Use the
existing configured build directory and staging prefix. For an already
patched manual native tree, copy the manifest overlays, apply patch 0125 once,
then build both `src/gnoblin` and `src/libmutter-51.so.0.0.0` with Ninja. Do not
reset a shared or dirty submodule to prepare this candidate.

The saved `astra-warning-crash-native-build.log` identifies main's native
build as `/home/luna/dev/gnoblin/build/make/mutter` and its executable source
root as `/home/kieran/dev/gnoblin`. Main must integrate the four changed plugin
files and new header into that executable source root, copy the header through
the overlay manifest into the native Mutter source, and apply 0125 to that
source. After those steps, the targeted native build command is:

```sh
ninja -C /home/luna/dev/gnoblin/build/make/mutter \
  src/gnoblin src/libmutter-51.so.0.0.0
```

This command is for main's VM lane; this regression lane does not run it.
Use main's existing staging/install procedure for both outputs and start a
fresh VM session. Replacing either mapped file is not proof that the running
process uses the candidate.

The existing public instructions in `docs/troubleshooting.md` already describe
these recovery actions and lock hiding. This change restores that behavior;
no documented command, setting, or public interface changes.

## Native handoff

Main reproduced the missing click with a QMP pointer on the native seat and
passively traced motion, press, and release. Each reached the stage, display,
Wayland compositor, and Wayland pointer handlers; neither the stage capture
nor UI handler ran. Main reported clean debugger detach and a live compositor.
No debugger inferior calls were used. Issue tracking is owned by main
(`gnoblin-0dm`).

After integrating both artifacts, repeat the same native click with an
underlying Wayland surface, verify Dismiss hides the notice, and verify an
outside dock click still works. Also exercise release outside the panel and
the lock/capture gates. The console shares the moved bridge, so check its
normal keyboard and pointer interaction. Check the mapped hashes of both
artifacts in the fresh session. The candidate has no native-runtime proof yet.

Main's separate hidden-marker A/B still stuttered. This input candidate makes
no performance claim.

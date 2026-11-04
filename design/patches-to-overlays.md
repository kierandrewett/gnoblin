# Moving Gnoblin-owned code out of patches

Repo rule (`AGENTS.md`): Gnoblin-owned behaviour lives in Gnoblin's source and overlays; an upstream patch is a narrow
hook. This note is the plan and the record for moving the Gnoblin-owned code that `patches/mutter/` and
`patches/xdg-desktop-portal-gnome/` still carry into overlay sources under `src/`. It is one half of the patch tidy-up;
squashing the remaining fix-up chains (option B) was a separate, later step and this change does not squash unrelated
patches. `design/patches-squash-plan.md` records that step; the patch names in the tables below are those before it.

## Overlay mechanism and its limits

- `src/**/manifest` lines are `<project> <source> <destination>`. `gnoblin_overlay` (`cmake/source-lib.cmake`) copies
  each source into the subproject **before** `git am`, adds the destination to the checkout's `info/exclude`, and the
  Nix and tarball paths copy the same files. An overlay file is therefore untracked inside the subproject.
- A patch cannot edit an overlay file: `git am` sees no tracked file. A patch also cannot sit under an overlay file that
  replaces a tracked upstream file, because the working tree would no longer match the index. Gnoblin code that
  replaces an upstream file therefore gets a new file name, and the patch shrinks to the build line that selects it
  (the way `gnoblin-filechooser.c` already works).
- Overlay destinations stay where the code is today (`src/core/gnoblin-*.c`), so no `#include` in an upstream patch changes.

## Baseline

| Project                    | Patches before |
| -------------------------- | -------------- |
| `mutter`                   | 237            |
| `xdg-desktop-portal-gnome` | 14             |
| `portal-dependencies`      | 1              |

Baseline of the checks every phase has to match (worktree at `a6c07358`, `--without-portal` build, 8 jobs):

- Build: OK, 1043 compile steps, 110 compiler warnings.
- `ctest`: 19 of 20 pass. `gnoblinctl-cli` fails in a `--without-portal` build (it requires the portal component).
- Static patch tests (`tests/*.test.py` that read patches): all pass except `session-environment.test.py` (known, #328).
- Exported symbols of `libmutter-51.so`: 1091, saved to compare after each phase.
- Compiled source set (`meson introspect --targets`, `compile_commands.json`): saved to compare after the build-wiring phase.

## Inventory

### A. `gnoblin-animation.c` and `.h`: 16 patches

`patches/mutter/26-animation/0001` adds the 1.6k-line file and 15 more patches edit it. The table lists the lines each
patch adds to the two files and what else the patch touches. "deleted" means the patch has no other file and goes away.

| Patch                                                         | Added lines in the Gnoblin files | Other files (stay as hooks)                                                                                       |
| ------------------------------------------------------------- | -------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `26-animation/0001-native-animation-runtime`                  | 1647                             | `src/core/display.c`, `src/core/util.c`, `src/meson.build`, `src/meta/util.h`                                     |
| `26-animation/0002-retire-shell-owned-animation-events`       | 1                                | deleted                                                                                                           |
| `26-animation/0003-native-window-open-animation`              | 278                              | `src/compositor/meta-window-actor.c`, `src/core/util.c`, `src/meta/util.h`                                        |
| `26-animation/0004-lua-pattern-matcher`                       | 26                               | deleted                                                                                                           |
| `26-animation/0005-declare-animation-lock-check`              | 1                                | deleted                                                                                                           |
| `26-animation/0006-native-window-close-animation`             | 97                               | deleted                                                                                                           |
| `26-animation/0007-animation-no-match-is-valid`               | 2                                | deleted                                                                                                           |
| `26-animation/0008-export-animation-plugin-api`               | 2                                | deleted                                                                                                           |
| `26-animation/0009-native-window-minimize-restore-animations` | 66                               | deleted                                                                                                           |
| `26-animation/0010-select-configured-lifecycle-animations`    | 57                               | deleted                                                                                                           |
| `26-animation/0011-native-workspace-switch-animation`         | 412                              | deleted                                                                                                           |
| `26-animation/0012-native-window-resize-animation`            | 193                              | `src/compositor/compositor.c`, `src/compositor/meta-window-actor-private.h`, `src/compositor/meta-window-actor.c` |
| `26-animation/0013-retire-shell-only-animation-events`        | 1                                | deleted                                                                                                           |
| `26-animation/0015-native-workspace-animation-events`         | 73                               | deleted                                                                                                           |
| `26-animation/0016-explicit-animation-policy`                 | 0                                | `src/core/prefs.c`                                                                                                |
| `26-animation/0017-animate-dialogs`                           | 22                               | deleted                                                                                                           |

Target: `src/compositor/animation/gnoblin-animation.c` and `.h` with a `manifest` mapping them to
`src/core/gnoblin-animation.{c,h}`. The final file is the file the patches build today, byte for byte. The hooks that
remain are the animation calls in `meta-window-actor.c`, `compositor.c`, `meta-window-actor-private.h`, `display.c`,
`meta/compositor.h`, `prefs.c` and the declarations in `meta/util.h`.

### B. Native control dispatch built up in `src/core/util.c`: 35 patches

The patches append about 1.9k lines to the end of `util.c` and add 90 lines of small functions near the top. None of
the added code calls a static helper of upstream `util.c`: the whole block depends only on public and private
mutter headers. The functions are `meta_display_dispatch_gnoblin_event`, `..._shortcut_event`, `..._native_event`,
`meta_gnoblin_dispatch_native_api` (monitor, layer, window, workspace and typed-window operations),
`meta_gnoblin_initialize_native_workspaces`, `meta_gnoblin_workspace_id`, `meta_gnoblin_window_pattern_match`,
`meta_gnoblin_layer_anchor`, `meta_gnoblin_layer_namespace` and `meta_is_gnoblin_session`.

| Patch                                                                       | Added lines in `util.c` | Other files (stay as hooks)                                                                                                |
| --------------------------------------------------------------------------- | ----------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `03-layer-shell/0007-layer-anchor-api`                                      | 25                      | `src/meta/util.h`                                                                                                          |
| `09-focus-activation/0001-honour-app-activation`                            | 8                       | `src/core/util-private.h`, `src/core/window.c`, `src/wayland/meta-wayland-activation.c`                                    |
| `09-focus-activation/0002-use-supervisor-session-predicate`                 | 2                       | deleted                                                                                                                    |
| `19-native-control-core/0001-dispatch-runtime-events`                       | 21                      | `meson.build`, `src/meta/display.h`, `src/wayland/meta-wayland-pointer.c`                                                  |
| `22-native-monitor-layer-api/0001-dispatch-monitor-list-in-compositor`      | 40                      | `src/meta/util.h`                                                                                                          |
| `20-native-window-api/0002-dispatch-basic-window-actions-in-compositor`     | 135                     | `src/meta/util.h`                                                                                                          |
| `20-native-window-api/0003-dispatch-window-monitor-move-in-compositor`      | 16                      | deleted                                                                                                                    |
| `20-native-window-api/0004-list-windows-in-native-compositor`               | 107                     | deleted                                                                                                                    |
| `24-native-workspaces/0001-list-and-switch-workspaces-in-native-compositor` | 93                      | deleted                                                                                                                    |
| `24-native-workspaces/0002-manage-numeric-workspaces-in-native-compositor`  | 189                     | deleted                                                                                                                    |
| `24-native-workspaces/0003-own-workspace-ids-in-native-compositor`          | 113                     | `src/core/meta-workspace-manager.c`, `src/meta/util.h`                                                                     |
| `20-native-window-api/0005-dispatch-typed-window-operations`                | 410                     | deleted                                                                                                                    |
| `20-native-window-api/0006-complete-native-window-records`                  | 29                      | deleted                                                                                                                    |
| `20-native-window-api/0007-window-record-state-and-capabilities`            | 26                      | deleted                                                                                                                    |
| `22-native-monitor-layer-api/0002-stable-monitor-connector-ids`             | 132                     | deleted                                                                                                                    |
| `22-native-monitor-layer-api/0003-native-monitor-snapshot-records`          | 113                     | deleted                                                                                                                    |
| `20-native-window-api/0008-native-window-match`                             | 108                     | deleted                                                                                                                    |
| `24-native-workspaces/0004-workspace-state-changed-signal`                  | 2                       | `src/core/meta-workspace-manager.c`                                                                                        |
| `22-native-monitor-layer-api/0004-native-layer-snapshot`                    | 87                      | deleted                                                                                                                    |
| `25-shortcuts/0003-verified-shortcut-focus-context`                         | 37                      | `src/core/display-private.h`, `src/core/display.c`, `src/core/util-private.h`                                              |
| `24-native-workspaces/0005-refresh-native-workspaces-after-config-commit`   | 324                     | deleted                                                                                                                    |
| `24-native-workspaces/0006-keep-lua-workspace-names-out-of-gsettings`       | 1                       | deleted                                                                                                                    |
| `26-animation/0001-native-animation-runtime`                                | 14                      | `src/core/display.c`, `src/core/gnoblin-animation.c`, `src/core/gnoblin-animation.h`, `src/meson.build`, `src/meta/util.h` |
| `19-native-control-core/0002-dispatch-launch-feedback-natively`             | 3                       | deleted                                                                                                                    |
| `19-native-control-core/0003-lua-window-pattern-matcher`                    | 13                      | `src/meta/util.h`                                                                                                          |
| `26-animation/0003-native-window-open-animation`                            | 6                       | `src/compositor/meta-window-actor.c`, `src/core/gnoblin-animation.c`, `src/core/gnoblin-animation.h`, `src/meta/util.h`    |
| `16-session-lock/0011-dispatch-shell-session-lock-request`                  | 2                       | deleted                                                                                                                    |
| `22-native-monitor-layer-api/0005-include-layer-shell-snapshot-declaration` | 2                       | deleted                                                                                                                    |
| `20-native-window-api/0009-native-window-unminimize`                        | 3                       | deleted                                                                                                                    |
| `20-native-window-api/0010-native-window-resize-user-operation`             | 1                       | deleted                                                                                                                    |
| `20-native-window-api/0011-native-window-move-user-operation`               | 5                       | deleted                                                                                                                    |
| `26-animation/0014-animate-typed-window-resizes`                            | 12                      | `src/meta/compositor.h`                                                                                                    |
| `20-native-window-api/0012-accept-unminimize-in-id-argument-validation`     | 1                       | deleted                                                                                                                    |
| `20-native-window-api/0013-report-unresponsive-windows-and-kill`            | 24                      | deleted                                                                                                                    |
| `20-native-window-api/0014-add-window-raise-and-lower`                      | 8                       | deleted                                                                                                                    |

Target: `src/control/gnoblin-native-dispatch.c` and `.h` (destination `src/core/gnoblin-native-dispatch.{c,h}`). The
header declares the functions that only overlay code calls; declarations that upstream patched files use stay in the
upstream headers (`meta/display.h`, `core/util-private.h`, `meta/util.h`). **No hook remains in `util.c`**: the file goes
back to upstream, because no upstream code has to call into a static function there. The declarations in `meta/util.h`
(window frame API, `meta_display_dispatch_gnoblin_native_event`) are rebuilt as one small patch.

### C. Meson wiring: 37 patch sections

`src/meson.build` carries about 290 added lines in 36 patches and `src/compositor/plugins/meson.build` is edited by four
patches whose net effect is one blank line. All of it lists Gnoblin sources and protocols, or defines the Gnoblin
executable. Each patch chains onto the previous one's hunk.

| Patch                                                                            | Added lines in the Meson files | Other files (stay as hooks)                                                                                                                                                                                                                                                              |
| -------------------------------------------------------------------------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `03-layer-shell/0001-layer-shell-wire-wlr-layer-shell-into-the-build`            | 5                              | `src/wayland/meta-wayland-surface.c`                                                                                                                                                                                                                                                     |
| `04-wayland-protocols/0001-screencopy-wire-wlr-screencopy-into-the-build`        | 6                              | `src/wayland/meta-wayland-surface.c`                                                                                                                                                                                                                                                     |
| `04-wayland-protocols/0002-gnoblin-protocols-wire-gnoblin-s-extra-Wayland-proto` | 32                             | `src/wayland/meta-wayland-surface.c`                                                                                                                                                                                                                                                     |
| `07-cursor/0001-cursor-themes-and-launch-feedback`                               | 7                              | `config.h.meson`, `meson.build`, `meson.options`, `src/backends/meta-cursor-theme.c`, `src/backends/meta-cursor-tracker.c`, `src/backends/meta-cursor-xcursor.c`, `src/meta/meta-cursor-tracker.h`                                                                                       |
| `04-wayland-protocols/0003-buffer-fades`                                         | 3                              | `src/wayland/meta-wayland-surface.c`                                                                                                                                                                                                                                                     |
| `11-window-frames/0001-add-window-frame-protocol-to-meson`                       | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `11-window-frames/0002-native-window-frames`                                     | 3                              | `src/compositor/meta-window-actor-wayland.c`, `src/compositor/meta-window-actor.c`, `src/meta/util.h`, `src/wayland/meta-wayland-window-configuration.c`, `src/wayland/meta-wayland-window-configuration.h`, `src/wayland/meta-wayland-xdg-shell.c`, `src/wayland/meta-window-wayland.c` |
| `11-window-frames/0003-native-renderer-services`                                 | 2                              | `src/meta/util.h`, `src/wayland/meta-wayland-surface.c`                                                                                                                                                                                                                                  |
| `11-window-frames/0007-kde-server-decoration-protocol`                           | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `23-supervisor-runtime/0001-host-compositor-control-socket-in-native-process`    | 2                              | `src/core/mutter.c`                                                                                                                                                                                                                                                                      |
| `18-build-wiring/0002-link-native-control-into-libmutter`                        | 3                              | deleted                                                                                                                                                                                                                                                                                  |
| `26-animation/0001-native-animation-runtime`                                     | 1                              | `src/core/display.c`, `src/core/gnoblin-animation.c`, `src/core/gnoblin-animation.h`, `src/core/util.c`, `src/meta/util.h`                                                                                                                                                               |
| `21-input-routing/0002-native-touchpad-router`                                   | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0003-build-gnoblin-runtime-and-plugin`                          | 32                             | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0004-fix-introspection-source-types`                            | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0005-build-lua-pattern-matcher`                                 | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0006-build-pipewire-privacy-monitor`                            | 4                              | deleted                                                                                                                                                                                                                                                                                  |
| `28-window-clipping/0001-native-rounded-window-clipping`                         | 2                              | `src/compositor/meta-window-actor-wayland.c`, `src/compositor/meta-window-actor-wayland.h`                                                                                                                                                                                               |
| `18-build-wiring/0007-build-location-agent`                                      | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0008-build-keybinding-catalog`                                  | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0009-build-auth-agent`                                          | 3                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0010-build-control-domains`                                     | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0011-build-privacy-grants`                                      | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0012-build-prompt-broker`                                       | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0013-build-input-command-domains`                               | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0014-build-workspace-layer-domains`                             | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0015-build-shortcut-domain`                                     | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0016-build-window-rules-domain`                                 | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0017-build-window-drag-domain`                                  | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0018-build-runtime-domain`                                      | 1                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0019-build-control-split`                                       | 16                             | deleted                                                                                                                                                                                                                                                                                  |
| `23-supervisor-runtime/0003-build-monolithic-gnoblin`                            | 139                            | `src/core/mutter.c`                                                                                                                                                                                                                                                                      |
| `31-diagnostic-console/0005-build-compositor-diagnostic-ui`                      | 4                              | `meson.build`                                                                                                                                                                                                                                                                            |
| `31-diagnostic-console/0006-link-diagnostics-into-gnoblin`                       | 19                             | `src/core/mutter.c`                                                                                                                                                                                                                                                                      |
| `18-build-wiring/0020-link-input-disposition`                                    | 2                              | deleted                                                                                                                                                                                                                                                                                  |
| `18-build-wiring/0021-optional-ibus-input-method-build`                          | 10                             | `config.h.meson`, `meson.build`, `meson.options`                                                                                                                                                                                                                                         |
| `23-supervisor-runtime/0004-build-process-control`                               | 7                              | deleted                                                                                                                                                                                                                                                                                  |

Target: one hook patch `18-build-wiring/0001-include-gnoblin-meson-fragments` that holds only these edits to
`src/meson.build`:

1. `subdir('gnoblin-wiring')` before the protocol loop. The fragment adds the Gnoblin entries to `mutter_sources` and
   `wayland_protocols` and defines `gnoblin_libmutter_deps`.
2. One line in the `libmutter` dependency list that adds `gnoblin_libmutter_deps`.
3. The IBus and Hyprcursor entries of `mutter_lib_deps`. They must exist before `mutter_deps` is built, which is before
   any fragment can run, so they stay as a hook (6 lines).
4. The upstream `mutter` executable is replaced by `subdir('gnoblin-executable')`, a fragment that defines the monolithic
   `gnoblin` executable.

The fragments are overlay files (`src/mutter-build/`). The root `meson.build`, `meson.options` and `config.h.meson`
patches define options and feature flags and stay as patches. The `meta-wayland-surface.c` and `mutter.c` halves of the
mixed patches stay too.

### D. Test files added by patches

| Patch                                                                   | Test file it adds                                       | Needs mutter internals                              |
| ----------------------------------------------------------------------- | ------------------------------------------------------- | --------------------------------------------------- |
| `15-preferences/0002-gnoblin-global-monitor-scale`                      | `src/tests/gnoblin-monitor-settings-test.c` (89)        | yes: `meta-settings-private.h`, `meta-test` harness |
| `14-input-settings/0014-test-gnoblin-keyboard-accessibility-settings`   | `src/tests/a11y-gnoblin-settings-test.c` (144)          | yes                                                 |
| `14-input-settings/0018-test-gnoblin-trackball-pointing-stick-settings` | `src/tests/trackball-gnoblin-settings-test.c` (337)     | yes                                                 |
| `30-privacy-screen/0002-test-lua-privacy-screen-settings-override`      | `src/tests/gnoblin-privacy-screen-settings-test.c` (98) | yes                                                 |
| `30-privacy-screen/0005-test-runtime-privacy-screen-override`           | edits the file above                                    | yes                                                 |

All five use `meta-backend-private.h`, `meta-settings-private.h` or `meta-input-settings-private.h` and run in mutter's
`meta-test` harness, so they cannot move to Gnoblin's own `tests/`. They move to the overlay
(`src/mutter-build/tests/`, destination `src/tests/`) and their `test_cases` entries move into one fragment,
`src/mutter-build/tests.meson` (installed as `src/tests/gnoblin-tests/meson.build`), that `src/tests/meson.build` includes
with one `subdir()` line before it builds `test_cases`. The test sources must sit in `src/tests/` itself, because Meson
resolves a test case's `sources` against the directory of the file that builds the test, not that of the fragment that
lists it. `15-preferences/0002` and the input-settings export patch (the old `14-input-settings/0018`) keep their non-test
hooks (settings entry points, private-header declarations); `14-input-settings/0014`, `30-privacy-screen/0002` and
`30-privacy-screen/0005` are deleted, and one new patch, `18-build-wiring/0002-include-gnoblin-test-fragment`, holds the
include.
The roughly 20 `.metatest` files edited by `15-preferences/0014-gnoblin-new-window-placement` are upstream test data
for a changed default, not Gnoblin-owned files, and stay.

### E. Portal patches

| Patch                                                                                    | Gnoblin-owned part                                                                   | Target                                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `04-native-portals/0003-route-global-shortcuts-through-gnoblin` (880 added lines)        | rewrites `src/globalshortcuts.c`; adds two D-Bus XML files                           | `src/portal/backend/gnoblin-globalshortcuts.c` and `org.gnoblin.Shell.GlobalShortcuts{Provider,Rebind}.xml` (destinations `src/gnoblin-globalshortcuts.c`, `data/`); patch keeps the Meson lines only |
| `04-native-portals/0004-preview-wallpapers-with-gtk` (188 added lines)                   | rewrites `src/wallpaperpreview.c`                                                    | `src/portal/backend/gnoblin-wallpaperpreview.c` (destination `src/gnoblin-wallpaperpreview.c`); patch keeps the Meson lines only                                                                      |
| `03-file-chooser/0002`, `04-native-portals/0001`, `0002`, `0005`                         | Meson and `xdg-desktop-portal-gnome.c` wiring for overlay sources that already exist | stay                                                                                                                                                                                                  |
| `02-identity`, `05-access-and-screencast/*`, `01-build-compat/*`, `03-file-chooser/0001` | small edits to upstream code (`access.c`, `screencast.c`, `settings.c`)              | stay: edits of upstream behaviour, not Gnoblin-owned files                                                                                                                                            |
| `portal-dependencies/01-libgxdp/0001`                                                    | GTK 4.20 compatibility fix in upstream `libgxdp`                                     | stays                                                                                                                                                                                                 |

Upstream `globalshortcuts.c` and `wallpaperpreview.c` stay in the tree, unmodified and unbuilt, because the overlay may
not overwrite a tracked file.

### F. Gnoblin-owned functions inside upstream files that do not move now

These patches add Gnoblin functions, but the functions use `static` state or private structs of the upstream file they
sit in. Moving them needs a new private header per file and changes to upstream code, which is more than a relocation.
They are listed so the next step can pick them up.

| File                                                                                                    | Added lines   | Why it stays                                                             |
| ------------------------------------------------------------------------------------------------------- | ------------- | ------------------------------------------------------------------------ |
| `src/core/prefs.c`                                                                                      | 676           | `meta_prefs_apply_gnoblin_*` writes the file-static preference variables |
| `src/backends/meta-input-settings.c`, `meta-settings.c`, `meta-orientation-manager.c`                   | 712, 307, 85  | use the private object structs and static helpers                        |
| `src/x11/meta-x11-display.c`                                                                            | 478           | XSETTINGS publisher uses static helpers and new `MetaX11Display` fields  |
| `src/compositor/meta-window-actor.c`                                                                    | 440           | PNG preview and clipping use static actor state                          |
| `src/core/keybindings.c`, `src/core/events.c`                                                           | 372, 245      | extend static dispatch tables and event routing                          |
| `src/wayland/meta-wayland-activation.c`                                                                 | 321           | extends static activation state                                          |
| `clutter/clutter/clutter-paint-nodes.c`, `src/compositor/meta-surface-actor.c`, `meta-shaped-texture.c` | 284, 205, 181 | blur and sampling use paint-node internals                               |

## Phases

Each phase is one commit. Before a commit: `manage-patches.py check`, dense renumber, `series` updated, the build, `ctest`
and the static patch tests match the baseline, and a nested-session smoke runs the moved code.

1. **Inventory and plan**: this file.
2. **Animation overlay**: sections A. The patched mutter tree plus overlay must equal the old patched tree file for file.
3. **Native dispatch overlay**: section B. The functions of the old `util.c` and of the new overlay file are compared as a
   set of function bodies; the only differences are the include list and the prototypes.
4. **Meson fragments**: section C. The compiled source list and the target list of `meson introspect` must equal the
   baseline; the exported symbols must equal the baseline.
5. **Test overlay**: section D.
6. **Portal overlay**: section E.

## Results

All six phases are done, each in its own commit.

| Project                    | Patches before | Patches after |
| -------------------------- | -------------- | ------------- |
| `mutter`                   | 237            | 174           |
| `xdg-desktop-portal-gnome` | 14             | 14            |
| `portal-dependencies`      | 1              | 1             |

- **Animation** (phase 2): `src/compositor/animation/gnoblin-animation.{c,h}`. 12 patches deleted, 4 kept as hooks.
  The patched tree plus overlay equalled the old tree file for file; the overlay was then clang-formatted, which
  changed whitespace only.
- **Native dispatch** (phase 3): `src/control/gnoblin-native-dispatch.{c,h}`. 27 patches that touched only `util.c`
  are deleted, `util.c` is back to upstream and keeps no hook. The added lines of the old `util.c` and the body of the
  new file differ by one blank line. `meta/util.h` keeps the window frame and native event declarations in one small
  patch. The file kept Mutter's style for the move; a later commit formats it with the repo's clang-format and drops
  its `clang-check` exclusion. That commit is whitespace only: the files are byte-identical with all whitespace removed,
  and `git diff -w --word-diff=porcelain --word-diff-regex='[^[:space:]]'` shows no changed character. (A plain
  `git diff -w` is not empty, because `-w` does not ignore the line breaks that clang-format moves.)
- **Meson** (phase 4): `src/mutter-build/wiring.meson` and `executable.meson` (installed as
  `src/gnoblin-wiring/meson.build` and `src/gnoblin-executable/meson.build`). One patch,
  `18-build-wiring/0001-include-gnoblin-meson-fragments`, replaces 23 wiring patches and the `src/meson.build` hunks
  of 14 others. It keeps the IBus and Hyprcursor `mutter_lib_deps` lines because that list is read before any fragment
  can run. The root `meson.build`, `meson.options` and `config.h.meson` patches stay: they define options and feature
  flags. The four plugin `meson.build` edits cancelled out (one blank line) and are gone.
- **Tests** (phase 5): `src/mutter-build/tests/*-test.c` (destination `src/tests/`) and the fragment
  `src/mutter-build/tests.meson` (installed as `src/tests/gnoblin-tests/meson.build`). Three patches that only added
  test code are deleted; the trackball patch keeps its header exports as `14-input-settings/0017-export-input-settings-to-tests`
  and `15-preferences/0002` keeps its settings hooks. One new patch, `18-build-wiring/0002-include-gnoblin-test-fragment`,
  adds the `subdir()` line, so `mutter` goes from 176 to 174 patches. The patched tree equals the old one except for
  `src/tests/meson.build`. The four tests build and pass in a Mutter build with `-Dtests=enabled` (five test cases, none
  skipped). The normal build uses `-Dtests=disabled`, which never reads `src/tests/`, so the check needs that separate
  configuration; it also needs `mutter = gnoblin` added in a scratch copy, because `src/tests/meson.build` still runs
  `test('get-state-tool-gio', mutter, ...)` and the monolithic `gnoblin` executable replaced Mutter's `mutter` target
  long before this work (already so at `a6c07358`). That is not fixed here.
- **Portal** (phase 6): `src/portal/backend/gnoblin-globalshortcuts.c`, `gnoblin-wallpaperpreview.c` and the two
  `org.gnoblin.Shell.GlobalShortcuts{Provider,Rebind}.xml` files. The existing overlay mechanism already copies files
  into the portal project (`manifest` names the project), so CMake needed no change. An overlay may not replace a
  tracked file, so the Gnoblin files get their own names and `0003-route-global-shortcuts-through-gnoblin` and
  `0004-preview-wallpapers-with-gtk` shrink to Meson lines (the Shell interfaces, `json-glib`, `glycin`, the source
  names). Upstream `globalshortcuts.c` and `wallpaperpreview.c` stay in the tree, unbuilt. The patched tree equals the
  old one except that the two files sit under their new names (equal to the old rewritten files byte for byte before
  formatting) and `src/meson.build` names them. The portal builds with them and `ctest` passes 20 of 20.
- Compiled sources after phase 4: the target list of `meson introspect` equals the baseline except that
  `libmutter` gains `native-dispatch.c` (phase 3); the `compile_commands.json` flags differ only in the build
  directory of the `gnoblin` executable. The 1091 exported `libmutter` symbols are unchanged after every phase.
- Tests that read patch text now read the overlay (`native-socket-text-snap-api`, `session-lock-api`,
  `location-agent-contract`, `session-environment`). `ctest` stays at 19 of 20 in a `--without-portal` build
  (`gnoblinctl-cli` needs the portal) and is 20 of 20 with the portal.
- `design/patches-layout-mapping.tsv`: the `new` column is the patch's current path. A patch deleted because its code
  moved to an overlay has `overlay:<path>` instead (a path that no longer exists would mislead).
- **`clutter_actor_insert_child_above` criticals.** The nested smoke (animation lifecycle and the native dispatch calls)
  prints `Clutter-CRITICAL ... clutter_actor_insert_child_above: assertion 'child->priv->parent == NULL' failed` in
  pairs. They are not new: a build of `origin/main` `a6c07358` (Mutter 51.0 plus its overlay and 237 patches, built
  from a scratch worktree and a scratch patched Mutter checkout) prints the same four in the animation smoke and the same three pairs in the native smoke as
  this branch does, with the same animation and workspace event counts. No fix belongs to this change.

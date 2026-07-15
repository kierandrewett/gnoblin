# Testing

gnoblin's automated checks are layered: deterministic logic and script tests,
isolated headless GNOME Shell boots, Mutter's in-tree compositor suite, and
RPM builds. The Shell integration tier runs headless. Mutter's native and
Wayland tests still need a real seat and working local file monitoring.
GDM login, visible chrome, and portal consent remain manual checks covered by
[Real-hardware verification](real-hardware-verification.md).

## Reference

| Recipe | Proves | Needs |
|---|---|---|
| `just test` / `just verify-fast` | Shell/Python syntax, fatal-log detection, private-state handling, RPM source staging, and the C config-parser regression suite | No build |
| `just test-config` | Config parser behaviour across documented quoting, comment, repeated-key, fallback, and enumeration cases | No build |
| `just test-mutter` | Gnoblin's Mutter patches (layer-shell, protocol overlays, WM/crash fixes) do not regress Mutter's unit/Wayland/native/focus suites | A real environment with a working local file monitor (inotify) and a seat; see below |
| `just gnome-verify` | Patched GNOME Shell boots in `gnoblin` mode, advertises `zwlr_layer_shell_v1`, suppresses the native panel, accepts incompatible extension metadata, and keeps `org.gnome.Shell.Eval` restricted | Current install prefix (`just dev`) |
| `just gnome-stock-protocol-isolation-verify` | The same packages boot in stock `user` mode without Gnoblin protocols or control APIs, with the native panel, upstream extension validation, and notification ownership intact | Current install prefix |
| `just gnome-protocol-boundaries-verify` | Owned Wayland globals bind and disconnect cleanly; foreign-toplevel stop/destruction and screencopy/layer-shell invalid geometry follow their protocol contracts | Current install prefix |
| `just gnome-dbus-verify` | The `org.gnoblin.Shell` control protocol round-trips over D-Bus, including typed listing and scoped revocation for Screen Cast and Remote Desktop grants | Current install prefix |
| `just gnome-devkit-verify` | The devkit's spawned-terminal environment (isolated bus plus `gnoblinctl` on `PATH`) reaches `org.gnoblin.Shell` | Current install prefix |
| `just gnome-hot-reload-verify` | Editing an extension and calling `ReloadExtension` loads the new code, reports completion only after the import finishes, and rejects broken replacement code | Current install prefix |
| `just gnome-scripting-verify` | The GJS user-scripting layer (`~/.config/gnoblin/scripts/*.js`) loads and waits for asynchronous reload completion | Current install prefix |
| `just gnome-notifications-verify` | In Gnoblin mode, disabling `notifications` releases `org.freedesktop.Notifications` and re-enabling it reclaims the name | Current install prefix |
| `just gnome-protocol-gating-verify` | Turning a protocol off in `gnoblin.conf` stops it being advertised | Current install prefix |
| `just verify-installed-headless` | Every isolated GNOME Shell integration recipe above, run serially against the current install | Current install prefix |
| `just verify` | `verify-fast`, a fresh `just dev`, and the complete installed headless suite | Build dependencies |
| `just verify-release` | `verify`, Mutter's real-host suite, and both RPM builds | Real host plus RPM build dependencies |

Use the narrow recipe while iterating. Use `just verify` for the complete local
gate: it rebuilds the current source before running every isolated Shell
integration test. `just verify-release` adds the real-host Mutter suite and
both package builds; it is intentionally unsuitable for restricted sandboxes.

## Day-to-day

- Editing `src/config/`: `just test-config` — it's a plain C compile + run,
  no build needed.
- Editing a protocol overlay (`src/protocols/`) or a Mutter patch: `just
  gnome-verify` after `just dev-mutter` + `just dev-gnome-shell` (or the
  matching `gnome-*-verify` for the specific behaviour you changed, e.g.
  `gnome-protocol-gating-verify` for a `[protocols]` gate).
- Editing `gnoblinControl.js` or the `org.gnoblin.Shell` protocol: `just
  gnome-dbus-verify`.
- Editing anything devkit-related (`scripts/run-gnome-devkit.sh`,
  `scripts/devkit_dbus.py`, `scripts/devkit-document-portal-stub.py`): `just
  gnome-devkit-verify`.
- Before calling a normal code change done: run its focused recipe, then
  `just verify` when the build dependencies are available. Use
  `just verify-release` for package or release work on a real host.

## `test-mutter` and sandboxes

The native/Wayland backend tests in `just test-mutter` boot a compositor
that monitors an ICC profile directory. In a sandbox without a working local
file monitor (inotify), they all bail with *"Unable to find default local
file monitor type"* (exit 251) — this is environmental, not a gnoblin
regression: the unit tests (no backend needed) still pass, and the
ref-image tests log "Image matched" before bailing on the same missing
monitor. Run this tier on real hardware, or wherever inotify actually works.

## Writing a new verify script

Follow the existing `scripts/test-*.sh` pattern: boot what you need with
`scripts/run-gnome-shell.sh`'s approach (throwaway `HOME`/`XDG_*_HOME`, real
isolation) or `scripts/run-gnome-devkit.sh`'s (real `HOME`/`XDG_RUNTIME_DIR`,
isolated D-Bus session bus only — see [Devkit § Isolation](devkit.md#isolation)
for the distinction), using `scripts/devkit_dbus.py` for the isolated D-Bus
config either way. Assert on shell logs, protocol errors, or D-Bus responses,
then add a `gnome-*-verify` recipe and include it in
`verify-installed-headless`.

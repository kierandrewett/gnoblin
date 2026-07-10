# Testing

gnoblin's automated suite is layered: a fast logic tier with no display,
Mutter's own in-tree suite, and a set of headless `gnome-shell` boots that
each prove one thing end to end. All of it runs headless. What can't be
automated (a real GDM login, a real GPU, unattended screensharing) is
covered by [Real-hardware verification](real-hardware-verification.md)
instead.

## Reference

| Recipe | Proves | Needs |
|---|---|---|
| `just test-config` | The C `gnoblin.conf` parser (`src/config/`) matches its documented grammar | Nothing built |
| `just test-mutter` | gnoblin's Mutter patches (layer-shell, protocol overlays, WM/crash fixes) don't regress Mutter's own unit/Wayland/native/focus suites | A real environment with a working local file monitor (inotify) and a seat — see the note below |
| `just gnome-verify` | Patched gnome-shell boots in the `gnoblin` session mode and advertises `zwlr_layer_shell_v1` | `./install` (`just dev`) |
| `just gnome-dbus-verify` | The `org.gnoblin.Shell` control protocol round-trips over D-Bus (Ping/GetVersion/Reload) | `./install` |
| `just gnome-devkit-verify` | The devkit's spawned-terminal environment (isolated bus + `gnoblinctl` on `PATH`) actually reaches `org.gnoblin.Shell` | `./install` |
| `just gnome-hot-reload-verify` | Editing an extension's code and calling `Reload` picks up the new code without a relogin | `./install` |
| `just gnome-scripting-verify` | The GJS user-scripting layer (`~/.config/gnoblin/scripts/*.js`) loads and hot-reloads | `./install` |
| `just gnome-notifications-verify` | Disabling the `notifications` feature releases `org.freedesktop.Notifications` (and reclaims it when re-enabled) | `./install` |
| `just gnome-protocol-gating-verify` | Turning a protocol off in `gnoblin.conf`'s `[protocols]` section stops it being advertised | `./install` |

`just test` runs the tier-1 (`test-config`) suite and prints what else needs
a build. There's no single "run everything" recipe beyond that — `test-mutter`
needs a real environment (see below) and the `gnome-*-verify` recipes need
`./install`, so running them all in one shot as part of a routine `just test`
would silently fail in most sandboxes rather than telling you why.

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
- Before anything you'd call "done": run the specific verify(s) for what you
  touched, not the whole suite — see the table above for what each one
  actually needs.

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
config either way. Assert on the shell log / D-Bus responses, and add a
`just gnome-*-verify` recipe pointing at it.

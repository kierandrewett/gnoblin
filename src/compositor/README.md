# gnoblin-shell — the gnoblin compositor (Phase 2 spike)

A from-scratch, first-class C program that embeds **libmutter-17** and drives it
with its own `MetaPlugin` (`GnoblinShellPlugin`), instead of running gnome-shell.
This is the pivot's foundation: mutter is the compositor/WM library; gnome-shell is
just its default plugin, and gnoblin-shell replaces it.

Unlike the protocol overlays under `src/<feature>/` (copied into the mutter
submodule), gnoblin-shell is **standalone** — concrete, owned source linking the
*installed* `libmutter-17` via pkg-config. No submodule, no patches.

## Files
- `gnoblin-shell.c` — `main()`: `meta_create_context` → configure → set plugin
  gtype → setup → start → run main loop (modelled on mutter's `src/core/mutter.c`).
  Optional `-- COMMAND` is spawned once ready (for autostarting the session).
- `gnoblin-shell-plugin.{c,h}` — the `MetaPlugin`. Minimal today: `start` shows the
  stage and mutter handles windows with default (instant) effects.

## Build
```sh
meson setup build/gnoblin-shell src/shell
meson compile -C build/gnoblin-shell
./build/gnoblin-shell/gnoblin-shell --version
```
Requires the `libmutter-17` development package (gnoblin's patched mutter provides
it). **Status: builds + links + runs `--version` (validated).**

## Devkit — build + run the whole stack in an isolated sandbox

The dev flow builds gnoblin into a local `./install` prefix (so the Mutter Devkit
viewer exists and everything links against the patched mutter — no system install,
no root) and runs it in its own dbus + GSettings + XDG sandbox:

```sh
just dev            # build+install patched mutter (+ devkit viewer), gnoblin-shell,
                    # topbar/dock, and the org.gnoblin.shell schema -> ./install
just devkit-verify  # headless: boot the stack, print the protocols it advertises
just devkit         # open the Mutter Devkit window running gnoblin-shell
just devkit foot    # ...with `foot` running inside the nested compositor
```

`just devkit-verify` is **validated** — it boots gnoblin-shell on the patched
libmutter and confirms all the wlr-/ext- protocols are live globals. `just devkit`
launches the windowed `mutter-devkit` viewer on your current session (it needs a
host display to show the window).

Implementation: `scripts/run-devkit.sh` (env from `./install`, isolated state),
`scripts/install-userspace.sh` (clients + schema), `scripts/wl-globals.c` (registry
dumper). gnoblin-shell **boots as a full compositor** — proven via the devkit.

## Still ahead
- **Real login session:** a `gnoblin.desktop` entry launching `gnoblin-shell --
  gnoblin-session`, picked at the login manager (needs a seat — beyond the nested
  devkit). Decision gate for committing fully to the pivot.
- **Effects:** more vtable overrides (`minimize`/`switch_workspace`) + a
  `ClutterEffect` on `MetaWindowActor` for corners/shadows/blur, attach in
  `gnoblin-shell-plugin.c`. Open/close (`map`/`destroy`) animations are done.

# Shell settings

Gnoblin does not provide a `gnoblin.configure.shell` section. A shell owns its
panels, docks, launchers, notifications, and other user interface, so configure
those in the shell you run. Configs that still contain a top-level `shell`
table are rejected instead of silently ignoring its settings.

Use Gnoblin's Lua API for compositor and session behavior:

- Register compositor transitions with [`gnoblin.animation`](/config/animation).
- Match windows and select per-window transitions with
  [window rules](/config/window_rule).
- Configure layer-surface behavior with
  [`gnoblin.configure.layer_shell`](/config/configure/layer_shell).
- Bind compositor operations with
  [`gnoblin.configure.shortcuts`](/config/configure/shortcuts).

See [choose a shell](/bring-your-own-shell) for shell projects that run on
Gnoblin.

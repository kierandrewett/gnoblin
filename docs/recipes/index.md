# Recipes

Each recipe combines settings to solve a desktop task. Add its Lua to
`~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines, then run
`gnoblinctl config reload`.

## Desktop setup

- [Build a small desktop](/recipes/small-desktop): start a bar and notification
  daemon, add launcher and terminal shortcuts, and set a cursor theme.
- [Organize a growing config](/recipes/organize-config-files): split settings
  by purpose while keeping load order predictable.

## Input workflows

- [Set up a laptop for two keyboard layouts](/recipes/laptop-input): keep US
  and UK layouts per window, use Caps Lock as Escape, and configure touchpad
  gestures.
- [Set up a pen display](/recipes/pen-display-workflow): map a tablet without
  stretching the drawing area and use the stylus button to switch monitors.

## Windows and workspaces

- [Set up a focused writing workspace](/recipes/writing-workspace): name fixed
  workspaces, place new windows consistently, and style editor windows without
  fading their text.

## Privacy and accessibility

- [Prepare remote support access](/recipes/remote-support): ask by default and
  grant one trusted support app limited access to a selected monitor and input.
- [Make pointer feedback easier to see](/recipes/pointer-feedback): enlarge
  the cursor, locate it on demand, and turn client bell requests into a visual
  signal.

See the [configuration reference](/config/configure) for every supported
option and the [guides](/guides/permissions) for behavior that depends on the
session or portal backend.

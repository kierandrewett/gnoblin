# Recipes

Short Lua examples for common desktop configuration tasks. Add a recipe to
`~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines so your choices
override settings loaded from other files.

After editing the file, apply the changes with `gnoblinctl config reload`.

- [Complete starter config](/recipes/complete-starter-config): start with a
  working shell, shortcut and window rule setup.
- [Add a shortcut](/recipes/add-a-shortcut): add or change one named shortcut
  without replacing other shortcuts.
- [Dim unfocused windows](/recipes/dim-unfocused-windows): lower the opacity of
  windows that are not focused.
- [Round application windows](/recipes/round-application-windows): set window
  corner radius and smoothing.
- [Turn off layer animations](/recipes/turn-off-layer-animations): disable
  compositor animations for layer surfaces.
- [Request a Gnoblin titlebar](/recipes/request-a-titlebar): provide a server
  titlebar when an application requests one.
- [Combine window rules](/recipes/combine-window-rules): apply defaults and
  override selected values for matching windows.
- [Split a config into files](/recipes/split-config-into-files): load separate
  files for appearance and shortcuts.
- [Set up a writing workspace](/recipes/writing-workspace): assign stable
  workspace IDs, place editor windows, and control workspaces from the CLI.

See the [configuration reference](/config/configure) for available options and
the [configuration guides](/guides/window_rules) for behavior and tradeoffs.

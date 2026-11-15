# Recipes

Short TypeScript examples for common desktop configuration tasks. Each recipe is a statement or two you add to your
`~/.config/gnoblin/init.ts`, or a set of files that `init.ts` imports. The file runs top to bottom against live objects,
such as `gnoblin.input`, `gnoblin.workspaces` and `gnoblin.clients`, and nothing is layered under it: see the
[complete starter](/recipes/complete-starter-config) for the stock file.

A save reloads the running shell, and `gnoblinctl config reload` does it on request.

- [Complete starter config](/recipes/complete-starter-config): start with a working focus and shortcut setup.
- [Add a shortcut](/recipes/add-a-shortcut): add or change one key without replacing other keys.
- [Dim unfocused windows](/recipes/dim-unfocused-windows): lower the opacity of windows that are not focused.
- [Combine window rules](/recipes/combine-window-rules): apply a default, then override it for matching windows.
- [Split a config into files](/recipes/split-config-into-files): import separate files for appearance and shortcuts.
- [Set up a writing workspace](/recipes/writing-workspace): declare stable workspace IDs, route editor windows, and bind
  keys that show a workspace or move a window.
- [Build a small desktop](/recipes/small-desktop): bind a launcher and a terminal, and start a bar with the session.
- [Set up a laptop keyboard and touchpad](/recipes/laptop-input): set key repeat, tap to click and natural scrolling.
- [Organize a growing config](/recipes/organize-config-files): split window placement and shortcuts into imported files.
- [Route new app windows to named workspaces](/recipes/put-apps-on-named-workspaces): send new editor, browser and chat
  windows to stable workspace IDs.

See the [shell API](/shell-api) for the objects and the [configuration guides](/guides/window_rules) for behaviour and
tradeoffs. The cursor theme, pointer feedback and tablet settings cannot be set from `init.ts`, so they have no recipe.
For fades, see [animations](/guides/animations).

# Recipes

Short TypeScript examples for common desktop configuration tasks. Each recipe is a complete `init.ts`, or a set of files
that `init.ts` imports. Merge a recipe into the `defineConfig` call of your `~/.config/gnoblin/init.ts`: a table key such
as `keys` merges with the stock config, and a list such as `rules` replaces it. See the
[defineConfig reference](/typescript-config) for how each key combines.

A save reloads the running shell, and `gnoblinctl config reload` does it on request.

- [Complete starter config](/recipes/complete-starter-config): start with a working focus and shortcut setup.
- [Add a shortcut](/recipes/add-a-shortcut): add or change one key without replacing other keys.
- [Dim unfocused windows](/recipes/dim-unfocused-windows): lower the opacity of windows that are not focused.
- [Combine window rules](/recipes/combine-window-rules): apply defaults and override selected values for matching
  windows.
- [Split a config into files](/recipes/split-config-into-files): import separate files for appearance and shortcuts.
- [Set up a writing workspace](/recipes/writing-workspace): declare stable workspace IDs, route editor windows, and bind
  keys that show a workspace or move a window.
- [Build a small desktop](/recipes/small-desktop): start a bar, notifications, launcher and terminal.
- [Set up a laptop for two keyboard layouts](/recipes/laptop-input): configure XKB layouts, keyboard options and
  touchpad settings.
- [Make pointer feedback easier to see](/recipes/pointer-feedback): enlarge the cursor and enable locate-pointer and
  visual-bell effects.
- [Set up a pen display](/recipes/pen-display-workflow): map a tablet and configure a stylus button.
- [Organize a growing config](/recipes/organize-config-files): split rules and shortcuts into imported files.
- [Route new app windows to named workspaces](/recipes/put-apps-on-named-workspaces): send new editor, browser and chat
  windows to stable workspace IDs.

See the [configuration reference](/config/configure) for available options and the
[configuration guides](/guides/window_rules) for behaviour and tradeoffs. For fades, see
[animations](/guides/animations).

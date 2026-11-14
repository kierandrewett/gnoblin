# Migrating from Lua

The TypeScript shell is the default shell. It reads `init.ts` and does not read `init.lua`. The Lua shell stays for one
more release so that an update never replaces your configuration with the stock one without telling you.

## What happens to an `init.lua` today

- If your config directory has an `init.lua` and no `init.ts` or `init.js`, and you have not set `GNOBLIN_SHELL_VM`, the
  session runs the Lua shell as before and logs one warning that names this page.
- Once `init.ts` exists, the TypeScript shell runs. The `init.lua` beside it is ignored and the log says so.
- `GNOBLIN_SHELL_VM=lua` runs the Lua shell whatever is on disk. It is deprecated and is removed after this release.

See [Which shell runs](typescript-shell.md#which-shell-runs) for the full table.

## Move your config

1. Write a starter: `gnoblinctl config init`. It adds `init.ts` and the editor files beside your `init.lua` and changes
   nothing else.
2. Move your settings across using the table below. `gnoblinctl check --types init.ts` reports a mistake with its line,
   and a retired key is refused with the key to use instead.
3. Log out and back in. A running Lua session keeps reading `init.lua`; from the next session the TypeScript shell runs and
   `init.ts` takes precedence.

There is no converter. Shell files (`shell.lua`, `shell/*.lua`) are arbitrary Lua over the object model and cannot be
translated mechanically.

## What each construct becomes

| In Lua                                                                | In TypeScript                                                                                                                      |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.configure { ["window-management"] = { ... } }`               | `defineConfig({ windowManagement: { ... } })`. Kebab and camel case keys both work, and an unknown key fails with a suggestion.    |
| `shortcuts`, `keybindings`                                            | `keys: { "Super+Return": spawn("foot") }`. A Lua `<Super>Return` chord is `Super+Return`. A shortcut on key release is not ported. |
| `workspaces = { { id, name } }`                                       | `workspaces: [{ id, layout }]`.                                                                                                    |
| `autostart = { ... }`                                                 | `autostart: ["waybar", ["foot", "--server"]]`. The session starts them and stops one you remove.                                   |
| `input`, `cursor`, `compositor`, `protocols` and the other sections   | The same names in camel case, as `defineConfig` keys. See the [defineConfig reference](typescript-config.md).                      |
| `gnoblin.load("config/*.lua")`, `require`                             | `import "./config/10-windows.ts"`. There is no glob.                                                                               |
| `gnoblin.on("gnoblin.window.created", fn)`                            | `on: { clientAdded(client) {} }`, or a `ClientListener`. Only the window events are mapped.                                        |
| `seat.data.window_rules = { { match = { app_id = "^x$" }, ... } }`    | `rules: [{ appId: /^x$/, ..., all: true }]`. Write `\.` where Lua wrote `%.`, and add `reapply: true` to re-evaluate on a change.  |
| `seat:bind({ ["<Super>x"] = fn })`, `Swipe3`                          | `keys: { "Super+x": ({ gnoblin }) => ... }`, and `gnoblin.input.primary.bind` for gestures.                                        |
| `seat.data.prompts = false`, `seat.data.alerts = false`               | `alerts: false`, `menus: false`. Prompts are drawn by the compositor and have no switch.                                           |
| `gnoblin.timers.after`, `gnoblin.process.spawn`                       | `gnoblin.after`, `gnoblin.spawn`.                                                                                                  |
| Reactive properties, `go`, `await`, `every`, `Node:extend`, selectors | None. Rewrite them as listeners and `gnoblin.after`.                                                                               |

Settings that were in the Lua document are in the TypeScript document with the same meaning, and each page under
`config/configure` says how it is applied.

# Window rule reference

[Shell files](/user-shell)

A window rule is one table in the list `seat.data.window_rules`, set in `~/.config/gnoblin/shell.lua`. The
[window rules guide](/guides/window_rules) has usage patterns.

```lua
seat.data.window_rules = {
    {
        match = { app_id = "^org%.example%.Editor$", focused = false },
        workspace = { id = "3" },
        maximised = true,
        opacity = 0.9,
    },
}
```

## Rule fields {#rule-fields}

| Field        | Type    | Effect                                                       |
| ------------ | ------- | ------------------------------------------------------------ |
| `match`      | Table   | The conditions a window must meet. Every key given must hold |
| `workspace`  | Table   | `{ id = "3" }`: the workspace, by name, the window opens on  |
| `maximised`  | Boolean | `true` maximises the window, `false` unmaximises it          |
| `fullscreen` | Boolean | `true` or `false`                                            |
| `secure`     | Boolean | `true` masks the window in screen capture                    |
| `opacity`    | Number  | From 0 to 1                                                  |

A rule with an unknown field, or a field of the wrong type, is refused. So is an `opacity` outside 0 to 1.

## Match keys

| Key            | Type    | Matches                                                  |
| -------------- | ------- | -------------------------------------------------------- |
| `app_id`       | Pattern | The application ID, or the WM class when the ID is empty |
| `title`        | Pattern | The current title                                        |
| `focused`      | Boolean | Whether the window has keyboard focus                    |
| `workspace_id` | String  | The name of the window's workspace                       |

A pattern is a [Lua pattern](https://www.lua.org/manual/5.4/manual.html#6.4.1), case-sensitive, matched anywhere in the
text. A malformed pattern refuses the whole list.

## Merging

Rules merge in list order. The last matching rule wins for each field, and a field a later rule does not set keeps the
earlier value. A field that no matching rule sets falls back to its default.

## Removed fields {#refused-keys}

The stock shell draws blur and shadow only, so these fields from earlier versions are refused. A rule that names one is
an unknown field.

- `corners`
- `blur`, `blur-ignore-shadows`, `shader`, `shader-uniforms` and `animation`
- `frame`
- `match.type`, `match.layer`, `match.client_type` and `match.workspace_number`: use `match.app_id` or
  `match.workspace_id` instead.

## Rules in `init.ts` {#javascript-rules}

The JavaScript shell reads its rules from `defineConfig({ rules: [...] })` in `init.ts`. A rule is a plain object: the
query keys the window must meet, the actions to take, and two options. It is checked when the config loads, and a
mistake names the rule's index and the field (`rules[2].appId: must be a glob string or a RegExp, got 5`).

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    workspaces: [{ id: "desk" }, { id: "dev" }],
    rules: [
        { appId: "org.example.Editor", workspace: "dev", maximised: true },
        { appId: "pinentry*", floating: true, position: { x: 400, y: 300 }, skipFocus: true },
        { title: /^Secret/, secure: true },
    ],
});
```

| Key           | Type                    | Meaning                                                                                   |
| ------------- | ----------------------- | ----------------------------------------------------------------------------------------- |
| `appId`       | glob string or `RegExp` | The application ID                                                                        |
| `title`       | glob string or `RegExp` | The current title                                                                         |
| `focused`     | Boolean                 | Whether the window has keyboard focus when the rule is read                               |
| `onWorkspace` | String                  | The id of the workspace the window is on (Lua `match.workspace_id`)                       |
| `workspace`   | String                  | The id of the workspace the window opens on; the config must declare it                   |
| `floating`    | Boolean                 | `true` keeps the window out of its workspace's tiling                                     |
| `size`        | `{ width, height }`     | The size the floating layout places the window at, or its size when `position` is given   |
| `position`    | `{ x, y }`              | Places the window there; the layout leaves it alone                                       |
| `style`       | `{ opacity, ... }`      | `opacity` from 0 to 1 is applied; `cornerRadius` and `shadows` are kept and not drawn yet |
| `fullscreen`  | Boolean                 | `true` covers the screen, `false` puts the window back                                    |
| `maximised`   | Boolean                 | `true` fills the work area, `false` puts the window back. A fullscreen window does not    |
| `secure`      | Boolean                 | `true` masks the window in screen capture                                                 |
| `skipFocus`   | Boolean                 | The window does not take focus when it opens                                              |
| `first`       | Boolean                 | When this rule matches, no later rule is consulted                                        |
| `reapply`     | Boolean                 | Default `true`. `false` applies the rule once, when the window opens                      |

A query key not given matches anything, and a rule needs at least one query key and one action. An unknown key is
refused by name, so `corners`, `blur` and the other [removed fields](#refused-keys) fail at load. `role` is refused too:
the compositor publishes no window role.

**Matching.**

- Every matching rule applies, in list order, and the later rule wins for each field they both set. A rule with `first: true` ends the search where it matches: the rules before it and the rule itself still merge, the ones after it are not consulted.
- A rule applies when its window opens and again when the app id, the title or the focus changes, as the Lua rules always did. Give a rule `reapply: false` to apply it once, when the window opens.
- A rule that stops matching sets `secure`, `fullscreen`, `maximised`, `opacity`, the corner radius and the shadows back to what a rule that still matches sets, or to their defaults when none does. A rule that still matches writes nothing.
- A window that was already open when the config loaded is not touched, so a reload does not move windows you have placed.

### From Lua patterns

A glob matches the whole text: `*` is any run of characters and `?` is one character. Everything else is itself, so
`.` and `%` need no escape. A `RegExp` is tested anywhere in the text, as a Lua pattern was.

| Lua rule                                 | JavaScript rule                                           |
| ---------------------------------------- | --------------------------------------------------------- |
| `match = { app_id = "^foot$" }`          | `appId: "foot"`                                           |
| `match = { app_id = "fire" }`            | `appId: "*fire*"`                                         |
| `match = { app_id = "^org%.mozilla%." }` | `appId: "org.mozilla.*"`                                  |
| `match = { app_id = "^org%.x%.(%w+)$" }` | `appId: /^org\.x\.\w+$/`                                  |
| `match = { title = "^Secret" }`          | `title: "Secret*"`                                        |
| `match = { title = "%d+ unread" }`       | `title: /\d+ unread/`                                     |
| `match = { focused = true }`             | `focused: true`                                           |
| `match = { workspace_id = "2" }`         | `onWorkspace: "2"`                                        |
| `workspace = { id = "3" }`               | `workspace: "3"`                                          |
| `opacity = 0.9`                          | `style: { opacity: 0.9 }`                                 |
| `maximised`, `fullscreen`, `secure`      | the same names                                            |
| the last matching rule wins for a field  | the same, with no option; `first: true` stops at a rule   |
| re-evaluated when any input changed      | `reapply`, on by default: app id, title and focus changes |
| `xr = false`                             | not ported: the key is refused                            |
| the WM class when the app id is empty    | not ported: the tree carries no class                     |

In a `RegExp`, the Lua classes `%a`, `%d`, `%w` and `%s` are `[A-Za-z]`, `\d`, `[A-Za-z0-9]` and `\s`, an escaped
character such as `%.` is `\.`, the lazy `-` is `*?`, and `^` and `$` anchor as they did.

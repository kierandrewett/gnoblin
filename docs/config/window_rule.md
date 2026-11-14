# Window rule reference

A window rule is one object in the `rules` list of `defineConfig` in `~/.config/gnoblin/init.ts`. The
[window rules guide](/guides/window_rules) has usage patterns. A rule is the query keys a window must meet, the actions
to take, and two options. It is checked when the config loads, and a mistake names the rule's index and the field, as in
`rules[2].appId: must be a glob string or a RegExp, got 5`.

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

## Rule fields {#rule-fields}

| Key           | Type                    | Meaning                                                                                 |
| ------------- | ----------------------- | --------------------------------------------------------------------------------------- |
| `appId`       | glob string or `RegExp` | The application ID                                                                      |
| `title`       | glob string or `RegExp` | The current title                                                                       |
| `focused`     | Boolean                 | Whether the window has keyboard focus when the rule is read                             |
| `onWorkspace` | String                  | The id of the workspace the window is on                                                |
| `workspace`   | String                  | The id of the workspace the window opens on; the config must declare it                 |
| `floating`    | Boolean                 | `true` keeps the window out of its workspace's tiling                                   |
| `size`        | `{ width, height }`     | The size the floating layout places the window at, or its size when `position` is given |
| `position`    | `{ x, y }`              | Places the window there; the layout leaves it alone                                     |
| `style`       | `{ opacity, ... }`      | `opacity` from 0 to 1 is applied; `cornerRadius` and `shadows` are stored and not drawn |
| `fullscreen`  | Boolean                 | `true` covers the screen, `false` puts the window back                                  |
| `maximised`   | Boolean                 | `true` fills the work area, `false` puts the window back. A fullscreen window does not  |
| `secure`      | Boolean                 | `true` masks the window in screen capture                                               |
| `skipFocus`   | Boolean                 | The window does not take focus when it opens                                            |
| `first`       | Boolean                 | When this rule matches, no later rule is consulted                                      |
| `reapply`     | Boolean                 | Default `true`. `false` applies the rule once, when the window opens                    |

A query key you leave out matches anything, and a rule needs at least one query key and one action. A rule with an
unknown key, or a field of the wrong type, is refused by name, and so is an `opacity` outside 0 to 1.

## Match patterns

`appId` and `title` take a glob string or a `RegExp`.

- A glob matches the whole text. `*` is any run of characters and `?` is one character. Everything else is itself, so
  `.` needs no escape.
- A `RegExp` is tested anywhere in the text. Anchor it with `^` and `$` to match the whole text.
- Matching is case-sensitive.

| To match                                    | Write                    |
| ------------------------------------------- | ------------------------ |
| Exactly `foot`                              | `appId: "foot"`          |
| Any app id that contains `fire`             | `appId: "*fire*"`        |
| Any app id that starts with `org.mozilla.`  | `appId: "org.mozilla.*"` |
| `org.x.` and one word of letters and digits | `appId: /^org\.x\.\w+$/` |
| A title that starts with `Secret`           | `title: "Secret*"`       |
| A title with a number and the word `unread` | `title: /\d+ unread/`    |
| A window that has the keyboard focus        | `focused: true`          |
| A window on workspace `2`                   | `onWorkspace: "2"`       |

## Matching and merging

- Every matching rule applies, in list order, and the later rule wins for each field they both set. A rule with
  `first: true` ends the search where it matches: the rules before it and the rule itself still merge, the ones after
  it are not consulted.
- A rule applies when its window opens and again when the app id, the title or the focus changes. Give a rule
  `reapply: false` to apply it once, when the window opens.
- A rule that stops matching sets `secure`, `fullscreen`, `maximised`, `opacity`, the corner radius and the shadows back
  to what a rule that still matches sets, or to their defaults when none does. A rule that still matches writes nothing.
- A window that was already open when the config loads is not touched, so a reload does not move windows you have
  placed.

## Fields that are refused {#refused-keys}

The stock shell draws opacity and, on the GLES renderer, shadow and blur from the window style. A rule does not take
these keys, and one that names one fails at load:

- `corners`
- `blur`, `blur-ignore-shadows`, `shader`, `shader-uniforms` and `animation`
- `frame`
- `match.type`, `match.layer`, `match.client_type` and `match.workspace_number`: use `appId` or `onWorkspace` instead.
- `role`: the compositor publishes no window role.
- `xr`, and matching on the WM class when the app id is empty: the tree carries no class.

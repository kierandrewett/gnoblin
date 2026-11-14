# Route new app windows to named workspaces

Give workspaces stable IDs in `init.ts`, then place each matching new window on its workspace when it opens. Replace
the example app IDs with the values reported by `gnoblinctl window list --json`.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    workspaces: [{ id: "code" }, { id: "web" }, { id: "chat" }],
    rules: [
        { appId: "org.gnome.TextEditor", workspace: "code" },
        { appId: "org.mozilla.firefox", workspace: "web" },
        { appId: "org.gnome.Fractal", workspace: "chat" },
    ],
});
```

The shell builds one workspace for each declared entry and names it by its `id`, so a rule targets a workspace by that
ID. IDs must be unique non-empty strings. The first one is shown when the shell starts.

| Option      | Values and default                          | Effect                                                 |
| ----------- | ------------------------------------------- | ------------------------------------------------------ |
| `appId`     | A glob or a `RegExp`; unset matches any app | Selects windows by application ID.                     |
| `workspace` | The id of a declared workspace; unset       | Places a matching new normal window on that workspace. |

A glob matches the whole ID, so `"org.mozilla.firefox"` matches only that app. Use `"org.mozilla.*"` for every app of
that prefix, or a `RegExp` for anything finer.

Save the file and the running shell reloads it. A window that is already open is not placed again.

Placement happens once; it does not lock a window to its workspace. You can move it afterward, and the shell leaves it
there.

- If an app reuses an existing window, the shell does not place it again.
- Dialogs stay with their parent window.
- A rule that names a workspace the config does not declare fails the load, so a typo is caught before it moves a
  window.

See [workspace rules](/guides/window_rules#workspaces) and the
[window-management reference](/config/configure/window_management).

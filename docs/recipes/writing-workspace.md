# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and bind keys that show a workspace or move the
focused window to one. Add this to `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig, show } from "gnoblin";

export default defineConfig({
    workspaces: [
        { id: "write", name: "Write", layout: "monocle" },
        { id: "research", name: "Research", layout: "master-stack" },
        { id: "review", name: "Review" },
        { id: "other", name: "Other" },
    ],
    rules: [{ appId: "org.gnome.TextEditor", workspace: "write" }],
    keys: {
        ...show.numbered("Super+{n}", ["write", "research", "review", "other"]),
        "Super+Shift+w": ({ gnoblin }) => {
            const workspace = gnoblin.workspaces.get("write");
            if (workspace !== undefined) {
                gnoblin.clients.focused?.moveToWorkspace(workspace);
            }
        },
    },
});
```

Each object declares a workspace. Its ID stays attached when the workspace position changes; the list order sets the
initial positions. Declare every ID that a rule or a `show` key uses: a `show` or a rule that names a workspace the
config does not declare fails the load, and the message names the key or rule.

| Field    | Accepted value                                  | Meaning                                                                                           |
| -------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `id`     | Required, unique non-empty string               | Stable workspace identity used by rules, keys and `gnoblinctl`.                                   |
| `name`   | A string; defaults to the id                    | Label shown for the workspace.                                                                    |
| `layout` | A layout name or object; defaults to `floating` | How the windows of the workspace are arranged. See [`workspaces`](/typescript-config#workspaces). |

The list replaces the stock four workspaces, so keep at least one entry. The first workspace is shown when the shell
starts. `show.numbered` binds `Super+1` to the first ID and so on, and `Super+Shift+w` moves the focused window to
`write` without changing the shown workspace.

The editor's `workspace` field takes effect once, when the window opens. Dialogs stay with their parent window. Placement
does not lock a window to its workspace: you can move it afterward, and the shell leaves it there.

Save the file, then list the workspaces:

```sh
gnoblinctl workspace list
```

Replace the app id with the value `gnoblinctl window list` prints for your editor. Workspace positions can change when
workspaces are removed or reordered; use configured IDs in rules and keys. See [workspace rules](/guides/window_rules#workspaces)
and the [window management reference](/config/configure/window_management).

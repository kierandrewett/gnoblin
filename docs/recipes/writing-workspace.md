# Set up a writing workspace

Give a writing workspace a stable ID, send new editor windows there, and bind keys that show a workspace or move the
focused window to one. Add this to `~/.config/gnoblin/shell.ts`:

```ts
import { ClientCollectionListener } from "gnoblin-core";
import { FloatingLayout, MasterStackLayout, MonocleLayout, Workspace } from "gnoblin-std";
import type { Client } from "gnoblin-core";

const layouts = {
    write: new MonocleLayout(),
    research: new MasterStackLayout(),
    review: new FloatingLayout(),
    other: new FloatingLayout(),
};
const names = { write: "Write", research: "Research", review: "Review", other: "Other" };

const input = gnoblin.input.primary;
const ids = ["write", "research", "review", "other"] as const;
ids.forEach((id, index) => {
    gnoblin.workspaces.add(new Workspace(id, { name: names[id], layout: layouts[id] }));
    input.bind(`Super+${index + 1}`, () => {
        gnoblin.workspaces.byId(id)?.show();
    });
});

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            const write = gnoblin.workspaces.byId("write");
            if (client.appId === "org.gnome.TextEditor" && write !== undefined) {
                client.moveToWorkspace(write);
            }
        }
    })(),
);

input.bind("Super+Shift+w", () => {
    const write = gnoblin.workspaces.byId("write");
    if (write !== undefined) {
        input.focusedClient?.moveToWorkspace(write);
    }
});
```

Each `new Workspace` declares a workspace. Its `id` stays attached when the workspace position changes, and the order of
the `add` calls sets the initial positions. `gnoblin.workspaces.byId` returns `undefined` for an id you did not declare, so
the code tests it before it moves a window.

| Argument or option | Accepted value                    | Meaning                                                                              |
| ------------------ | --------------------------------- | ------------------------------------------------------------------------------------ |
| first argument     | Required, unique non-empty string | Stable workspace identity used by code, keys and `gnoblinctl`.                       |
| `name`             | A string; defaults to the id      | Label shown for the workspace.                                                       |
| `layout`           | A layout object                   | How the windows of the workspace are arranged. See [layouts](/shell-api/workspaces). |

Your file replaces the stock four workspaces, so keep at least one. The first workspace added is shown when the shell
starts. `Super+1` shows the first ID and so on, and `Super+Shift+w` moves the focused window to `write` without changing
the shown workspace.

The editor listener takes effect once, when the window opens. Dialogs stay with their parent window. Placement does not
lock a window to its workspace: you can move it afterward, and the shell leaves it there.

Save the file, then list the workspaces:

```sh
gnoblinctl workspace list
```

Replace the app id with the value `gnoblinctl window list` prints for your editor. Workspace positions can change when
workspaces are removed or reordered; use the ids in keys and listeners. See [window rules](/guides/window_rules#workspaces)
and the [workspaces](/shell-api/workspaces) reference.

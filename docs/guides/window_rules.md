# Window rules

A window rule is code in your `shell.ts` that runs for the windows it matches. Gnoblin has no rule table. You attach a
listener to `gnoblin.clients` and decide in a function, or you style a selection of windows with
`gnoblin.clients.where(...)`. The file runs top to bottom against live objects, and a save reloads it.

## Add a rule

This places every editor window on workspace `dev` and maximises it:

```ts
import { ClientCollectionListener } from "gnoblin-core";
import { FloatingLayout, Workspace } from "gnoblin-std";
import type { Client } from "gnoblin-core";

gnoblin.workspaces.add(new Workspace("web", { layout: new FloatingLayout() }));
gnoblin.workspaces.add(new Workspace("dev", { layout: new FloatingLayout() }));

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.example.Editor") {
                const dev = gnoblin.workspaces.byId("dev");
                if (dev !== undefined) {
                    client.moveToWorkspace(dev);
                }
                client.maximised = true;
            }
        }
    })(),
);
```

`onAdded` runs when a window opens, and once for each window that is already open when the file loads, so a save
applies the rule to the windows on screen too. Run `gnoblinctl check --types shell.ts` before you save.

The listener may use the window's members from `onAdded`, the library's too: a rule that writes `client.bounds` and
`client.floatingBounds` puts a new window there, and a `FloatingLayout` leaves a window that has floating bounds where it
is, instead of placing it in free space.

## Choose what to match

A rule is an `if`. Test any property of the client:

| Property         | Matches                                  |
| ---------------- | ---------------------------------------- |
| `client.appId`   | The application ID.                      |
| `client.title`   | The current title.                       |
| `client.focused` | `true` for the window that has the focus |

Choose the property based on what identifies the window:

- Use `appId` to match an application across its windows.
- Use `title` to target a particular document or window. Titles often change as the document or page changes.

Layer-shell surfaces are not windows, so a listener never sees them.

## Find the values

List the live windows with:

```sh
gnoblinctl window list
```

Use the `app_id` and `title` of the window you want as the values to match.

## Write a matcher

Compare with `===` for a whole id, or use a `RegExp` for a part. A `RegExp` is tested anywhere in the text, so use `^`
and `$` to match the whole value. Both are case-sensitive.

```ts
import { ClientCollectionListener } from "gnoblin-core";
import type { Client } from "gnoblin-core";

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.gnome.Nautilus") {
                client.maximised = true;
            }
            if (/Notes/.test(client.title)) {
                client.maximised = false;
            }
        }
    })(),
);
```

The `/Notes/` test also matches `Notes - Work`. For only the title `Notes`, write `client.title === "Notes"`.

A title or app id can change after the window opens. A rule written in `onAdded` sees the values at that moment. To
react to a change, add `onTitleChanged(client, title)` or `onAppIdChanged(client, appId)` to a `ClientListener`.

## Style the windows of one app

Appearance does not need a listener. `where` selects windows by property, live, and the selection has its own style:

```ts
import "gnoblin-std";

const nautilus = gnoblin.clients.where({ appId: "org.gnome.Nautilus" });
nautilus.style.opacity = 0.9;
```

A window that stops matching the query goes back to the style it inherits. The scopes, and the properties, are in
[window style](/js-style).

## Workspaces {#workspaces}

Declare the workspaces yourself with `new Workspace(id, { layout })` and `gnoblin.workspaces.add`, and fetch one by its
id with `gnoblin.workspaces.byId("code")`. It returns `undefined` for an id you did not declare, so test it before you
move a window:

```ts
import { ClientCollectionListener } from "gnoblin-core";
import { FloatingLayout, Workspace } from "gnoblin-std";
import type { Client } from "gnoblin-core";

for (const id of ["code", "web"]) {
    gnoblin.workspaces.add(new Workspace(id, { layout: new FloatingLayout() }));
}

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            const target = client.appId === "org.example.Editor" ? gnoblin.workspaces.byId("code") : undefined;
            if (target !== undefined) {
                client.moveToWorkspace(target);
            }
        }
    })(),
);
```

To act on the windows already on a workspace, read `workspace.clients`, a collection with its own `style`.

## Order

Code runs in the order you wrote it. A later statement overrides what an earlier one set, so put the general rule first
and the one for a single app after it. A later `maximised = false` beats an earlier `maximised = true`.

## Window flags

A listener sets `client.secure` to keep a window out of screen captures, and `client.floating = true` to mark a window
floating, so a tiling layout leaves it where it is. Skip-focus has no writable property.

```ts
import { ClientCollectionListener } from "gnoblin-core";
import "gnoblin-std"; // gives a window its `floating` member
import type { Client } from "gnoblin-core";

gnoblin.clients.addListener(
    new (class extends ClientCollectionListener {
        onAdded(client: Client) {
            if (client.appId === "org.example.Player") {
                client.secure = true;
                client.floating = true;
            }
        }
    })(),
);
```

A window rule is not re-applied for you when a window's title or focus changes: a handler that should react to one
attaches its own `onTitleChanged` or `onFocusChanged`.

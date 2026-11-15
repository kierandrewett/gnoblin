# Window management settings

The focus, raise, tiling and titlebar-click policy of the compositor has no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings). The runtime keeps these values and `init.ts`
cannot change them:

- Focus follows a click. Moving the pointer does not change focus.
- A new window takes focus, and a window raises when you click it.
- Dropping a window on a screen edge tiles it, and a dragged window stays inside the work area.
- A double click on a stock titlebar toggles maximise, a right click opens the window menu, and a middle click does
  nothing.

A client that draws its own titlebar handles those clicks itself, and GTK apps usually do.

## Move and resize with the pointer

The floating layout binds `Super` and a left drag to move a window and `Super` and a right drag to resize it from the
nearest corner. These gestures work on application windows with client or server decorations. To bind another drag, set
the layout's triggers when you build it:

```ts
import { FloatingLayout, Workspace } from "gnoblin";

gnoblin.workspaces.add(
    new Workspace({
        id: "main",
        layout: new FloatingLayout({ triggers: { move: "Alt+MouseLeft+Drag", resize: "Alt+MouseRight+Drag" } }),
    }),
);
```

## Workspaces

Declare workspaces with `gnoblin.workspaces.add`, as in [workspaces](/shell-api/workspaces). To place a window on a
workspace when it opens, write a listener on `gnoblin.clients`: see [window rules](/guides/window_rules).

# Window management settings

The `windowManagement` key sets the compositor's focus, raise, tiling and titlebar-click policy. A reload applies
them, except `attachModalDialogs` and `disableWorkarounds`, which nothing reads.

The `focus`, `edgeTiling` and `dragConstraint` keys of `defineConfig` are the same policy written for the shell: see
[the shell's own keys](#the-shells-own-keys).

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    windowManagement: { focusMode: "hover" },
    workspaces: [{ id: "main" }, { id: "chat" }],
});
```

## Settings at a glance

### Focus and raising

| Key                        | Values                                 | Default     | Effect                                                    |
| -------------------------- | -------------------------------------- | ----------- | --------------------------------------------------------- |
| `focusMode`                | `"click"`, `"hover"`, `"hover-strict"` | `"click"`   | Selects when pointer movement changes focus.              |
| `focusNewWindows`          | `"allow"`, `"prevent"`                 | `"prevent"` | Selects the new-window focus policy.                      |
| `raiseOnClick`             | Boolean                                | `true`      | Raise a window when clicked.                              |
| `autoRaise`                | Boolean                                | `false`     | Raise the focused window automatically.                   |
| `autoRaiseDelay`           | 0–10000 ms                             | `500`       | Delay before automatically raising the focused window.    |
| `focusChangeOnPointerRest` | Boolean                                | `false`     | Change focus after the pointer rests over another window. |

### Placement and responsiveness

| Key                       | Values          | Default | Effect                                                                           |
| ------------------------- | --------------- | ------- | -------------------------------------------------------------------------------- |
| `checkAliveTimeout`       | 0–4294967295 ms | `5000`  | Client ping timeout before it is marked frozen; `0` disables the check.          |
| `edgeTiling`              | Boolean         | `false` | Tile a window dropped on a screen edge.                                          |
| `attachModalDialogs`      | Boolean         | `false` | Place modal dialogs with their parent window. Nothing reads it.                  |
| `disableWorkarounds`      | Boolean         | `false` | Disable compatibility workarounds for broken X11 applications. Nothing reads it. |
| `constrainDragToWorkArea` | Boolean         | `true`  | Keep dragged windows inside the work area.                                       |

`constrainDragToWorkArea` controls how far a dragged window can leave the work area, which is the monitor minus the
space that panels reserve.

- With `true`, the whole window stays inside the work area.
- With `false`, a window can move until only a strip of it stays on screen, at most 75 pixels tall.

The table shows the compositor default when a key is missing. The stock config sets these values instead, so they are
what you get unless your own config changes them:

| Key                         | Stock value | Why                                                                         |
| --------------------------- | ----------- | --------------------------------------------------------------------------- |
| `attachModalDialogs`        | `true`      | A modal dialog opens centred over its parent and moves with it.             |
| `edgeTiling`                | `true`      | Dragging a window to a screen edge tiles it.                                |
| `focusNewWindows`           | `"allow"`   | A window started from a launcher takes focus. `"prevent"` leaves it behind. |
| `actionMiddleClickTitlebar` | `"none"`    | A stray middle click does not lower a window.                               |

Set any of them in your own config to change it.

### Titlebar buttons and actions

| Key                         | Values                | Default             | Effect                              |
| --------------------------- | --------------------- | ------------------- | ----------------------------------- |
| `actionDoubleClickTitlebar` | Titlebar action below | `"toggle-maximize"` | Action for a titlebar double-click. |
| `actionMiddleClickTitlebar` | Titlebar action below | `"lower"`           | Action for a titlebar middle-click. |
| `actionRightClickTitlebar`  | Titlebar action below | `"menu"`            | Action for a titlebar right-click.  |

## Focus behaviour

`focusMode` controls how pointer movement changes focus:

- `"click"` focuses a window when you click it. Moving the pointer does not change focus.
- `"hover"` focuses a window when the pointer enters it. If the pointer leaves all windows, focus returns to the last
  window you used that can take focus.
- `"hover-strict"` also focuses on pointer entry, but clears focus when the pointer leaves all windows.

Set `focusChangeOnPointerRest: true` to change focus only after the pointer rests briefly.

`focusNewWindows` controls when app activation requests can focus a window:

- `"prevent"` turns focus-stealing prevention on. It is the compositor's value when the key is missing.
    - Activation requests need recent launch or user activity.
    - A newly opened window must also be a transient descendant of the focused window, such as a dialog opened by that
      app.
    - Requests without valid recent activity leave the window unfocused, including a window you start from a launcher
      and a password dialog.
    - When no window has focus, nothing can be stolen, so a new window takes focus.
- `"allow"` is the stock value. It allows applications to activate their windows after you have interacted with
  another window since they opened them.

## Move and resize with the pointer

The floating layout binds `Super` and a left drag to move a window and `Super` and a right drag to resize it from the
nearest corner. These gestures work on application windows with client or server decorations. To bind another drag,
see [triggers](/typescript-config#keys).

## Workspaces

Declare workspaces in the top-level `workspaces` key of `defineConfig`, not here. See
[`workspaces`](/typescript-config#workspaces). Window rules that place a window on a workspace are in the
[window rules guide](/guides/window_rules#workspaces).

## Titlebar actions

The three titlebar click fields accept these values:

| Value                          | Action                                      |
| ------------------------------ | ------------------------------------------- |
| `toggle-maximize`              | Toggle maximise.                            |
| `toggle-maximize-horizontally` | Accepted and does nothing.                  |
| `toggle-maximize-vertically`   | Accepted and does nothing.                  |
| `minimize`                     | Minimise the window.                        |
| `lower`                        | Lower the window in the stacking order.     |
| `menu`                         | Open the window menu (needs `menus: true`). |
| `none`                         | Do nothing.                                 |

These settings control what a click on the stock titlebar does. Applications that draw their own client-side
titlebars handle those clicks themselves, and GTK applications usually do, so this section does not change them.

## The shell's own keys

The shell reads this policy from the main seat, so a reload changes its next focus, raise or titlebar decision. The
`defineConfig` keys below set the same policy for the shell alone. A key you leave out is the seat's.

- `focus: { mode, newWindows, raiseOnClick, autoRaise, autoRaiseDelay, changeOnPointerRest }`. The modes are
  `click`, `sloppy` (this page's `hover`) and `follow-mouse` (`hover-strict`).
- `edgeTiling` and `dragConstraint`.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    focus: { mode: "sloppy", autoRaise: true, autoRaiseDelay: 300 },
    edgeTiling: true,
    dragConstraint: "minimum_strip",
});
```

Window rules and workspaces are not part of this document: see [window rules](/config/window_rule) and the
[defineConfig reference](/typescript-config).

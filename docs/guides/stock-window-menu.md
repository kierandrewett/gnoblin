# The window menu and the not-responding alert

The stock shell draws two small dialogs that belong to a window: the window menu, and the alert for a window that has
stopped answering. Both are the shell's own interface, so a config can change what they offer or draw its own. Each has a
switch: leave `WindowMenus.get().start()` or `NotRespondingAlerts.get().start()` out of your `shell.ts`.

## The window menu

The menu opens when:

- You right-click a titlebar. The titlebar runs the seat's `right_click` action, which is `menu` by default.
- A client calls `xdg_toplevel.show_window_menu` with a serial the compositor gave it on an input event. A request with any
  other serial is ignored, whether or not a shell is running: a menu takes the keyboard, so a client could otherwise steal it.
- You press the window-menu key.
- A program calls `window.request_menu`.

The compositor sends each of these to the shell as a `menu` request on the window, with the anchor in global coordinates.
While a shell holds the tree, no `menu_requested` signal goes out, so a second program does not open a second menu over the
shell's.

The menu offers:

| Entry             | What it does                                                   |
| ----------------- | -------------------------------------------------------------- |
| Maximise, Restore | Maximises the window. Restore leaves maximise or fullscreen.   |
| Minimise          | Minimises the window.                                          |
| Move, Resize      | Starts the keyboard mode. Off for a maximised or tiled window. |
| Always on top     | Keeps the window over windows that are raised later.           |
| Move to workspace | Opens a submenu with a workspace on each line.                 |
| Close             | Asks the window to close.                                      |

An entry that does not apply is shown dim and the keyboard skips it.

### Keys and pointer

- Up, Down, Tab and Shift+Tab move the selection. Home and End go to the ends.
- A letter jumps to the next entry that starts with it.
- Return or Space runs the selected entry. Right opens a submenu, and Left closes it.
- Escape closes the submenu, or the menu when none is open.
- The pointer selects the entry it is over, and a left click runs it.
- A press anywhere else closes the menu. The press is not passed on.

Only one menu is open at a time. Asking for another closes the first. The menu also closes when its window closes, when the
session locks, and when the not-responding alert opens. No menu opens while the session is locked.

The menu for a `secure` window is `secure`, so a screen capture masks it. The menu takes the keyboard while it is open and
gives it back when it closes.

### Move and Resize

Move and Resize use the keyboard. The arrow keys move the window by 16 pixels, or grow and shrink it when you chose Resize.
Return keeps the new place and Escape puts the window back. The window stays inside the work area. The same modes are
`client.moveWithKeys()` and `client.resizeWithKeys()`: see [Window policy](/js-window-policy#move-and-resize-with-the-keyboard).

## Change what the menu offers

Give the menu a `MenuProvider` in your `shell.ts`. Its `items(client)` method takes a window, as its id in text, and
returns the list of entries to show. Each entry is a `MenuEntry`: a `label` and any of these:

| Field            | Meaning                                                                       |
| ---------------- | ----------------------------------------------------------------------------- |
| `run`            | A function that receives the window's id. It runs after the menu has closed.  |
| `enabled`        | `false` shows the entry dim, and the keyboard skips it.                       |
| `checked`        | `true` shows a tick.                                                          |
| `submenu`        | A list of entries. Use it instead of `run`.                                   |
| `submenuEntries` | A function that takes the window's id and returns the entries of the submenu. |
| `separator`      | `true` draws a rule between groups. An entry with `separator` has no label.   |

```ts
import { MenuEntry, MenuProvider, WindowMenus } from "gnoblin-std";

const menus = WindowMenus.get();

class CopyTitle extends MenuProvider {
    items(id: string): MenuEntry[] {
        const window = gnoblin.clients.get(id);
        const entries = window === undefined ? [] : menus.defaultItems(window);
        const copy = new MenuEntry("Copy title");
        copy.run = (target: string) => {
            gnoblin.spawn(["wl-copy", gnoblin.clients.get(target)?.title ?? ""]);
        };
        entries.splice(entries.length - 1, 0, copy);
        return entries;
    }
}

menus.provider = new CopyTitle();
menus.start();
```

Put this in `~/.config/gnoblin/shell.ts`, or in a module you import. `defaultItems(client)` is the stock list, so a provider
can add to it. Right-click a titlebar and Copy title sits above Close. The entry runs `wl-copy`, so install it first.

## Draw your own menu

Every request ends in the provider's `show(client, x, y)`, with the anchor in global coordinates. Give a provider a `show`
method to draw something else in place of the library's card:

```ts
import { Box, MenuProvider, Text, WindowMenus } from "gnoblin-std";

class OwnMenu extends MenuProvider {
    show(id: string, x: number, y: number): void {
        const client = gnoblin.clients.get(id);
        if (client === undefined) return;
        const card = new Box({ x, y, width: 160, height: 36, radius: 8, fill: "#1e1e2e", secure: client.secure });
        card.add(new Text({ text: "Close", size: 14 }));
        card.onPointer = (event) => {
            if (event.kind === "press") {
                card.destroy();
                client.close();
            }
            return true;
        };
        card.mountTop();
    }
}

const menus = WindowMenus.get();
menus.provider = new OwnMenu();
menus.start();
```

A menu you draw yourself should take the keyboard, close on a press elsewhere, and be `secure` when the window is, as the
stock menu does. Without `WindowMenus.get().start()` the compositor still sends the requests and nothing answers them.

## Keyboard modes and the open card

The stock Move and Resize entries start a keyboard mode, where the arrow keys move or resize the window, Return keeps the place and Escape puts the window back. The entries are dim until a mode can start. Start the library's own modes with `Repositioning.get().start()`, or give the service a function of your own:

- `moveWithKeys` starts the keyboard move mode in place of the library's.
- `resizeWithKeys` starts the keyboard resize mode in place of the library's.

Example 32 (`examples/32-native-policy`, with `-ts` and `-py`) starts the library's window policy, including these modes.

`WindowMenus.current` is the open card. Listen to `currentChanged` to hear a card open, and to the card's `closed` signal to hear it close.

Example 29 (`examples/29-native-menus`, with `-ts` and `-py`) starts the menu with three entries of its own and logs what the card does.

## The not-responding alert

The compositor pings each client that has a window. A client that leaves a ping unanswered for longer than the
`check-alive-timeout` setting has its windows marked `unresponsive`. The setting is in the
[window management](/config/configure/window_management) section, 5000 ms by default. A timeout of `0` turns the pings off,
so no window is ever marked.

The stock shell looks at the windows once a second and opens an alert for each one that has stopped answering:

- **Wait** closes the alert. It does not come back for that window until the window has answered a ping and then stopped
  answering again. It is the default, and Escape chooses it.
- **Force quit** kills the window's process. It does nothing for the first half second after the alert opens, so a click or
  a key already on its way cannot press it. The compositor refuses its own process, init, and a process that is not the one
  that opened the window.

The alert takes the keyboard and gives it back when it closes. It is `secure` when the window is. It closes by itself when the
window answers again or goes, and no alert opens while the session is locked.

To turn it off, leave `NotRespondingAlerts.get().start()` out of your `shell.ts`. To draw your own alert, turn it off and read `client.unresponsive`
yourself. `client.kill()` is the call behind Force quit.

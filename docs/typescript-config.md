# defineConfig reference

`defineConfig` is the one call a TypeScript config makes. Each key below says what to put in it, what it
defaults to and what it does. To write your first config, start with the [TypeScript shell](typescript-shell.md).

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    gaps: { inner: 8, outer: 8 },
    workspaces: [{ id: "web", layout: "master-stack" }, { id: "dev" }],
    keys: { "Super+Return": spawn("foot") },
});
```

A key you leave out keeps its default. Your config is laid over the stock config rather than replacing it.

## How your config combines with the stock one

| Key                                      | What happens                                                                                                                    |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `workspaces`                             | Yours replaces the stock list. Leave it out, or empty, to keep the stock `1` to `4`.                                            |
| `rules`, `autostart`, `touchpadGestures` | Yours replaces the list as a whole.                                                                                             |
| `keys`                                   | Merged by trigger. Your key replaces the stock key with the same trigger.                                                       |
| `launcher`, `setup`                      | Yours, when you give one.                                                                                                       |
| `gaps`, `style`, `outputs`, `on`         | Merged by key, your value winning. A nested object, such as a shadow, is one value.                                             |
| `focus`                                  | Merged by key. A key you leave out is the compositor's own setting.                                                             |
| `decorations`                            | Not layered. The stock config has titlebars, for the windows that take the shell's frame. A `defineConfig` without it has none. |
| `menus`, `alerts`                        | Yours, when you give one. Both are on in the stock config.                                                                      |
| Compositor settings                      | Merged by key. See [compositor settings](#compositor-settings).                                                                 |

Triggers are compared in their canonical spelling, so `"super+1"` replaces the stock `"Super+1"`. A stock key that
shows a workspace by id is dropped when your config does not declare that workspace.

An `init.ts` that never calls `defineConfig` is a program, not a config. It runs as written and gets no stock keys.

## Windows and workspaces

### `workspaces`

A list of workspaces, each an object with these keys:

- `id`: a non-empty string, unique in the list.
- `name`: what the workspace is called where a name is shown. It defaults to the id.
- `layout`: a layout name or a layout object. It defaults to `floating`.

The first workspace is shown when the shell starts. Leave the key out and you get the stock four, `1` to `4`.

```ts
workspaces: [
    { id: "web", layout: "master-stack" },
    { id: "chat", layout: "grid" },
],
```

Each layout name arranges the windows of a workspace in its own way:

- `floating` leaves windows where they open and lets you move and resize them. `Super` and a left drag moves a window,
  and `Super` and a right drag resizes it.
- `master-stack` puts one main window on the left and stacks the others beside it. `{ id: "web", layout: "master-stack" }`.
- `grid` tiles the windows in a near-square grid. `{ id: "grid", layout: "grid" }`.
- `monocle` shows one window at a time, filling the workspace, and hides the rest. `{ id: "one", layout: "monocle" }`.
- `dwindle` splits the space of the focused window in two for each new window. `{ id: "tree", layout: "dwindle" }`.
- `kiosk` shows one window at a time like `monocle`. Use it for a workspace that runs a single app.
- `strip` puts windows in columns side by side on a strip that scrolls to the focused one. `Super+Left` and
  `Super+Right` move focus between columns.
- `carousel-3d` places the windows on a ring with the focused window at the front.

To set a layout's options, pass an object. The classes are `FloatingLayout`, `MasterStackLayout`, `GridLayout`,
`MonocleLayout`, `DwindleLayout`, `KioskLayout`, `StripLayout` and `Carousel3dLayout`, all exported by `gnoblin`.

```ts
import { defineConfig, MasterStackLayout } from "gnoblin";

export default defineConfig({
    workspaces: [{ id: "web", layout: new MasterStackLayout({ ratio: 0.6, orientation: "right" }) }],
});
```

These options take a closed set of values:

| Layout class        | Option        | Values and what they do                                                                                      |
| ------------------- | ------------- | ------------------------------------------------------------------------------------------------------------ |
| `FloatingLayout`    | `placement`   | `smart` opens a window where it overlaps nothing. `center` opens it centred. `cascade` steps down and right. |
| `MasterStackLayout` | `orientation` | `left` (the default), `right`, `top` or `bottom`: the edge the main window sits on.                          |
| `GridLayout`        | `fill`        | `row` fills a row before starting the next. `column` fills a column first.                                   |
| `DwindleLayout`     | `split`       | `aspect` splits across the longer side. `horizontal` puts windows side by side. `vertical` stacks them.      |

A tiling layout drags with the pointer:

- `MasterStackLayout`, `DwindleLayout` and `GridLayout` swap two windows with `Super+MouseLeft+Drag`.
- `MasterStackLayout` and `DwindleLayout` move a divider with `Super+MouseRight+Drag`.

The `triggers` option, `{ swap, resize }`, replaces either trigger, and `null` turns it off. See
[Input handlers](/shell-api/input-handlers#tiling-drags).

```ts
new MasterStackLayout({ triggers: { swap: null } });
```

### `gaps`

`{ inner, outer }`, each a number of pixels from 0 up. Both default to 0. `inner` is the space between windows and
`outer` is the space at the edge of the workspace.

```ts
gaps: { inner: 8, outer: 12 },
```

### `decorations`

A boolean. `true` gives the windows that take the shell's frame a titlebar above their content, with a title and
minimise, maximise and close buttons. `false` draws none. The stock config has `true`. A `defineConfig` that leaves the
key out has no titlebars.

A window that draws its own title bar (a GTK or Qt client, foot) gets no titlebar, so it never shows two.

- With `decorations` on, the shell tells the compositor that it draws decorations. A client that asks for server-side
  decorations with `xdg-decoration` is then told the server draws them.
- With `decorations` off, or with no shell, every client is told to draw its own.

See [the titlebar](js-ui.md#titlebar).

### `menus` and `alerts`

Two booleans for the shell's own windows. Both are `true` in the stock config, and a `defineConfig` that leaves one out
keeps it on.

- `menus` answers a window's menu requests with the stock window menu: a right click on a titlebar, the window menu key,
  and a client's own request. With `menus: false` those requests get no menu.
- `alerts` opens the not-responding alert, with Wait and Force quit, for a window that stops answering. With
  `alerts: false` the window is marked and nothing more.

```ts
menus: false,
alerts: true,
```

### `rules`

A list of window rules. Every rule that matches a window applies, in list order, and the later rule wins for each field
they both set; a rule with `first: true` ends the search where it matches. A rule applies again when the window's app
id, title or focus changes unless it says `reapply: false`. See [window rules](/config/window_rule) for every field.

```ts
rules: [
    { appId: "pavucontrol", floating: true },
    { title: "Picture in Picture", floating: true, skipFocus: true },
],
```

### `focus`

A table of focus settings. A key you leave out is the compositor's own value for that setting.

| Key                   | Values                              | Default   |
| --------------------- | ----------------------------------- | --------- |
| `mode`                | `click`, `sloppy` or `follow-mouse` | `click`   |
| `newWindows`          | `allow` or `prevent`                | `prevent` |
| `raiseOnClick`        | `true` or `false`                   | `true`    |
| `autoRaise`           | `true` or `false`                   | `false`   |
| `autoRaiseDelay`      | Milliseconds, 0 or more             | `500`     |
| `changeOnPointerRest` | `true` or `false`                   | `false`   |

The three modes differ in how a window gets focus:

- `click` focuses a window when you click it. `focus: { mode: "click" }`.
- `sloppy` focuses the window under the pointer as the pointer enters it, and keeps focus when the pointer moves onto
  the desktop. `focus: { mode: "sloppy" }`.
- `follow-mouse` is `sloppy`, and clears focus when the pointer leaves the focused window. `focus: { mode: "follow-mouse" }`.

`newWindows` decides whether a window started from a launcher takes focus. `allow` gives it focus. `prevent` leaves
focus where it was.

```ts
focus: { mode: "sloppy", autoRaise: true, autoRaiseDelay: 300 },
```

### `edgeTiling` and `dragConstraint`

`edgeTiling` is a boolean. When it is `true`, dropping a dragged window on a screen edge tiles it to that edge. Left
out, it follows the compositor's `window-management` setting.

`dragConstraint` is `work_area` or `minimum_strip`. Left out, it follows the compositor.

- `work_area` keeps a dragged window inside the work area. `dragConstraint: "work_area"`.
- `minimum_strip` lets a window leave the work area until only a strip of it, 75 pixels, stays visible.
  `dragConstraint: "minimum_strip"`.

```ts
edgeTiling: true,
dragConstraint: "minimum_strip",
```

### `style` and `outputs`

Both keys are accepted and checked, and neither changes how windows or screens look.

- `style` takes `cornerRadius`, `opacity` and `shadows`.
- `outputs` takes `{ mode, scale }` for each connector name, such as `"DP-1"`.

To set a window's opacity, use the `style` field of a [window rule](/config/window_rule).

## Keys

### `keys`

A table from a trigger to an action. Your keys are added to the stock keys.

```ts
keys: {
    "Super+Return": spawn("foot"),
    "Super+q": "close",
    "Super+b": spawn("firefox"),
    "Super+m": ({ gnoblin }) => gnoblin.spawn(["notify-send", "hello"]),
},
```

A trigger is modifiers and one key, joined by `+`: `Super+Return`, `Alt+Shift+Tab`, `XF86AudioMute`. The modifiers
are `Super`, `Alt`, `Ctrl`, `Shift`, `Meta` and `Hyper`. A letter key is the same trigger in either case, so write `Super+q`.

Triggers can also name the pointer and the touchpad:

- `Super+MouseLeft+Drag` is a drag with a held modifier. The buttons are `MouseLeft`, `MouseMiddle` and `MouseRight`.
- `WheelUp`, `WheelDown`, `WheelLeft` and `WheelRight` are scroll directions.
- `Swipe3` is a three-finger swipe, and `Swipe4Left` a four-finger swipe to the left. `Pinch2In` is a two-finger pinch
  in. A gesture takes 2 to 5 fingers and no modifiers.

An action is one of these:

| Action                                       | What it does                                                           |
| -------------------------------------------- | ---------------------------------------------------------------------- |
| `spawn("foot")`                              | Starts a program with no shell. Extra arguments are passed along.      |
| `spawn(["grim", "-g", "-"])`                 | The same, with the program and arguments as one list.                  |
| `show("web")`                                | Shows the workspace with that id.                                      |
| `show.numbered("Super+{n}", ["a", "b"])`     | Binds `Super+1` to `a` and `Super+2` to `b`. Spread it into `keys`.    |
| `show.position(3)`                           | Shows the third workspace, whatever it is called.                      |
| `show.positions("Super+{n}")`                | Binds `Super+1` to `Super+9` to the workspaces at those positions.     |
| `focusColumn("left")`                        | Moves focus one column on a `strip` workspace. Does nothing elsewhere. |
| `binding(() => new WorkspaceSlideBinding())` | Binds a gesture or drag binding at the trigger.                        |
| `"close"`                                    | Closes the focused window.                                             |
| `({ gnoblin }) => ...`                       | Runs your function when the key is pressed.                            |

`show` with an id the config does not declare fails the load, and names the key.

### Moving from `shortcuts` and `keybindings`

`keys` is the one way to bind a key. The `shortcuts` and `keybindings` sections of an older `init.lua` are not
`defineConfig` keys, and a config that uses either name is refused with a message that points here.

A legacy shortcut entry becomes one `keys` entry:

| Legacy `shortcuts` entry         | `keys` entry                                  |
| -------------------------------- | --------------------------------------------- |
| `binding = "<Super>Return"`      | The trigger `"Super+Return"`.                 |
| `binding = "<Super><Shift>q"`    | `"Super+Shift+q"`.                            |
| `binding = "<Primary>Tab"`       | `"Ctrl+Tab"`.                                 |
| `command = {"foot", "--server"}` | `spawn("foot", "--server")`.                  |
| `enable = false`                 | Leave the entry out.                          |
| `trigger = "release"`            | No equivalent. A key acts when it is pressed. |

```lua
shortcuts = {
    terminal = { binding = "<Super>Return", command = {"foot"} },
}
```

```ts
keys: { "Super+Return": spawn("foot") },
```

The name that keyed the entry (`terminal`) is not needed. A trigger may be bound once, so a duplicate fails at load.
A shortcut that must act on key release has no equivalent.

### `launcher`

A command line or an argv. When it is set, `Super+space` runs it. With no `launcher`, `Super+space` opens the stock launcher:
an entry on the screen that starts what you type. A `keys` entry for `Super+space` wins. A command line is split on spaces
and started with no shell.

```ts
launcher: "wofi --show drun",
```

## Starting programs

### `autostart`

A list of commands, each a command line or an argv. The session starts each command in a process group of its own, when
the config that lists it is first applied.

```ts
autostart: ["waybar", ["foot", "--server"], "nm-applet --indicator"],
```

- A command line is split on spaces and started with no shell. Use an argv, or `["sh", "-c", "..."]`, when you need quoting.
- A reload starts only the commands it adds. A command that is already running is not started again, and it keeps
  running when the shell restarts.
- A command is the same command when its program and arguments are the same.
- Removing an entry stops its process group at the next reload. Listing it again starts it again.
- A program that exits is not started again until you reload with it newly added.
- Each command is an entry of the config document's `autostart` section, named after its program (`foot`, then `foot-2`
  for a second `foot` command).

The same command listed twice fails the load.

## Listeners and `setup`

### `on`

Functions the shell calls when something happens to a window. Each is optional.

| Key             | Called with         | When                           |
| --------------- | ------------------- | ------------------------------ |
| `clientAdded`   | `client`            | A window appears.              |
| `clientRemoved` | `client`            | A window goes.                 |
| `titleChanged`  | `client`, `title`   | A window's title changes.      |
| `focusChanged`  | `client`, `focused` | A window gains or loses focus. |

```ts
on: {
    clientAdded: (client) => client.title === "Calculator" && (client.width = 320),
},
```

A listener that throws is logged once, and the others still run.

### `setup`

A function that runs once after everything else in the config is applied. It gets the whole `gnoblin` object, so it can
use anything the keys do not cover. It runs in the same time budget as the rest of the file, and a throw in it fails the
config like any other error.

```ts
setup(gnoblin) {
    gnoblin.dialogs.addListener(new NoScreenshots());
},
```

See [dialogs](#dialogs) for the listener this example adds.

## Compositor settings

These keys are sections of the compositor's configuration. Write them in camelCase: `windowManagement`, `buttonLayout`,
`reducedMotion`. Each section is the one of that name in the [configuration reference](/config/configure).

| Key                | Section                                                                |
| ------------------ | ---------------------------------------------------------------------- |
| `compositor`       | [`compositor`](/config/configure/compositor)                           |
| `windowManagement` | [`window-management`](/config/configure/window_management)             |
| `cursor`           | [`cursor`](/config/configure/cursor)                                   |
| `appearance`       | [`appearance`](/config/configure/appearance)                           |
| `dialogs`          | [`dialogs`](/config/configure/dialogs)                                 |
| `input`            | [`input`](/config/configure/input): keyboard, mouse, touchpad, tablets |
| `inputSources`     | [`input-sources`](/config/configure/input_sources)                     |
| `accessibility`    | [`accessibility`](/config/configure/accessibility)                     |
| `layerShell`       | [`layer-shell`](/config/configure/layer_shell)                         |
| `keyring`          | [`keyring`](/config/configure/keyring)                                 |
| `auth`             | [`auth`](/config/configure/auth)                                       |
| `location`         | [`location`](/config/configure/location)                               |
| `compat`           | `compat`                                                               |
| `capture`          | `capture`                                                              |
| `monitors`         | [`monitors`](/config/configure/monitors)                               |
| `xwayland`         | [`xwayland`](/config/configure/xwayland)                               |
| `logging`          | `logging`                                                              |
| `protocols`        | [`protocols`](/config/configure/protocols)                             |
| `touchpadGestures` | [`touchpad-gestures`](/config/configure/touchpad_gestures)             |

```ts
export default defineConfig({
    appearance: { colorScheme: "prefer-dark", reducedMotion: true },
    cursor: { size: 28 },
    input: { keyboard: { delay: 250, repeatInterval: 30 } },
});
```

Each setting takes effect the way its page says: immediately, when its process next starts, or not at all. A setting
that is accepted and not in effect is logged as `not applied: ...`.

Your settings are the whole document: the shell commits what `init.ts` describes and nothing else. A reload that drops a
key puts that setting back to its default, and a setting an earlier session committed does not come back.

A value the compositor refuses fails the reload and names the setting, for example `dialogs.style.radius`. The running
config and the document stay as they were.

### `protocols`

A table from a Wayland protocol name to `true` or `false`. `false` stops advertising the protocol to clients. A protocol you leave out stays on. The
change applies when the compositor next starts.

```ts
protocols: { wlrScreencopy: false, extIdleNotify: false },
```

| Key                            | Protocol                          | What it lets clients do                    |
| ------------------------------ | --------------------------------- | ------------------------------------------ |
| `wlrLayerShell`                | `wlr-layer-shell`                 | Draw panels, bars and backgrounds.         |
| `extForeignToplevelList`       | `ext-foreign-toplevel-list`       | List the open windows.                     |
| `wlrForeignToplevelManagement` | `wlr-foreign-toplevel-management` | List and control the open windows.         |
| `xdgDecoration`                | `xdg-decoration`                  | Negotiate who draws a window's titlebar.   |
| `kdeServerDecoration`          | `kde-server-decoration`           | Negotiate titlebars the older way.         |
| `wlrScreencopy`                | `wlr-screencopy`                  | Capture the screen.                        |
| `extImageCopyCapture`          | `ext-image-copy-capture`          | Capture windows and screens.               |
| `extBackgroundEffectV1`        | `ext-background-effect-v1`        | Ask for a blurred window background.       |
| `extDataControl`               | `ext-data-control`                | Read and set the clipboard.                |
| `extIdleNotify`                | `ext-idle-notify`                 | Be told when the user is idle.             |
| `wlrGammaControl`              | `wlr-gamma-control`               | Set screen gamma, as night-light tools do. |
| `wlrOutputPowerManagement`     | `wlr-output-power-management`     | Switch screens on and off.                 |
| `wlrOutputManagement`          | `wlr-output-management`           | Change screen modes and layout.            |
| `extSessionLock`               | `ext-session-lock`                | Lock the session.                          |

### `touchpadGestures`

A list of touchpad gestures that start a command when they complete. A gesture is a table:

| Field       | Values                                                                         | Default                           |
| ----------- | ------------------------------------------------------------------------------ | --------------------------------- |
| `name`      | 1 to 64 letters, digits, `_` or `-`; unique in the list                        | Required.                         |
| `gesture`   | `swipe` or `pinch`                                                             | Required.                         |
| `fingers`   | A whole number from 2 to 5                                                     | Required.                         |
| `path`      | A swipe: 2 to 16 points `{ x, y }` from -1 to 1, the first at `{ x: 0, y: 0 }` | Required for a swipe.             |
| `direction` | A pinch: `in` or `out`                                                         | Required for a pinch.             |
| `command`   | The program and its arguments                                                  | Required.                         |
| `threshold` | A swipe: 16 to 240. A pinch: 0.05 to 0.5                                       | 48 for a swipe, 0.12 for a pinch. |
| `tolerance` | How closely a swipe follows its path, 0.05 to 0.5                              | The compositor's.                 |

A `swipe` follows a path. This one is three fingers going right, then down:

```ts
touchpadGestures: [
    {
        name: "open-terminal",
        gesture: "swipe",
        fingers: 3,
        path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }],
        tolerance: 0.2,
        command: ["foot"],
    },
],
```

A `pinch` has a direction. `in` is fingers moving together and `out` is fingers moving apart:

```ts
touchpadGestures: [
    { name: "launcher", gesture: "pinch", fingers: 4, direction: "out", command: ["fuzzel"] },
],
```

Moving between workspaces with a gesture is the `Swipe3` key of the stock config, not a command.

### Sections that are not keys

Four sections of the compositor's configuration have no `defineConfig` key:

- `shortcuts` and `keybindings` are replaced by [`keys`](#keys).
- `animations` and `portals` are read by nothing, so there is nothing to set.

## Dialogs

The compositor draws its consent and passphrase dialogs. The `dialogs` key restyles them, and a listener can refuse a
request before its dialog opens.

```ts
dialogs: { style: { radius: 14, buttonLayout: "stretch" } },
```

| Setting        | Values                                                                                             | Default            |
| -------------- | -------------------------------------------------------------------------------------------------- | ------------------ |
| `radius`       | A whole number of points from 0 to 1000; a dialog is never rounder than 24                         | The built-in look. |
| `accent`       | `blue`, `teal`, `green`, `yellow`, `orange`, `red`, `pink`, `purple`, `slate`, `#rgb` or `#rrggbb` | The system accent. |
| `font`         | `{ family, size }`                                                                                 | The system font.   |
| `buttonLayout` | `start`, `end`, `stretch` or `stacked`                                                             | The built-in look. |
| `scale`        | A number from 0.1 to 100; a dialog is drawn at 1 to 3                                              | The built-in look. |

The button layouts place the dialog's buttons differently:

- `start` groups the buttons at the left.
- `end` groups them at the right.
- `stretch` makes them share the width of the dialog.
- `stacked` puts one above the other.

To refuse requests, add a listener in `setup`:

```ts
import { DialogListener } from "gnoblin";

class NoScreenshots extends DialogListener {
    onConsentRequest(request) {
        if (request.subject === "Screenshot") {
            request.decline("screenshots are off");
        }
    }
}

gnoblin.dialogs.addListener(new NoScreenshots());
```

A `DialogListener` has two optional methods: `onConsentRequest(request)` for a portal consent request and
`onSecretRequest(request)` for a passphrase request. A request has these members:

- `kind`, `consent` or `secret`.
- `appId`, the app id the compositor verified.
- `subject`, what is asked about.
- `deadline`, when the compositor takes the request back.
- `decline(reason)`, which refuses the request.

There is no way to allow a request and none to read a secret. A request is shown to the shell for 250 ms. A listener that
throws, runs too long or does nothing leaves the request to the built-in dialog.

## Errors

A wrong key name fails the load, and the message says what it found and what you probably meant. In an editor with the
[editor files](typescript-shell.md#editor-support) it is also a type error as you type.

```text
config has no "autostrt" (did you mean "autostart"?); expected one of gaps, style, outputs, workspaces, keys, ...
```

A key is suggested when it is within two edits of what you wrote. Case, `-` and `_` do not count as edits, so
`window-management` is read as `windowManagement`. A section from an older `init.lua` that has no key names what to write instead.

A value of the wrong type names the path and what it should be:

```text
config.workspaces must be a list of workspaces such as [{ id: "web" }], got string
config.autostart[1] must be a command line or an argv array of strings, got number
config.focus.mode must be click, sloppy or follow-mouse, got bogus
```

A setting the compositor refuses names the setting as the document spells it, such as `protocols.wlr-layer-shell`. A failed
load keeps the config that is running and logs one line, `file:line:column: message`.

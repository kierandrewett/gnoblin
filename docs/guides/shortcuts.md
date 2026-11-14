# Keyboard shortcuts

[Configuration reference](/config/configure)

Bind a key in the `keys` table of `defineConfig` in `~/.config/gnoblin/init.ts`. A key starts a program, runs a
shell action, or runs a function of yours. Your keys are added to the stock keys, and a save reloads the config. The
full list of actions is in the [defineConfig reference](/typescript-config#keys).

## Launch a command

This opens a terminal with Super+Enter and a launcher with Super+D:

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    keys: {
        "Super+Return": spawn("foot"),
        "Super+d": spawn("fuzzel"),
    },
});
```

Replace `foot` and `fuzzel` with installed programs. `spawn` takes the program and then each argument as a separate
string, or one list of them. Spaces inside a string stay in that argument.

![Fuzzel searching for Firefox on a clean Waybar desktop, with the pointer visible](../images/gnoblin-waybar-launcher.png)

_The shortcut opens Fuzzel; Firefox is the selected search result._

## Run a function

A function receives the shell and can do more than start a program:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    keys: {
        "Super+q": "close",
        "Super+m": ({ gnoblin }) => gnoblin.spawn(["notify-send", "Gnoblin", "hello"]),
    },
});
```

`"close"` closes the focused window. A function runs when the key is pressed. A key acts on press only: there is no
release trigger.

## Trigger syntax

A trigger is modifiers and one key, joined by `+`: `Super+Return`, `Alt+Shift+Tab`, `XF86AudioMute`.

| Trigger                  | Keys                                       |
| ------------------------ | ------------------------------------------ |
| `"Super+Return"`         | Super + Enter                              |
| `"Super+Shift+q"`        | Super + Shift + Q                          |
| `"Alt+F8"`               | Alt + F8                                   |
| `"XF86AudioMute"`        | The mute key, with no modifier             |
| `"Super+MouseLeft+Drag"` | A drag with Super held and the left button |
| `"WheelUp"`              | The scroll wheel up                        |
| `"Swipe3"`               | A three-finger swipe                       |

- The modifiers are `Super`, `Alt`, `Ctrl`, `Shift`, `Meta` and `Hyper`. Super is usually the Windows-logo key.
- The key is one XKB keysym name, such as `Return`, `F9`, `q` or `XF86AudioMute`. A letter key is the same trigger in
  either case, so write `Super+q`.
- Triggers are compared in their canonical spelling, so `"super+1"` replaces the stock `"Super+1"`.
- A trigger with a wrong shape fails the load, and the message names the key.

The legacy `<Super>Return` spelling is not a trigger. Write `Super+Return`.

## Chord syntax

The `keys` table uses the `+` triggers above. A few settings that go to the compositor take its own chord spelling,
which writes each modifier in angle brackets before the key: the tablet and stylus `keybinding` values and the
accelerators of the compositor socket.

| Chord               | Keys                                         |
| ------------------- | -------------------------------------------- |
| `"<Super>Return"`   | Super + Enter                                |
| `"<Super><Shift>q"` | Super + Shift + Q                            |
| `"<Alt>F8"`         | Alt + F8                                     |
| `"XF86AudioMute"`   | The mute key, with no modifier               |
| `"Super"`           | Super press and release, without another key |

- The modifiers are `<Shift>`, `<Control>`, `<Alt>`, `<Super>`, `<Hyper>` and `<Meta>`, in any case and any order, but
  not twice. The older GTK names `<Primary>`, `<Ctrl>` and `<Ctl>` read as `<Control>`, `<Shft>` as `<Shift>`, and
  `<Mod1>` as `<Alt>`. `<Mod2>` to `<Mod5>` depend on the keymap, so they are refused.
- The key is one XKB keysym name, such as `Return`, `F9`, `q` or `XF86AudioMute`. You can leave out the `XF86` prefix.
  `Above_Tab` is the key above Tab on any layout. `0x` and two hex digits, such as `0x3f`, is a hardware key code.
- `Super` alone, with no angle brackets, is a chord only where the page for that mechanism says so.
- A pointer chord ends with a button, `Button1` to `Button9`, for example `<Super>Button1`. It needs at least one
  modifier, except `Button8` and `Button9`.
- A chord has no spaces, and a chord with plus signs is refused with the form to write.

## Media keys

The stock config binds the volume and microphone mute keys through WirePlumber's `wpctl`, and playback through
`playerctl`. Install those commands if you use the matching keys. Brightness keys are not bound. Bind them to a
command such as `brightnessctl` if your hardware and permissions support it:

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    keys: {
        XF86MonBrightnessUp: spawn("brightnessctl", "set", "5%+"),
        XF86MonBrightnessDown: spawn("brightnessctl", "set", "5%-"),
        XF86AudioRaiseVolume: spawn("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"),
    },
});
```

The last line replaces the stock volume key, which raises it by 5%. Gnoblin handles shortcuts inside the compositor and
does not need systemd or the GNOME Settings Daemon.

## Avoid conflicts

- A trigger is bound once. Binding the same trigger twice in one `keys` table fails the load.
- A key of yours replaces the stock key with the same trigger. To drop a stock key, bind its trigger to something else.
- A gesture of yours replaces the stock binding of the same trigger.

See the [add a shortcut recipe](/recipes/add-a-shortcut) for a worked example.

## Commands and shell syntax

Commands run directly. `$HOME`, `~`, pipes and redirection are not expanded. Use an absolute path, a program on PATH,
or explicitly run a shell:

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    keys: {
        "Super+Shift+t": spawn("sh", "-c", 'date >> "$HOME/shortcut.log"'),
    },
});
```

This appends the current time to `~/shortcut.log` when you press Super+Shift+T.

## Not available

The JavaScript shell has no equivalent for these features of the Lua shell:

- A shortcut that acts when the key is released.
- Overriding a built-in Mutter keybinding by schema group and action name.
- A popup that captures the keys typed after a bare Super press.

See also [restore-or-minimise bindings](/guides/window_state_shortcuts).

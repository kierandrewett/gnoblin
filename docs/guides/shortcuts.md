# Keyboard shortcuts

[Configuration reference](/config/configure)

Bind a key with `gnoblin.input.primary.bind(trigger, function)` in `~/.config/gnoblin/shell.ts`. A key starts a program
or runs a function of yours, and a save reloads the file. The calls are in [keyboard shortcuts](/shell-api/keyboard-shortcuts).

## Launch a command

This opens a terminal with Super+Enter and a launcher with Super+D:

```ts
const input = gnoblin.input.primary;

input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
input.bind("Super+d", () => {
    gnoblin.spawn(["fuzzel"]);
});
```

Replace `foot` and `fuzzel` with installed programs. `gnoblin.spawn` takes the program and then each argument as a
separate string in one list. Spaces inside a string stay in that argument.

![Fuzzel searching for Firefox on a clean Waybar desktop, with the pointer visible](../images/gnoblin-waybar-launcher.png)

_The shortcut opens Fuzzel; Firefox is the selected search result._

## Run a function

A key is a function, so it can do more than start a program:

```ts
const input = gnoblin.input.primary;

input.bind("Super+q", () => {
    input.focusedClient?.close();
});
input.bind("Super+m", () => {
    gnoblin.spawn(["notify-send", "Gnoblin", "hello"]);
});
```

`input.focusedClient` is the window that has the focus, or `null`. A function runs when the key is pressed. A key acts
on press only: there is no release trigger.

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
- Triggers are compared in their canonical spelling, so `"super+1"` and `"Super+1"` are the same trigger.
- A trigger with a wrong shape fails the load, and the message names the key.

The legacy `<Super>Return` spelling is not a trigger. Write `Super+Return`.

## Chord syntax

`input.bind` uses the `+` triggers above. A few settings that go to the compositor take its own chord spelling,
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

The stock file binds the volume and microphone mute keys through WirePlumber's `wpctl`, and playback through
`playerctl`. Install those commands if you use the matching keys. Brightness keys are not bound. Bind them to a
command such as `brightnessctl` if your hardware and permissions support it:

```ts
const input = gnoblin.input.primary;

input.bind("XF86MonBrightnessUp", () => {
    gnoblin.spawn(["brightnessctl", "set", "5%+"]);
});
input.bind("XF86MonBrightnessDown", () => {
    gnoblin.spawn(["brightnessctl", "set", "5%-"]);
});
```

Gnoblin handles shortcuts inside the compositor and does not need systemd or the GNOME Settings Daemon.

## Avoid conflicts

- A trigger is bound once. Binding a trigger that is already bound is refused, and the load reports it.
- Your `shell.ts` replaces the stock file as a whole. The stock keys are lines of `gnoblinctl config init`'s starter, so to
  change one, edit that line in your copy: for a smaller volume step, change `5%+` to `2%+` on the
  `XF86AudioRaiseVolume` line.
- To drop a stock key, delete its line.

See the [add a shortcut recipe](/recipes/add-a-shortcut) for a worked example.

## Commands and shell syntax

Commands run directly. `$HOME`, `~`, pipes and redirection are not expanded. Use an absolute path, a program on PATH,
or explicitly run a shell:

```ts
gnoblin.input.primary.bind("Super+Shift+t", () => {
    gnoblin.spawn(["sh", "-c", 'date >> "$HOME/shortcut.log"']);
});
```

This appends the current time to `~/shortcut.log` when you press Super+Shift+T.

## Not available

The shell has no equivalent for these features of the retired Lua shell:

- A shortcut that acts when the key is released.
- Overriding a built-in Mutter keybinding by schema group and action name.
- A popup that captures the keys typed after a bare Super press.

See also [restore-or-minimise bindings](/guides/window_state_shortcuts).

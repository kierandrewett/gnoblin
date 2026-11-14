# Autostart settings

`autostart` is a list of commands the shell starts in the user session. Each entry is a command line or an argv:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    autostart: ["waybar", ["waybar", "--config", "/home/you/.config/waybar/config"], "nm-applet --indicator"],
});
```

| Entry form     | Meaning                                                                                            |
| -------------- | -------------------------------------------------------------------------------------------------- |
| A string       | A command line, split on spaces and started with no shell.                                         |
| A list         | The program and its arguments, one item each. Use it when an argument contains a space or a quote. |
| `["sh", "-c"]` | A shell you start yourself, when you need pipes, redirection or expansion.                         |

The list replaces the stock list as a whole, and the stock config has no autostart entries. The same command listed
twice fails the load. Each command is an entry of the config document's `autostart` section, named after its program
(`foot`, then `foot-2` for a second `foot` command).

![Firefox below Waybar in a Gnoblin session](../../images/gnoblin-waybar-firefox.png)

_Waybar starts as an independent session client; Firefox is a regular application window._

## How a reload treats the list

- A reload starts only the commands it adds. A command that is already running is not started again, and it keeps
  running when the shell restarts.
- A command is the same command when its program and arguments are the same.
- Removing an entry stops its process group at the next reload, including descendants that remain in that group.
  Listing it again starts it again.
- A program that exits is not started again until you reload with it newly added.
- A command that cannot be started logs a warning and does not stop the others.

See the [autostart guide](/guides/autostart) for launch timing and examples.

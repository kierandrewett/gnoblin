# Autostart

[Configuration reference](/config/configure/autostart)

Use `autostart` for commands that should start in your session. Add this to `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    autostart: ["waybar"],
});
```

Install the program first. Gnoblin runs the command directly, without shell expansion. A string is split on spaces; use
a list of strings, one for each argument, when an argument has a space or a quote. If you need pipes or redirection,
explicitly run a shell.

Gnoblin starts each command once per session and does not restart it after it exits.

A new command starts after a successful config reload if you are already logged in; otherwise it starts at the next
login. A command that is already running is left alone, and it keeps running when the shell restarts.

Gnoblin starts configured commands after the Wayland display is published.

- It does not read XDG autostart `.desktop` files. Add a program here instead.
- It refuses an entry that starts gnome-keyring while `keyring.enabled` is on. That daemon would take
  `org.freedesktop.secrets` from Gnoblin's keyring. See
  [Apps reach gnome-keyring instead](/config/configure/keyring#apps-reach-gnome-keyring-instead).

When using Bingux, load its Gnoblin module to start its shell and layer clients through this same configuration path.

## Change a command

A command is the same command when its program and arguments are the same. Change an argument and you have a new
command: the old process group stops at the next reload and the new command starts.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    autostart: [["waybar", "--config", "/home/you/.config/waybar/work.jsonc"]],
});
```

Replace the path with your own.

## Stop starting a program

Remove the entry. The `autostart` list of your config is the whole list, so a program that is not in it is not started
in future sessions.

A reload also stops a running command you removed. Gnoblin sends `SIGTERM` to the process group it started, and
`SIGKILL` if the group has not exited after 200 ms. Descendants that remain in the group are included.

## When does it run?

- A command you add starts after Gnoblin accepts a config reload.
- Each command gets one launch per login. Saving again or unlocking does not start a second copy.
- Changing a command stops the owned process group and starts the updated command.
- Removing a command stops the owned process group.

See [command syntax](/guides/shortcuts#commands-and-shell-syntax) for shell commands and
[files and load order](/guides/files_and_load_order) for splitting a config over files.

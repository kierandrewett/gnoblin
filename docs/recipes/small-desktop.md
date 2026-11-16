# Build a small desktop

This setup binds a launcher and a terminal, and leaves the shell itself to your choice: a bar and notifications are
separate programs that you start with the session. It gives you two ways to open applications.

![Firefox beneath Waybar in a clean Gnoblin session](../images/gnoblin-waybar-firefox.png)

_Firefox is an ordinary window; Waybar runs as a separate desktop client._

Add this to `~/.config/gnoblin/shell.ts`. Replace any command with an application installed on your system.

```ts
const input = gnoblin.input.primary;

input.bind("Super+d", () => {
    gnoblin.spawn(["fuzzel"]);
});
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

Install the selected applications first. Start Waybar and Mako with the session, because `shell.ts` has no autostart list:
see [autostart](/guides/autostart). `shell.ts` cannot set the cursor theme either: see
[cursor themes](/guides/cursors).

A trigger is bound once, so if the starter you wrote with `gnoblinctl config init` already binds one of these, edit that
line instead of adding another. See [shell setup](/bring-your-own-shell) for choosing the programs that provide the
visible desktop.

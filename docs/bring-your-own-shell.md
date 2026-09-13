# Bring your own shell

Gnoblin deliberately stops after the compositor, session services and window
management. It does not draw a top bar, dock, launcher or notification centre.
This keeps the session small and lets you choose the desktop chrome that fits
your workflow.

## Bingux

Bingux is the reference shell for Gnoblin. Build and install it from its
[standalone guide](https://github.com/kierandrewett/bingux/blob/main/docs/standalone.md).
For a personal source install, the recommended command is:

```sh
make install-user
```

This keeps the Bingux payload under one managed user directory and starts its
user target. Package installations can use their package manager instead.
To enable an already-installed package manually:

```sh
systemctl --user daemon-reload
systemctl --user enable --now bingux.target
```

The target starts the shell, search and status services. Bingux uses Gnoblin's
layer-shell and D-Bus interfaces for placement, window state, OSD ownership and
notifications.

## Waybar or another shell

Any shell that supports `zwlr_layer_shell_v1` can draw surfaces in a Gnoblin
session. Start it from the session's user systemd manager or an autostart entry
so it appears after login. A minimal Waybar configuration can start as soon as
the session is ready:

```sh
waybar
```

The external shell owns OSDs, capture controls and workspace feedback.
Gnoblin retains the desktop right-click menu for recovery and forwards
OSD events through `org.gnoblin.Shell.OsdRequested` automatically. GNOME
extensions cannot load in this session.

Notifications and the native keyboard-layout popup are disabled by default.
Use `gnoblinctl feature list` to inspect the remaining controls. Keyboard-layout
state and switching remain available through `ListInputSources` and
`InputSourceChanged` while the native popup is disabled.

## A custom shell

Use the [control protocol](configuration.md) for session state and the
implemented layer-shell protocols for visible surfaces. The protocol gates in
`init.lua` are enabled by default in the Gnoblin session and can be
disabled before login when a client does not need them.

Start with these checks:

```sh
gnoblinctl ping
gnoblinctl version
gnoblinctl feature list
```

If no visible layer surface appears for eight seconds, Gnoblin shows a native
Desktop unavailable panel with Terminal, Console, Settings, Config folder and Close. The panel
also appears if all visible layer surfaces disappear. It closes automatically
when a layer surface appears. Close keeps it hidden until a layer surface
has appeared and subsequently disappeared, the session unlocks, or a new
session begins.

Right-click the desktop for Open Terminal and Settings at any time. These
recovery tools do not depend on Bingux or another external shell. They are
disabled while the session is locked. The panel detects absent surfaces; it
cannot detect a frozen client that still has a mapped surface.

For a separate text console, the standard shortcut is Ctrl+Alt+F3. If TTY
shortcuts were disabled in user settings, restore one with:

```sh
gsettings reset org.gnome.mutter.wayland.keybindings switch-to-session-3
```

From the terminal, inspect the external shell's service log and start it again
once the startup failure is fixed.

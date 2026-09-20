# Try Gnoblin in a window

The devkit runs a nested Gnoblin session inside your Wayland desktop.
Use it to test a build without logging out.

First complete the [source build](install-source.md).

## Start

From the checkout:

```sh
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit
```

A desktop viewer and terminal open. Programs started from that terminal connect
to the nested compositor.

To choose a terminal explicitly:

```sh
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit kitty
```

## Try your shell

From the devkit terminal, launch an installed layer-shell client:

```sh
waybar
```

The bar should appear inside the viewer. Try its menus and launcher.
Close the terminal to stop the devkit.

## Options

| Variable                   | Default    | Purpose                                      |
| -------------------------- | ---------- | -------------------------------------------- |
| `MONITOR`                  | `1600x900` | Virtual display size                         |
| `GNOME_DEVKIT_HEADLESS`    | Unset      | Set `1` to hide the viewer                   |
| `GNOME_DEVKIT_EXEC`        | Unset      | Command to run instead of a terminal         |
| `GNOME_DEVKIT_UNSAFE_MODE` | Unset      | Enable privileged Eval for this test process |

## Headless / scripting mode

```sh
GNOME_DEVKIT_HEADLESS=1 \
GNOME_DEVKIT_EXEC='gnoblinctl feature list --json' \
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit
```

This runs without a host Wayland display and exits after the command.
`just gnome-devkit-verify` checks this environment.

## Isolation

The nested display and D-Bus bus are separate. Host X11, accessibility and gvfs
connections are not passed through.

**Your HOME and runtime directory remain real.** Applications can still read
or change your files and configuration. This is not a security sandbox.

For disposable settings, use the [private test harness](testing.md).
A devkit run does not verify the installed login session.

## Troubleshooting

| Problem                    | Next step                                                |
| -------------------------- | -------------------------------------------------------- |
| No host `WAYLAND_DISPLAY`  | Use a Wayland desktop or headless mode                   |
| No Shell in `./install`    | Finish the source build                                  |
| No terminal found          | Install one or pass its command explicitly               |
| Quickshell/Qt mismatch     | Install or rebuild a matching Quickshell                 |
| `EBUSY` taking the session | Use this devkit launcher, not a direct native/KMS launch |

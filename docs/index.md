# Gnoblin

Gnoblin is a standalone Wayland compositor session built on Mutter. Its Lua
configuration and runtime API control compositor state; independent shell
projects such as Bingux provide the panels, launchers, and other desktop UI.
Build your own shell or try Bingux, a separate project that uses Gnoblin.

![GNOME Files open above the Bingux dock](images/gnoblin-bingux-files.png)

_Bingux is one separate shell project that uses Gnoblin. Its packaged rules round the Files window, fill its CSD corner gaps and draw the window shadow._

![GNOME Files beneath Waybar and a Quickshell dock](images/gnoblin-waybar-quickshell-dock.png)

_Waybar provides the top bar; [Quickshell](/bring-your-own-shell#quickshell) provides the dock._

The [configuration API](/config) links each setting to a detailed reference.

## Get started

- [Install Gnoblin](/installation)
- [Choose a shell](/bring-your-own-shell)
- [Build a desktop](/build-a-desktop)

## Configure

- [Configuration API](/config)
- [Configuration recipes](/recipes/)
- [Configuration guides](/guides/window_rules)

## Develop

- [Shell integration](/shell-integration)
- [Compositor bridge](/compositor-bridge)
- [Wayland protocols](/wayland-protocols)

## Help

- [Troubleshooting](/troubleshooting)

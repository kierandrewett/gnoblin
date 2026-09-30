# Gnoblin

## Register

product

## Users and purpose

Gnoblin is a compositor and session runtime for people who use an external
desktop shell. It owns window management, input policy, and session services.
Bingux and other Wayland clients provide panels, launchers, notifications, and
other desktop controls. See README.md and docs/bring-your-own-shell.md.

The standalone session has no built-in developer console or GNOME Shell/GJS
runtime. Users configure the session with Lua. Developers inspect and control a
running session with `gnoblinctl` and the documented runtime API.

## Design principles

- Keep shell presentation in external Wayland clients.
- Keep configuration and runtime control in the shared Lua API.
- Keep terminal input in terminal clients; it is not compositor JavaScript.
- Report operation errors and asynchronous completion through the runtime API.

## References and boundaries

Use the [configuration reference](/config) for Lua settings and the
[runtime API reference](/config/runtime-api) for compositor state and
operations. Use [gnoblinctl](/gnoblinctl) for session diagnostics and direct
control. The compositor API does not create widgets or provide a terminal UI.

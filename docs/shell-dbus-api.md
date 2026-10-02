# Legacy shell D-Bus API

The `org.gnoblin.Shell` D-Bus service belonged to the former GNOME Shell
integration and is not provided by the standalone Gnoblin session.

Use [the compositor bridge](/compositor-bridge) for shell clients,
[`gnoblinctl`](/gnoblinctl) for command-line operations, and the
[Lua runtime API](/config/runtime-api) for session configuration and scripts.

# gnoblin.configure.portals

Choose which installed XDG Desktop Portal backend handles each portal
interface in a Gnoblin session.

```lua
gnoblin.configure {
    portals = {
        default = {"gnoblin", "*"},
        interfaces = {
            ["org.freedesktop.impl.portal.ScreenCast"] = {"gtk", "gnoblin", "*"},
        },
    },
}
```

The backend lists are ordered. For each request, the portal service tries the
first installed backend in the list that implements that interface.

`default` applies to interfaces not listed under `interfaces`.

Common backend IDs include `gtk`, `gnome`, `kde`, and `gnoblin`. Use IDs
provided by the backends installed on your system.

Each interface entry overrides `default` for that interface. The interface
name is the D-Bus name supported by the portal service. Gnoblin preserves it
exactly, including underscores.

`default` is required when `portals` is set. Each backend list must contain
between 1 and 32 strings. Backend IDs use ASCII letters, digits, underscores,
or hyphens, and start with a letter or digit.

The special backend value `"*"` selects the first installed implementation in
name order. It can appear in a list with named backends. The value `"none"`
disables that portal interface and must be used by itself.

The Gnoblin session installation provides a desktop-specific default that
prefers `gnoblin`, then tries any installed backend. The optional
`gnoblin-portal` package supplies that backend. The portal service selects the
Gnoblin file only in a Gnoblin session; other desktop sessions keep their own
portal configuration.

Set `portals` when you want a per-user route. For example, prefer GTK for every
interface and keep Gnoblin first for ScreenCast:

```lua
gnoblin.configure {
    portals = {
        default = {"gtk", "gnoblin", "*"},
        interfaces = {
            ["org.freedesktop.impl.portal.ScreenCast"] = {"gnoblin", "gtk", "*"},
        },
    },
}
```

Set portal routes in your Gnoblin Lua config,
`~/.config/gnoblin/init.lua`. This is the only file you edit for Gnoblin
settings.

The portal service reads standard `portals.conf` files. When you set `portals`,
Gnoblin writes a generated override at
`$XDG_CONFIG_HOME/xdg-desktop-portal/gnoblin-portals.conf` (normally
`~/.config/xdg-desktop-portal/gnoblin-portals.conf`). Do not edit that file.
Your Lua config remains the only file you maintain for Gnoblin settings. See
the [XDG portal configuration
reference](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)
for the portal service's configuration interface.

When `portals` is set, Gnoblin writes the generated override before starting
session services. A configuration reload updates the override, but an already
running portal service keeps its current routing until it starts again.
Gnoblin does not restart it automatically because that can interrupt active
portal requests, including screen sharing.

To apply a reload immediately, restart the portal service when no portal
request is active:

```sh
systemctl --user restart xdg-desktop-portal.service
```

If you omit `portals`, the default installed for the Gnoblin session applies.
It uses Gnoblin's backend when installed and otherwise selects another
installed backend. Other desktops continue using their own defaults.

An existing per-user `gnoblin-portals.conf` or generic `portals.conf` takes
precedence. Gnoblin leaves it alone and reports a conflict rather than
overwriting it. If a generated Gnoblin file exists when a generic `portals.conf`
is added, Gnoblin removes its generated file so the user's configuration can
take effect.

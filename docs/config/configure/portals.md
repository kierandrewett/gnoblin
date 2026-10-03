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

Gnoblin's seeded Lua config prefers the optional `gnoblin` backend, then uses
`"*"` as a fallback. To prefer another portal for one interface, list that
backend first under `interfaces`, as in the ScreenCast example. To change the
preference for every interface, move its backend ID before `"gnoblin"` in
`default`.

Set portal routes in your Gnoblin Lua config,
`~/.config/gnoblin/init.lua`. This is the only file you edit for Gnoblin
settings.

The portal service accepts routes through its own configuration format, so
Gnoblin translates this setting into a generated adapter automatically. You do
not need to create or edit a portal configuration file. See the [XDG portal
configuration
reference](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)
for how the service resolves backend preferences.

Gnoblin writes portal preferences before starting session services. A
configuration reload updates the generated preference, but an already running
portal service keeps its current routing until it starts again. Gnoblin does
not restart it automatically because that can interrupt active portal
requests, including screen sharing.

To apply a reload immediately, restart the portal service when no portal
request is active:

```sh
systemctl --user restart xdg-desktop-portal.service
```

The `gnoblin-portal` package installs only the backend implementation. Your
Lua setting chooses it when wanted; if you omit `portals`, xdg-desktop-portal
uses the system's desktop-specific default.

An existing user portal preference takes precedence. Gnoblin leaves it alone
and reports a conflict rather than overwriting it.

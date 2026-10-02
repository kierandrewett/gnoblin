# gnoblin.configure.portals

Choose which installed XDG Desktop Portal backend handles each portal
interface in a Gnoblin session.

```lua
gnoblin.configure {
    portals = {
        default = {"gtk"},
        interfaces = {
            ["org.freedesktop.impl.portal.ScreenCast"] = {"gnoblin", "gtk"},
            ["org.freedesktop.impl.portal.RemoteDesktop"] = {"gnoblin", "gtk"},
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

Gnoblin's optional `gnoblin-portal` package supplies the `gnoblin` backend. If
you omit the `portals` section, the portal service uses the system's
desktop-specific default.

The portal service does not read Lua. It reads a desktop-specific INI file
based on `XDG_CURRENT_DESKTOP`. In a Gnoblin session, that file is named
`gnoblin-portals.conf`. Gnoblin generates it from this Lua section, so you do
not need to create or maintain it. See the [XDG portal configuration
reference](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)
for how the portal service resolves backend preferences.

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

If you created `~/.config/xdg-desktop-portal/gnoblin-portals.conf` using older
Gnoblin instructions, move or remove that file once to let the Lua setting take
effect. User-owned portal preferences are preserved.

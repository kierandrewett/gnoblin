# gnoblin.configure.location

Set the global location policy used by Gnoblin's GeoClue agent:

```lua
gnoblin.configure {
    location = {
        enabled = true,
        max_accuracy = "city",
    },
}
```

This policy controls whether Gnoblin can approve location requests and the
maximum precision it can return. It does not approve individual applications.
A shell or Lua handler must still answer each
[`gnoblin.location.authorization-requested`](/config/runtime-api#privacy-and-permissions)
event.

| Field          | Type and accepted values                                                       | Default                                             | Effect                                  |
| -------------- | ------------------------------------------------------------------------------ | --------------------------------------------------- | --------------------------------------- |
| `enabled`      | Boolean or `"inherit"`                                                         | Current system setting; schema default is `false`   | Enables requests or denies them all.    |
| `max_accuracy` | `"country"`, `"city"`, `"neighborhood"`, `"street"`, `"exact"`, or `"inherit"` | Current system setting; schema default is `"exact"` | Caps precision for an approved request. |

The fallback keys and defaults come from the
[`org.gnome.system.location` schema](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.system.location.gschema.xml.in).
System or administrator values take precedence over schema defaults.

Each field overrides its corresponding `org.gnome.system.location` setting
independently. If a field is omitted or set to `"inherit"`, Gnoblin uses that
setting. This lets you, for example, enable location in Lua while keeping the
system's configured accuracy cap.

Changes apply when Gnoblin reloads the configuration. Setting `enabled =
false` denies location requests and sets the maximum accuracy to zero. An
approved request is also limited by the application's requested accuracy.

Choose the least precise level an application needs:

- `"country"`: country-level location.
- `"city"`: city-level location.
- `"neighborhood"`: neighborhood-level location.
- `"street"`: street-level location.
- `"exact"`: exact location, typically requiring a GPS receiver.
- `"inherit"`: clear an earlier Lua override and follow the system setting.

## When GeoClue asks your handler

GeoClue asks the agent about an app only when it can identify the app as a
sandboxed Flatpak app. It reads that identity from the app's systemd scope
(`app-flatpak-ID-N.scope`). Every other app counts as a system component:
GeoClue does not ask, and the app receives location up to `max_accuracy`.
`enabled` and `max_accuracy` apply to every app, with or without a request.

## Stop the GeoClue demo agent

GeoClue accepts one agent for each user. On Fedora the `geoclue-demo-agent`
autostart entry hides itself only in GNOME, so it also starts in a Gnoblin
session. It approves every request, and it can take the agent slot before
Gnoblin registers.

To stop it, create `~/.config/autostart/geoclue-demo-agent.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Geoclue Demo agent
Exec=/usr/libexec/geoclue-2.0/demos/agent
Hidden=true
```

Run `systemctl --user daemon-reload`, then log out and in. A running demo agent
keeps running until then. To check that GeoClue accepted Gnoblin, stop the
service with `sudo systemctl stop geoclue`, start it by hand with
`sudo -u geoclue env G_MESSAGES_DEBUG=Geoclue /usr/libexec/geoclue`, and look
for `New agent for user ID` in its output.

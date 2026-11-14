# Location settings

The `location` key sets the global location policy used by Gnoblin's GeoClue agent:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    location: {
        enabled: true,
        maxAccuracy: "city",
    },
});
```

This policy controls whether Gnoblin can approve location requests and the maximum precision it can return. It does
not approve individual applications. GeoClue asks about an app only when it can identify it, as
[below](#when-geoclue-asks-the-agent).

| Field         | Type and accepted values                                                       | Default                                             | Effect                                  |
| ------------- | ------------------------------------------------------------------------------ | --------------------------------------------------- | --------------------------------------- |
| `enabled`     | Boolean or `"inherit"`                                                         | Current system setting; schema default is `false`   | Enables requests or denies them all.    |
| `maxAccuracy` | `"country"`, `"city"`, `"neighborhood"`, `"street"`, `"exact"`, or `"inherit"` | Current system setting; schema default is `"exact"` | Caps precision for an approved request. |

The fallback keys and defaults come from the
[`org.gnome.system.location` schema](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.system.location.gschema.xml.in).
System or administrator values take precedence over schema defaults.

Each field overrides its corresponding `org.gnome.system.location` setting independently. If a field is omitted or set
to `"inherit"`, Gnoblin uses that setting. This lets you, for example, enable location in your config while keeping the
system's configured accuracy cap.

The GeoClue agent reads the section when it starts, so a change takes effect at the next session start. Setting
`enabled: false` denies location requests and sets the maximum accuracy to zero. An approved request is also limited
by the application's requested accuracy.

Choose the least precise level an application needs:

- `"country"`: country-level location.
- `"city"`: city-level location.
- `"neighborhood"`: neighborhood-level location.
- `"street"`: street-level location.
- `"exact"`: exact location, typically requiring a GPS receiver.
- `"inherit"`: follow the system setting.

## When GeoClue asks the agent

GeoClue asks the agent about an app only when it can identify the app as a sandboxed Flatpak app. It reads that
identity from the app's systemd scope (`app-flatpak-ID-N.scope`). Every other app counts as a system component:
GeoClue does not ask, and the app receives location up to `maxAccuracy`. `enabled` and `maxAccuracy` apply to every
app, with or without a request.

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

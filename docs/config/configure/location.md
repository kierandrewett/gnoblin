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

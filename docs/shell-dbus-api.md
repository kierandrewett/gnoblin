# Shell D-Bus API

`org.gnoblin.Shell` is Gnoblin's session-bus interface for shell status,
input-source selection, feature switches and permission state. The service
owns `/org/gnoblin/Shell` while the shell component is running.

Use [`gnoblinctl`](/gnoblinctl) for command-line and script use. Shells that
need live window state or temporary input grabs should use the
[compositor bridge](/compositor-bridge).

```sh
gdbus introspect --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell

gdbus call --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell \
    --method org.gnoblin.Shell.Ping
```

`Ping` returns `('pong',)`. `GetVersion` returns the running shell version
string, such as `51.0-gnoblin`. Use `gnoblinctl version` for the installed
Gnoblin package version.

## Methods

Signatures use D-Bus type notation. Methods with no output return an empty
tuple.

| Method                  | Input → output    | Purpose                                                                                  |
| ----------------------- | ----------------- | ---------------------------------------------------------------------------------------- |
| `Ping`                  | `() → s`          | Liveness check; returns `pong`.                                                          |
| `GetVersion`            | `() → s`          | Running shell version string, such as `51.0-gnoblin`.                                    |
| `Reload`                | `() → ()`         | Soft-reload theme and scripts while keeping application windows.                         |
| `ReloadConfig`          | `() → ()`         | Reload the active Lua configuration.                                                     |
| `ListInputSources`      | `() → a(ssss)`    | List input sources as `(type, id, shortName, displayName)`.                              |
| `GetCurrentInputSource` | `() → (ssss)`     | Return the current input source in the same field order.                                 |
| `SetInputSource`        | `(ss) → ()`       | Activate the source identified by `(type, id)` from the list.                            |
| `GetPrivacyState`       | `() → (bbb)`      | Read screen sharing, microphone use and location use.                                    |
| `ListScripts`           | `() → as`         | List successfully loaded script names.                                                   |
| `GetPermissions`        | `() → s`          | Return a JSON string containing the active policy, capabilities, levels and config path. |
| `CheckPermission`       | `(ss) → (ssasub)` | Evaluate `(capability, identity)`; return `(level, rule, monitors, devices, clipboard)`. |
| `ListPortalGrants`      | `() → a(sssubb)`  | List grants as `(id, portal, requester, devices, clipboard, screenStreams)`.             |
| `RevokePortalGrant`     | `(ss) → ()`       | Revoke a grant by `(portal, id)`. Portal is `screen-cast` or `remote-desktop`.           |
| `ListFeatures`          | `() → a(ssb)`     | List feature `(id, description, enabled)` records.                                       |
| `GetFeature`            | `(s) → b`         | Read a feature state; an unknown ID returns `false`.                                     |
| `SetFeature`            | `(sb) → ()`       | Persist and apply a supported feature switch.                                            |

Errors:

- `Reload` and `ReloadConfig` report failures as D-Bus errors.
- `CheckPermission` reports invalid requests as
  `org.gnoblin.Shell.Error.PermissionPolicy`.
- `SetFeature` reports unknown IDs and attempts to enable removed native UI
  features as errors. Feature IDs are runtime data; discover them with
  `ListFeatures` or `gnoblinctl feature list`.

For permission identities and policy fields, see the
[permission guide](/guides/permissions). Do not treat a portal grant as a
permission-policy rule: grants represent user-approved portal sessions.

## Properties

| Property      | Type | Meaning                                                              |
| ------------- | ---- | -------------------------------------------------------------------- |
| `IsWayland`   | `b`  | Whether this shell is running as a Wayland compositor.               |
| `SessionMode` | `s`  | Current GNOME Shell session mode; Gnoblin sessions report `gnoblin`. |

Read a property with `gdbus`:

```sh
gdbus call --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell \
    --method org.freedesktop.DBus.Properties.Get \
    org.gnoblin.Shell IsWayland
```

## Signals

Listen with `gdbus monitor --session --dest org.gnoblin.Shell`. Signals have no
replay: connect first, then read the corresponding method for current state.

| Signal                | Fields                   | Meaning                                                                                                                        |
| --------------------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `SuperReleased`       | `(u, t)`                 | Super was released without another input. Includes protocol version `1` and a monotonic timestamp in microseconds.             |
| `OsdRequested`        | `(u, i, s, s, d, d, as)` | OSD protocol version `2`, monitor index, icon, label, level, maximum level and physical output names for that logical monitor. |
| `InputSourceChanged`  | `(s, s, s, s)`           | Current input source changed: `(type, id, shortName, displayName)`.                                                            |
| `InputSourcesChanged` | `b`                      | The value is `true`; refresh the source list with `ListInputSources`.                                                          |
| `PrivacyStateChanged` | `(b, b, b)`              | Screen sharing, microphone use and location use changed.                                                                       |
| `FeatureChanged`      | `(s, b)`                 | A feature ID and its new enabled state.                                                                                        |

`InputSourcesChanged` is followed by `InputSourceChanged` for the current
source. The initial privacy and feature states are not emitted as a snapshot;
call `GetPrivacyState` and `ListFeatures` when connecting. The OSD signal
replaces the removed in-process OSD UI; an external shell decides how to
display it.

For the versioned JSON socket used by window lists, actions, previews and
temporary grabs, see the [compositor bridge](/compositor-bridge). For launch
feedback tokens, see the [launch feedback interface](/launch-feedback).

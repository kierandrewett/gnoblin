# Session Activity Interface

Use `org.gnoblin.SessionIdle` to read idle state and receive activity changes.

The interface is exported on the session bus at `/org/gnoblin/SessionIdle` by the service name `org.freedesktop.ScreenSaver`. It reports the idle state that Gnoblin derives from Mutter's idle monitor.

| Member            | Arguments                                                                            | Result                                                                               | Errors                                                               |
| ----------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| `GetActivity`     | None                                                                                 | `available: boolean`, `idle: boolean`, `threshold_ms: uint64`, `idle_for_ms: uint64` | Returns a D-Bus error if Gnoblin cannot read the current idle state. |
| `ActivityChanged` | `available: boolean`, `idle: boolean`, `threshold_ms: uint64`, `idle_for_ms: uint64` | Signal                                                                               | Emitted when availability or idle state changes.                     |

The current threshold is 120000 ms. When `available` is false, `idle` is false and `idle_for_ms` is zero. The service emits no signal for changes only to the elapsed idle duration.

## Read activity

```sh
busctl --user call org.freedesktop.ScreenSaver /org/gnoblin/SessionIdle org.gnoblin.SessionIdle GetActivity
```

The output contains the four fields in the order shown above. Use `gdbus monitor --session --dest org.freedesktop.ScreenSaver --object-path /org/gnoblin/SessionIdle` to watch `ActivityChanged` while activity state changes.

# Screen Saver Interface

Use `org.freedesktop.ScreenSaver` to inhibit idle, inspect lock state, or request a lock.

Gnoblin exports this interface on the session bus as `org.freedesktop.ScreenSaver`, at `/org/freedesktop/ScreenSaver` and at `/ScreenSaver`. The service answers from the compositor's idle and lock state. It does not forward calls to another screen saver.

The interface follows the [freedesktop idle-inhibit specification](https://whot.pages.freedesktop.org/xdg-specs/idle-inhibit-spec/latest/re01.html) for `Inhibit` and `UnInhibit`, and the member signatures of the [KDE screen locker's interface](https://github.com/KDE/kscreenlocker/blob/master/dbus/org.freedesktop.ScreenSaver.xml) for the rest. The specification defines no error codes, so the errors below are Gnoblin's own.

| Member                 | Arguments                               | Result            | Errors and behavior                                                                                                                                                      |
| ---------------------- | --------------------------------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Inhibit`              | `application: string`, `reason: string` | `cookie: uint32`  | Holds the screen awake until released. A caller may hold 64 inhibits; a 65th is `org.freedesktop.DBus.Error.LimitsExceeded`.                                             |
| `UnInhibit`            | `cookie: uint32`                        | None              | Releases a cookie the caller holds. A cookie nobody holds, or one another caller holds, is `org.freedesktop.DBus.Error.AccessDenied` and changes nothing.                |
| `GetActive`            | None                                    | `active: boolean` | Reports the lock state. It is an error when the compositor publishes no lock object, since nothing reported the state.                                                   |
| `SetActive`            | `active: boolean`                       | `active: boolean` | `true` requests a lock and returns `true` once confirmed; a refused lock is an error. `false` is `org.freedesktop.DBus.Error.NotSupported`, since clients cannot unlock. |
| `Lock`                 | None                                    | None              | Requests a lock. Succeeds only once the compositor confirms the session is locked. A refusal, a failed cover or a timeout is an error.                                   |
| `GetActiveTime`        | None                                    | `seconds: uint32` | Seconds the session has been locked through this service, or `0` when it is not locked.                                                                                  |
| `GetSessionIdleTime`   | None                                    | `seconds: uint32` | Seconds since the last input event. It is an error when the compositor's idle object is missing, rather than reporting `0`.                                              |
| `SimulateUserActivity` | None                                    | None              | `org.freedesktop.DBus.Error.NotSupported`. The compositor does not take simulated activity yet.                                                                          |

## Signal

| Signal          | Arguments         | Behavior                                                                                                     |
| --------------- | ----------------- | ------------------------------------------------------------------------------------------------------------ |
| `ActiveChanged` | `active: boolean` | Emitted once each time the lock state changes. It is not emitted for a repeat of the state already reported. |

## Inhibit and release

Call `Inhibit` and pass its returned cookie to `UnInhibit` when the activity ends. Run these commands in a terminal; the first prints the cookie and the second releases it after you substitute that value:

```sh
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver Inhibit ss example-player "Video playback"
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver UnInhibit u COOKIE
```

`application` and `reason` appear only in the service log. The hold is identified by the caller's D-Bus unique name and the cookie. `UnInhibit` only accepts a cookie returned to that same caller.

When a caller leaves the bus, every hold it made is released, so a player that exits without calling `UnInhibit` does not keep the screen awake.

Gnoblin also holds a logind sleep inhibitor. On `PrepareForSleep(true)` (suspend or hibernate) it locks the session while it still holds the inhibitor, then releases it. On resume it takes the inhibitor again without locking. A lock on the session's own logind object (`loginctl lock-session`) locks the session the same way as `Lock`.

## Request a lock

```sh
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver Lock
```

A successful method reply means the compositor confirmed the session is locked. Check session lock state through the [Lua runtime API](/shell-api/session-runtime) to read the same state from the desktop side.

## GNOME name

The service also serves `org.gnome.ScreenSaver` at `/org/gnome/ScreenSaver` on the session bus. Both names answer from the same lock and idle state and share one inhibit table, so a cookie from `Inhibit` on either name is released by `UnInhibit` on either name.

| Member               | Arguments                               | Result            | Errors and behavior                                                                                        |
| -------------------- | --------------------------------------- | ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `Inhibit`            | `application: string`, `reason: string` | `cookie: uint32`  | Same as the freedesktop name.                                                                              |
| `UnInhibit`          | `cookie: uint32`                        | None              | Same as the freedesktop name.                                                                              |
| `GetActive`          | None                                    | `active: boolean` | Same as the freedesktop name.                                                                              |
| `SetActive`          | `active: boolean`                       | None              | `true` requests a lock and succeeds once confirmed; a refused lock is an error. `false` is `NotSupported`. |
| `Lock`               | None                                    | None              | Same as the freedesktop name.                                                                              |
| `GetActiveTime`      | None                                    | `seconds: uint32` | Same as the freedesktop name.                                                                              |
| `GetSessionIdleTime` | None                                    | `seconds: uint32` | Same as the freedesktop name.                                                                              |

On this name, `ActiveChanged` is emitted from `/org/gnome/ScreenSaver`, once for each change of the lock state.

`SimulateUserActivity` is not on this name. Call it on `org.freedesktop.ScreenSaver`, where it is refused with `NotSupported`.

The name is requested without queueing. When another owner holds `org.gnome.ScreenSaver`, the service logs the owner and does not take the name, and the freedesktop name is served as usual. The refusal is named in the service log.

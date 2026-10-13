# Screen Saver Interface

Use `org.freedesktop.ScreenSaver` to inhibit idle suspend, inspect lock state, or request a lock.

Gnoblin exports this interface on the session bus as `org.freedesktop.ScreenSaver` at `/org/freedesktop/ScreenSaver`. Some methods forward to GNOME's screen saver when it owns that service; otherwise, Gnoblin handles them through the session lock and logind.

| Method      | Arguments                               | Result            | Errors and behavior                                                                                                                                              |
| ----------- | --------------------------------------- | ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Inhibit`   | `application: string`, `reason: string` | `cookie: uint32`  | Acquires a logind idle inhibitor. The inhibitor is tied to the caller's D-Bus name.                                                                              |
| `UnInhibit` | `cookie: uint32`                        | None              | Releases a cookie owned by the caller. Unknown or foreign cookies return `org.freedesktop.DBus.Error.AccessDenied`.                                              |
| `GetActive` | None                                    | `active: boolean` | Reports current lock state when handled by Gnoblin, or forwards to GNOME's screen saver.                                                                         |
| `SetActive` | `active: boolean`                       | None              | `true` requests a lock when Gnoblin handles the call; `false` is rejected with `org.freedesktop.DBus.Error.NotSupported`. Forwarded when GNOME owns the service. |
| `Lock`      | None                                    | None              | Requests a lock when Gnoblin handles the call, or forwards to GNOME's screen saver.                                                                              |

## Inhibit and release

Call `Inhibit` and pass its returned cookie to `UnInhibit` when the activity ends. Run these commands in a terminal; the first prints the cookie and the second releases it after you substitute that value:

```sh
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver Inhibit ss example-player "Video playback"
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver UnInhibit u COOKIE
```

`application` and `reason` are strings. If `application` is empty, Gnoblin uses the D-Bus caller name. `UnInhibit` only accepts a cookie returned to that same caller.

## Request a lock

```sh
busctl --user call org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver org.freedesktop.ScreenSaver Lock
```

A successful method reply confirms the request was accepted. Check session lock state through the [Lua runtime API](/shell-api/session-runtime) to confirm that the lock transition completed.

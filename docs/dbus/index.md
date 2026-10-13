# D-Bus Interfaces

Use Gnoblin D-Bus interfaces for session integration and idle monitoring.
Availability depends on the standalone session and the relevant service.

## `org.gnoblin.Compositor`

The compositor exports this interface at `/org/gnoblin/Compositor`. Its
`CheckPermission` method is restricted to the Gnoblin portal backend. The
method takes two strings and returns a decision, matching rule, matching rule
names, revision, and a boolean result. Portal clients should use the supported
[permission workflow](/guides/permissions) instead of calling it directly.

## `org.gnoblin.SessionIdle`

The session idle service is available at `/org/gnoblin/SessionIdle`.
`GetActivity()` returns the same values as the `ActivityChanged` signal:

| Value          | D-Bus type              | Meaning                                            |
| -------------- | ----------------------- | -------------------------------------------------- |
| `available`    | boolean                 | Whether the idle monitor is available.             |
| `idle`         | boolean                 | Whether the fixed idle threshold has elapsed.      |
| `threshold_ms` | unsigned 64-bit integer | Idle threshold in milliseconds; currently 120,000. |
| `idle_for_ms`  | unsigned 64-bit integer | Elapsed idle time, or zero when not idle.          |

The signal is sent when availability or idle state changes. Its duration is a
sample and does not update continuously. Idle inhibitors and desktop timeout
preferences do not change the fixed threshold.

The service also implements `org.freedesktop.ScreenSaver` for inhibition and
lock requests. See the [screen saver interface reference](/dbus/screensaver).

## Former interfaces

`org.gnoblin.LaunchFeedback` is not implemented in current source. Use the
supported [launch feedback APIs](/shell-api/launch-feedback).

`org.gnoblin.Shell` belonged to the former GNOME Shell integration and is not
provided in the standalone Gnoblin session. Shell clients should use the
[compositor bridge](/bridge/).

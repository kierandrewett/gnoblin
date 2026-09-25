# Launch feedback

Show a busy cursor while an application starts. This API supplies feedback;
it does not launch the application.

## Availability

Launch feedback is built into the Gnoblin session. The CLI and shell clients
can use it without installing a user script.

It uses the compositor's themed wait cursor. Application focus is unchanged.

## Use the CLI

```sh
gnoblinctl launch begin example org.gnome.Nautilus 3000
gnoblinctl launch status
gnoblinctl launch end example
```

Here `example` is your request token, `org.gnome.Nautilus` identifies Files,
and `3000` sets a three-second timeout. The commands alone show and clear the
busy cursor; they do not open Files.

Wait for `begin` to return before launching the app.
Use a unique token for each launch and call `end` if launch fails.

Requests end when a matching window appears or gains focus, on explicit end,
or after the timeout. Overlapping requests keep the cursor busy until all end.

## D-Bus API

Name/interface: `org.gnoblin.LaunchFeedback`.
Object: `/org/gnoblin/LaunchFeedback`.

| Method     | Signature    | Result                                                         |
| ---------- | ------------ | -------------------------------------------------------------- |
| `Begin`    | `(ssu) → ()` | Start feedback for `(token, app hint, timeout in ms)`.         |
| `End`      | `s → ()`     | End the request with this token; an unknown token is harmless. |
| `GetState` | `() → s`     | Return a JSON state record.                                    |

`Begin` requires a nonempty token of at most 128 characters and an app hint of
at most 512 characters. Up to 64 distinct requests can be pending. The timeout
is clamped to 100–10000 ms. Reusing a token replaces its earlier request.

The app hint is matched case-insensitively against a window's GTK application
ID, WM class, WM class instance, desktop ID or desktop name. A trailing
`.desktop` is ignored. Windows marked skip-taskbar do not complete a request.

`GetState` returns these JSON fields:

| Field            | Meaning                                                      |
| ---------------- | ------------------------------------------------------------ |
| `busy`           | Whether the wait cursor is currently active.                 |
| `pending`        | Number of launch requests still pending.                     |
| `nativeCursor`   | Whether this Mutter build provides Gnoblin's cursor support. |
| `pointerVisible` | Whether the pointer is visible.                              |
| `spinnerVisible` | Whether the themed wait cursor is active.                    |
| `cursorSource`   | Cursor theme source, or `null` before feedback has started.  |
| `position`       | Reserved; currently `null`.                                  |

Requests expire even if a launcher exits before sending `End`. The cursor
override is also released when the Gnoblin session shuts down.

## Test

```sh
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-launch-feedback.py" scripts/run-gnome-shell.sh
```

Checks pointer output, overlapping requests, expiry, reload and window-map completion.

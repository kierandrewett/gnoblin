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

| Method     | Arguments                      | Result                                    |
| ---------- | ------------------------------ | ----------------------------------------- |
| `Begin`    | Token, app hint, timeout in ms | Start feedback                            |
| `End`      | Token                          | End feedback; unknown token is harmless   |
| `GetState` | None                           | JSON count, visibility and artwork source |

Timeouts are bounded to 100–10000 ms. The app hint may be a desktop ID,
WM class, GTK application ID or desktop name.

Requests expire even if a launcher exits before sending `End`. The cursor
override is also released when the Gnoblin session shuts down.

## Test

```sh
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-launch-feedback.py" scripts/run-gnome-shell.sh
```

Checks pointer output, overlapping requests, expiry, reload and window-map completion.

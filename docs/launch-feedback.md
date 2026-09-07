# Global launch feedback

The `launch-feedback.js` user script owns timed global busy-cursor requests.
It uses Mutter's native themed wait-cursor override when available, including
Hyprcursor themes. On an older compositor, it displays the installed GNOME
cursor theme's `wait` artwork, hotspot and animation frames in an input-transparent
overlay, with Adwaita as a fallback. Window focus and input remain with the
application under the pointer.

Install or link `src/scripts/launch-feedback.js` into
`~/.config/gnoblin/scripts/`, then run `gnoblinctl reload-scripts`. The script is
also loaded at login. Reloading it releases the native override or fallback
visibility inhibitor.

The session-bus name and interface are `org.gnoblin.LaunchFeedback`, at
`/org/gnoblin/LaunchFeedback`:

| Method | Arguments | Behaviour |
| --- | --- | --- |
| `Begin` | token (string), application (string), timeout (uint32, milliseconds) | Start one busy-cursor request. |
| `End` | token (string) | Release that request. Unknown tokens are harmless. |
| `GetState` | none | Return JSON with pending count, cursor visibility and artwork source. |

Tokens must be unique per launch. The application hint can be its desktop ID,
WM class, GTK application ID or desktop name. Requests complete when a matching
window maps or gains focus, when the client calls `End`, or when their timeout
expires. Timeouts are bounded to 100-10000 ms. Quickshell uses 3000 ms. Overlapping
requests keep the cursor busy until all have completed.

The launcher must wait for `Begin` to return before starting the application, so
fast windows cannot map before the service records its baseline. Search leaves
the request with Gnoblin when its popup closes after successful dispatch. A
failed activation or explicit cancellation releases it immediately. If a client
dies, the compositor-owned timeout still restores the cursor.

```sh
gnoblinctl launch-begin example org.gnome.Nautilus 3000
gnoblinctl launch-state
gnoblinctl launch-end example
```

Run the isolated integration check with:

```sh
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-launch-feedback.py" scripts/run-gnome-shell.sh
```

It checks a virtual pointer, overlapping requests, timeout, script reload, and
completion when a real Foot window maps. It does not alter the live session.

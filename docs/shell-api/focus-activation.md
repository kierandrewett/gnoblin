# Focus and activation

`gnoblin.focus.policy` returns a read-only snapshot of the supported focus
settings. Its `revision` identifies the committed settings snapshot.

The snapshot includes:

- `focus_mode` and `focus_new_windows`.
- `raise_on_click`, `auto_raise`, and `focus_change_on_pointer_rest`.
- `auto_raise_delay` and `revision`.

| Field                          | Values                          | Default   | Effect                                      |
| ------------------------------ | ------------------------------- | --------- | ------------------------------------------- |
| `focus_mode`                   | `click`, `hover`, `hover-strict` | `click`   | Sets how pointer movement changes focus.    |
| `focus_new_windows`            | `allow`, `prevent`               | `prevent` | Sets focus policy for new windows.          |
| `raise_on_click`               | Boolean                         | `true`    | Raises a window when clicked.               |
| `auto_raise`                   | Boolean                         | `false`   | Raises the focused window automatically.    |
| `auto_raise_delay`             | Integer from 0 to 10000 ms       | `500`     | Sets the automatic raise delay.             |
| `focus_change_on_pointer_rest` | Boolean                         | `false`   | Changes focus after pointer rest.           |

The Default column shows the compositor's value when a key is missing. The
starter config sets `focus_new_windows` to `allow`.

These are committed config values. See [focus and raising options](/config/configure/window_management#focus-and-raising)
for when each setting takes effect.

`gnoblin.focus.policy-changed` runs after a successful config commit only when
one of these effective focus settings changes. Its `policy` field contains the
committed snapshot, and the event's `revision` matches `policy.revision`.
Rejected or failed config changes do not emit the event.

Use `gnoblin.focus.history(filter?)` to read native window snapshots in recent
focus order. Filters accept `workspace_id`, `monitor_id`, and `limit`. Workspace
and monitor IDs match exactly. The limit is 1–256 and defaults to 50; filters
may return fewer records.

History includes minimized windows and removes windows after they close. At
startup, only the focused window seeds the recent order. Other open windows
stay in snapshot order until the runtime observes their focus.

The settings property becomes available after the initial config load commits.
Native compositor snapshots are seeded after the initial Lua config
evaluation. They are unavailable while Gnoblin first loads the config file, so
call these read methods from a runtime event callback after native startup.
Calling one before the snapshot is ready raises a Lua error.



Window snapshot properties are read-only. In the native runtime,
`Window:focus(context)` requests focus using the context from
`gnoblin.shortcut.activated`. The callback provides that value only for a
trusted shortcut key press. It expires after five seconds.
A successful focus operation completes with `{id = window.id}`.

The same context can authorize `begin_move` or `begin_resize`. It works once
across all three operations. A config reload invalidates contexts from the old
Lua runtime; a session lock also revokes them.

Mutter still applies its normal window activation policy.

Contexts are not provided for clicks, external-client shortcuts, release
bindings, synthetic input, input-method events, or repeated key presses. Direct
`Window:focus`, `Window:begin_move`, and `Window:begin_resize` calls without a
context remain denied.

For the socket activation contract, see the [compositor bridge](/compositor-bridge).

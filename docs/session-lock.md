# Session locking

Gnoblin uses the standard `ext-session-lock-v1` Wayland protocol in the
Gnoblin session. The compositor owns the security boundary: it covers every
output, stops normal input, replaces desktop capture with its lock scene, and
stays locked if a locker exits. The locker owns its appearance and
authentication. See the [protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/session-lock/ext-session-lock-v1.xml).

Gnoblin does not provide a default lock screen. Bingux is one separate shell
project with its own lock client.

Third-party lockers such as hyprlock, swaylock, gtklock and waylock can connect
directly to the same standard protocol. Any client with access to this user's
Wayland socket can request the lock; Gnoblin does not filter requests by
process. The first request owns the lock, and concurrent requests receive
`finished`.

In Bingux, use its Lock action or run `bingux-lock`. With another locker
installed, run its command (for example, `hyprlock`) in your Gnoblin session.
Your shell or idle daemon decides when to run that command; Gnoblin does not
set a timeout or launch a locker for you.

The regular GNOME session is separate. It does not advertise this protocol and
continues to use GNOME ScreenShield.

The protocol is enabled by default in Gnoblin. To hide its global, set
`ext_session_lock = false` under `gnoblin.configure.protocols`, then log out and
back in; protocol globals are registered at compositor startup. See the
[protocol setting reference](/config/configure/protocols).

## Portal and remote access while locked

Existing portal permission remains in force. A monitor stream already
authorised through the desktop portal receives the compositor lock scene while
the session is locked, rather than normal desktop content.

Remote keyboard and pointer input is available after `locked` and lock-scene
presentation, and is routed only to the active lock surface. It is refused
during the transition and failsafe states. This requires no lock-specific
portal setting, opt-in, or change to an existing authorised grant.

## Locker requirements

A locker binds `ext_session_lock_manager_v1` and sends `lock`. The compositor
then sends `locked` or `finished` on the new lock object. `finished` means the
request was not granted; do not show an authentication UI for that lock.

After requesting the lock, create one `wl_surface` per output and pass each to
`get_lock_surface` before attaching or committing a buffer. An output can have
only one lock surface for this lock. For every `configure(serial, width,
height)`, acknowledge its serial before committing a non-null buffer with those
exact surface-local dimensions. If multiple configures arrive before a commit,
acknowledging the latest one is sufficient.

| Interface                     | Request or event                    | Contract                                                                                                         |
| ----------------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `ext_session_lock_manager_v1` | `destroy`                           | Release the manager object; existing lock objects remain valid.                                                  |
| `ext_session_lock_manager_v1` | `lock`                              | Create a lock request; it later receives `locked` or `finished`.                                                 |
| `ext_session_lock_v1`         | `destroy`                           | Cancel or retire a lock only before `locked`; destroy its lock surfaces too.                                     |
| `ext_session_lock_v1`         | `get_lock_surface(surface, output)` | Create the output's lock surface before the `wl_surface` has a role or buffer.                                   |
| `ext_session_lock_v1`         | `locked`                            | The lock scene has been presented and it is safe to treat the session as locked.                                 |
| `ext_session_lock_v1`         | `finished`                          | The compositor did not grant this lock request.                                                                  |
| `ext_session_lock_v1`         | `unlock_and_destroy`                | After authentication succeeds and `locked` was received, release the session lock.                               |
| `ext_session_lock_surface_v1` | `ack_configure(serial)`             | Acknowledge a configure before committing its buffer.                                                            |
| `ext_session_lock_surface_v1` | `configure(serial, width, height)`  | Resize to the exact output-local dimensions, acknowledge `serial`, then commit a matching buffer.                |
| `ext_session_lock_surface_v1` | `destroy`                           | Retire the lock surface. Removing one for an active output before unlock makes Gnoblin show a solid color there. |

After `locked`, only the owner calls `unlock_and_destroy`; destroying the lock
object instead is a protocol error. If the client exits immediately after
unlocking, send `wl_display.sync` and wait for its callback before disconnecting
so the compositor processes the request. A locker crash leaves the compositor
covered and locked; another client may acquire the lock afterwards.

The protocol rejects a null lock-surface buffer, a commit before the first
configure is acknowledged, a buffer with the wrong dimensions, duplicate
output surfaces, and destroying the active lock without unlocking. See the
[protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/session-lock/ext-session-lock-v1.xml)
for the complete error enums.

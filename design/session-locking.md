# Session locking

Gnoblin's standalone Mutter implements the standard `ext-session-lock-v1`
protocol. The compositor enforces coverage, input isolation, presentation
ordering, and the locked fallback if a locker exits. An external Wayland client
owns the lock UI and authentication. Gnoblin does not choose or launch a
locker.

The protocol global is enabled by default in the supervised Gnoblin session.
Disable it for the next login with:

```lua
gnoblin.configure {protocols = {ext_session_lock = false}}
```

Regular GNOME sessions do not receive the global and keep using GNOME
ScreenShield. See the [protocol implementation notes](../src/protocols/session-lock/README.md)
and the [user guide](../docs/session-lock.md).

## API boundary

`gnoblin.session.lock()` asks subscribed shell clients to show their lock UI.
Its result reports request delivery; it does not confirm that a lock screen was
shown or that the compositor is locked. The lock client then acquires
`ext-session-lock-v1` directly. Lua clients can observe
`gnoblin.session.lock-state-changed` and read `gnoblin.session.status()`.
`gnoblin.session.lock-requested` is a socket event for external shell clients,
not a Lua configuration event.

Any client with access to the user's Wayland socket can request the first lock.
The protocol does not authenticate a client's password or apply a PID
allowlist. The selected locker owns authentication; the compositor enforces the
seat lock. Applications that must not trust one another need separate Wayland
socket access.

## Compositor guarantees

The controller moves through `unlocked`, `covering`, `locked`, and `failsafe`.
On a valid request, it covers every output and blocks normal input before
accepting a lock surface. It sends `locked` only after a protected frame has
been presented on each output. A locker must acknowledge each surface
configuration and commit a buffer with the configured dimensions.

Only one locker owns the active lock. Concurrent requests receive `finished`.
Only the owner can unlock, and only after `locked`. If it exits while covering
or locked, the compositor stays covered in `failsafe`; a replacement may take
over without exposing the desktop. Output, monitor, and stack changes reset
the presentation barrier.

During a lock transition, normal keyboard, pointer, touch, shortcut, clipboard,
and capture paths remain isolated. An already-authorized portal monitor stream
may receive the presented lock scene. Remote input is admitted only after the
lock scene is presented and is routed to its active surface. These guarantees
come from Mutter; a fullscreen or layer-shell window alone cannot provide
them.

## Verification

The current checks cover different boundaries:

- `tests/session-lock-protocol.test.py` checks protocol and fail-closed source
  invariants. It does not prove runtime behavior.
- `tests/session-lock-api.test.py` checks the shared Lua lock request contract.
- `tests/session-lock-runtime/run-nested.sh` exercises the supervised Lua
  request, protocol presentation, contention, owner death, and takeover in a
  nested devkit.
- `tests/session-lock-runtime/run-raw-remote-path.sh` checks Mutter's raw
  ScreenCast and RemoteDesktop path. It does not prove portal or RustDesk
  integration.

The nested devkit does not prove display scanout, physical input isolation,
hotplug, suspend/resume, or a fresh installed login. Before claiming those
paths, use a fresh GDM login with an external locker and verify:

1. The lock client shows an accessible UI on every output, and authentication
   succeeds, fails, and cancels without exposing the session.
2. Keyboard, pointer, touch, tablet, shortcuts, Xwayland, clipboard, and
   capture cannot reach ordinary clients while locked.
3. Killing or reloading the locker, changing output scale or rotation, and
   hotplugging a monitor keep the compositor covered until a fresh protected
   presentation is confirmed.
4. Suspend and resume preserve coverage; unlock restores normal focus, input,
   cursor, capture, and desktop services.
5. Logout and GNOME session switchback leave GNOME's own lock behavior intact.

See the [runtime smoke suite](../tests/session-lock-runtime/README.md) for the
available commands and the limits of each test.

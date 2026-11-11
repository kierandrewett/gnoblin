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

## Rust compositor status

The Rust compositor (`crates/gnoblin-wayland`) covers every output with a solid cover
from the first frame presented while locked, including outputs hotplugged during the
lock. Nothing else is drawn over the cover: no modal, cursor or client content. A
modal cannot be set while locked. Capture is refused during the lock, and streams that
were open before it are stopped. Lock surfaces are tracked for mapping and input but
are not composed, so the locker's own UI is not shown yet. The portal-stream guarantee
in the section below is the Mutter behaviour and is not implemented by the Rust
compositor.

## API boundary

`gnoblin.session.lock()` asks subscribed shell clients to show their lock UI.
Its result reports request delivery; it does not confirm that a lock screen was
shown or that the compositor is locked. The lock client then acquires
`ext-session-lock-v1` directly. Lua clients can observe
`gnoblin.session.lock_state_changed` and read `gnoblin.session.status()`.
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

## Shell lock node

The stock Lua shell drives the compositor's `lock` node (`class lock`):

- `requested` is emitted to the shell when a lock is requested. `Lock:on_requested` in `lock.lua` engages the lock.
- `authenticate` checks the password for the session user on a worker thread, behind a rate limiter (three denials, then a 2 s wait; success resets it). The shell gets `token` with `result` `ok` on accept, or `error` `denied`, `rate-limited`, `not-locked`, `busy` or `failed: <reason>`.
- Only the compositor unlocks. The shell's answer never disengages the lock on its own, and with no session user every attempt fails closed.

Residual limits:

- The compositor does not load the stock library yet. The lock screen runs where a shell VM loads it; `a_session_lock_request_engages_the_stock_lock_screen` in `crates/gnoblin-compositor/tests/shell_vm.rs` is the real-VM proof, and it covers request to engage only.
- Unlock through `Lock:unlock` is tested against the fake core in `tests/lua/ui_test.lua`, not through PAM in a VM.
- The real-PAM test (`crates/gnoblin-lock-auth/src/pam.rs`) is `#[ignore]`d and needs `GNOBLIN_PAM_TEST=1` plus a test account.
- No real-VM check renders the lock screen's pixels.

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

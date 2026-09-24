# `ext-session-lock-v1`

Gnoblin implements the standard `ext-session-lock-v1` protocol so a locker
such as Bingux, hyprlock, swaylock, gtklock, or waylock can provide its own
lock UI. Gnoblin does not launch a locker, authenticate users, select a
default locker, or expose a separate lock protocol.

## Current status

The manager is advertised only when
`GNOME_SHELL_SESSION_MODE=gnoblin`, through the existing
`gnoblin_config_protocol_enabled("ext-session-lock")` predicate. It defaults
on in that session and can be disabled in Gnoblin's `[protocols]` settings. A
regular GNOME session never receives this global, so GNOME ScreenShield remains
its lock implementation. The native capability returns one only after the
global is created, allowing GNOME Shell to cut over to the compositor lock.

The isolated Mutter lifecycle and real hyprlock protocol paths pass. A fresh
installed session remains necessary to verify Bingux, portal capture and
RustDesk end to end.

## Protocol and ownership

The implementation follows version 1 of the upstream protocol:

- The first client to call `lock()` while unlocked owns the active lock.
- A concurrent lock request receives `finished`.
- Each active output has at most one `MetaWaylandSessionLockSurface`, with the
  normal configure/ack handshake and strict first-buffer size checks.
- Only the active owner may call `unlock_and_destroy`, and only after its
  `locked` event.
- A client that exits while locked leaves the compositor in **FAILSAFE**: the
  opaque cover and input embargo remain. A later client may take over that
  failsafe lock and must receive a freshly presented covered frame before its
  own `locked` event.

There is no PID allowlist or private launcher requirement. A session lock is
therefore selected by standard Wayland session policy: once the protocol is
advertised, any client permitted to connect to that compositor can request the
first lock. This keeps standard clients interoperable and makes the normal
Wayland socket/session boundary the admission boundary.

## Compositor invariants

On a lock transition the controller:

1. Creates a full-stage opaque black cover and reparents only verified lock
   role actors above it.
2. Disables direct scanout/unredirect and hides the cursor while the cover is
   active.
3. Reasserts its scene as the last stage child on `child-added` and before
   every paint. This handles direct stage restacks from Shell UI such as
   ripples.
4. Resets the presentation barrier on output, monitor, or stack changes. Each
   stage view must present a frame with a global frame counter newer than the
   reset baseline before `locked` can be sent.
5. Embargoes normal session input from the early Mutter event path. A verified,
   mapped lock surface is the only allowed destination; a miss is consumed.
6. Notifies private consumers synchronously on state changes. Direct capture,
   clipboard and data control cannot reach ordinary session data. An existing
   authorised portal monitor stream sees the lock scene after `LOCKED` and
   presentation; authorised remote input then routes to the active lock
   surface. Earlier transitions and failsafe refuse remote input.

`unlock_and_destroy` removes the lock scene and input embargo in one Mutter
main-loop transaction, balances cursor/direct-scanout holds, and then reports
`UNLOCKED`. A client disconnect never takes that path.

## Installed-session acceptance

The isolated tests cover protocol ordering and hyprlock's basic path. Before
claiming release verification, run a fresh installed Gnoblin session with
Bingux and a third-party client:

- Lock, suspend, and resume: every output remains covered before and after
  `locked`; no queued pre-cover frame is accepted.
- Verify ordinary keyboard shortcuts, IME, pointer, touch, tablet, virtual
  input, Xwayland grabs and clipboard/DnD/data-control cannot reach normal
  clients. An authorised portal monitor stream must show only the lock scene,
  and remote input must reach only the lock UI after presentation.
- Kill and reload the locker. The cover must persist; a replacement client can
  take ownership without exposing the desktop.
- Hotplug, unplug, rotate, and scale outputs. The new or reconfigured output
  stays covered until a fresh covered presentation and lock-surface configure.
- Unlock as the owner and verify normal focus, cursor visibility, direct
  scanout, capture, and GNOME session consumers recover together.

The relevant source-level guard is `tests/session-lock-protocol.test.py`; it
does not replace these runtime checks.

# `ext-session-lock-v1`

Gnoblin implements the standard `ext-session-lock-v1` protocol so a locker
such as Bingux, hyprlock, swaylock, gtklock, or waylock can provide its own
lock UI. Gnoblin does not launch a locker, authenticate users, select a
default locker, or expose a separate lock protocol.

## Current status

**Implemented behind an unadvertised manager global.** The generated protocol,
manager state machine, lock-surface role, compositor cover, recovery path, and
input/capture integration seams are present. `meta_wayland_session_lock_get_capability()`
returns `0`, and `wl_global_create()` is deliberately absent. Therefore no
client can bind the protocol in a normal Gnoblin session yet.

This is a security boundary, not a feature flag. The global may only be
advertised after the runtime acceptance suite below passes in the patched
Mutter build. Until then, GNOME's established lock path remains responsible
for locking a shipped session.

When enabled, the manager is created only when
`GNOME_SHELL_SESSION_MODE=gnoblin`, through the existing
`gnoblin_config_protocol_enabled("ext-session-lock")` predicate. It defaults
on in that session and can be disabled in Gnoblin's `[protocols]` settings. A
regular GNOME session never receives this global, so GNOME ScreenShield remains
its lock implementation.

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
6. Notifies private consumers synchronously on state changes. Capture, remote
   input, clipboard, data control, and related integrations must deny access
   in every state except `UNLOCKED`.

`unlock_and_destroy` removes the lock scene and input embargo in one Mutter
main-loop transaction, balances cursor/direct-scanout holds, and then reports
`UNLOCKED`. A client disconnect never takes that path.

## Runtime acceptance before advertisement

The source guard tests only protect wiring. Before changing the capability or
creating a global, run a real Gnoblin session with at least Bingux and one
unmodified third-party client:

- Lock, suspend, and resume: every output remains covered before and after
  `locked`; no queued pre-cover frame is accepted.
- Verify keyboard shortcuts, IME, pointer, touch, tablet, virtual input,
  Xwayland grabs, clipboard/DnD/data-control, screencast, remote desktop, and
  portal capture cannot reach normal-session data while locked.
- Kill and reload the locker. The cover must persist; a replacement client can
  take ownership without exposing the desktop.
- Hotplug, unplug, rotate, and scale outputs. The new or reconfigured output
  stays covered until a fresh covered presentation and lock-surface configure.
- Unlock as the owner and verify normal focus, cursor visibility, direct
  scanout, capture, and GNOME session consumers recover together.

The relevant source-level guard is `tests/session-lock-protocol.test.py`; it
does not replace these runtime checks.

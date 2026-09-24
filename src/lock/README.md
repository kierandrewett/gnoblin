# Gnoblin lock broker (prototype)

`gnoblin-lockd` is the policy boundary between logind and a desktop shell's
locker.  It is **off by default** and is not added to the Gnoblin session target.
It must stay that way until the compositor implements and hardware-validates
`ext-session-lock-v1`.

The broker listens for the current logind session's `Lock` signal and
`PrepareForSleep(true)`. It holds a logind `sleep` *delay* inhibitor from daemon
startup and launches the configured locker once. It has no trusted compositor
callback yet, so it never releases that FD based on a locker report. Logind's
configured maximum delay eventually proceeds; this prototype therefore cannot
make a suspend safety claim or replace GNOME ScreenShield.

## State contract

```
unlocked --request--> requested --compositor callback--> compositor-locked
                                     |
                                     +-- timeout/client death/failure --> failed
```

There is no D-Bus `Unlock` method.  The locker must authenticate and send
`ext_session_lock_v1.unlock_and_destroy`.  Quickshell currently exposes no
server round-trip confirmation after that request, so the public prototype has
no completion method. A future compositor callback will update the broker.

`ReportPresented` is diagnostic only. The token avoids accidental reports from
unrelated applications but does not authenticate a hostile same-UID process and
cannot advance the lock state or release the sleep inhibitor. Production
confirmation must come from the compositor after it has blanked every output and
isolated keyboard, pointer, touch and tablet input.

## Idle timeout and inhibitors

`IdleTimeoutSeconds` uses Mutter's session-wide `org.gnome.Mutter.IdleMonitor`,
which measures actual compositor input. A positive value creates an `AddIdleWatch`
and also locks immediately if the session is already past the threshold at broker
startup. `Inhibit(application, reason)` returns a cookie; it applies only to the
calling D-Bus unique name, and is removed if that caller disconnects. `UnInhibit`
only accepts cookies issued to the same caller. The resulting request reason is
available through `GetLastReason` (`manual`, `idle`, `login1`, or `sleep`).

## Required compositor gate

The existing [protocol plan](../protocols/session-lock/README.md) describes the
missing implementation.  Before enabling this service, test a real session with:

1. first lock frame blacking every output before `locked`;
2. no keyboard, pointer, touch, tablet, shortcuts, selection or normal content
   access while locked;
3. lock client `kill -9` keeping a black, input-isolated session;
4. hotplug, resize, DPMS, suspend/resume and two or more displays; and
5. failed authentication, successful PAM/fingerprint authentication and recovery.

Only then should session wiring replace GNOME ScreenShield.  The two cannot own
the ScreenSaver compatibility APIs concurrently.  `OwnScreenSaverNames` is held
for that integration and does nothing in this prototype.

If a trusted locker crashes after `presented`, the compositor must retain its
black cover and input isolation.  The future compositor/broker recovery contract
may launch a replacement trusted locker while that cover remains in place.  It
must use bounded exponential backoff and a finite retry limit; this prototype
does not attempt recovery because the server cannot yet transfer lock ownership.

## Bingux locker contract

Bingux creates one fresh `ext_session_lock_surface_v1` per output, waits for and
acknowledges configure before its first buffer, then calls `ReportPresented` with
`GNOBLIN_LOCK_TOKEN` only after the protocol `locked` event.  It retains its
Wayland lock object until authentication succeeds and sends
`unlock_and_destroy`. It makes no D-Bus completion report: Quickshell lacks a
completed server round trip and `secure=false` is ambiguous.

The broker starts only when explicitly installed/enabled:

```ini
# ~/.config/gnoblin/lock.conf
[Lock]
Enabled=true
Command=/path/to/bingux-lock
```

Install the unit manually during development; packaging/session integration is
intentionally withheld pending the compositor gate.

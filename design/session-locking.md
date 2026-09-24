# Session locking design

Gnoblin is moving session locking from GNOME Shell's `ScreenShield` to a
compositor-enforced Wayland lock. This page records the security contract and
the rollout gates. **The running Gnoblin session still uses GNOME's lock screen
until the compositor passes the protocol checks below.** A vendored
protocol XML file, a build that succeeds, or a lock window that looks correct
does not establish a secure lock.

## Why a compositor lock

An ordinary fullscreen or layer-shell window cannot guarantee that the desktop
underneath is hidden, that every output is covered, or that keyboard, pointer,
touch, shortcuts and capture stop reaching ordinary clients. The compositor
must make these decisions before it acknowledges a lock. The standard
[`ext-session-lock-v1` protocol](https://wayland.app/protocols/ext-session-lock-v1)
defines this boundary. Its `locked` event is sent only after a locked frame has
been presented on every output. If the client dies, the compositor stays
locked. [Hyprland uses this split](https://github.com/hyprwm/Hyprland/blob/main/src/managers/SessionLockManager.cpp):
the compositor enforces the lock, while
[hyprlock](https://github.com/hyprwm/hyprlock) supplies the graphics and
authentication.

Mutter does not currently provide this protocol in Gnoblin. The XML under
`src/protocols/session-lock/` is a specification, not an advertised global.
The compositor implementation must be validated before any external lock
client replaces `ScreenShield`.

## Responsibilities

| Component                   | Owns                                                                                                                                   |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Gnoblin's Mutter fork       | Lock state, opaque fallback on every output, input and capture isolation, lock surface placement, and the `ext-session-lock-v1` server |
| Bingux lock client, current | Lock-screen appearance, accessible prompts, authentication flow, and `unlock_and_destroy` after successful authentication              |
| Bingux policy, future       | Optional manual, idle, logind, sleep, timeout, inhibitor, and compatibility D-Bus integration                                          |
| GDM and logind              | Login/greeter and system session management; both remain installed                                                                     |

The Bingux desktop shell process is separate from its lock client. Reloading
the bar, dock or settings cannot dismiss a lock. The compositor owns the blank
fallback, so a crashed Bingux lock client cannot expose the desktop. Another
shell can supply a different conforming lock client.

## Compositor state machine

```text
unlocked → covering → locked → unlocked
                  ↘ failsafe ↗
```

On a valid `lock()` request, the compositor enters **covering** immediately:
normal content and input are suppressed, and an opaque compositor-owned cover
is scheduled on every output. It may wait briefly for lock surfaces. It enters
**locked** and sends the protocol event only after a locked frame or opaque
fallback has actually been presented on every output. A missing surface never
extends this wait indefinitely.

While locked, only the active lock client's correctly configured surfaces and
explicit compositor-owned UI may be visible. The compositor rejects a second
lock owner. New or resized outputs get an opaque cover before any normal frame;
the client then receives a new `configure`. Destroying a lock surface or losing
the client leaves the session locked and covered. Recovery starts another
client connected to the same Wayland session while the cover remains, or
requires ending the session from a separate VT. The dead client's
disappearance never counts as authentication.

Only the owning live lock object can send `unlock_and_destroy`, and only after
`locked`. The client waits for a `wl_display.sync` round trip before exiting.
The compositor removes covers and restores focus/input together. A lock client
must wait for each surface's first `configure`, acknowledge the serial, and
commit a buffer at the configured output-local size. Lock UI popups and other
ordinary surfaces do not get special treatment.

## Policy and system integration

The current Bingux lock client does not yet own `loginctl`, idle timeouts,
suspend handling, `org.gnome.ScreenSaver`, `org.freedesktop.ScreenSaver`, or
logind inhibitors. A user may run an idle-only tool such as hypridle alongside
a conforming lock client; that is a policy choice outside Gnoblin and does not
establish D-Bus or suspend compatibility.

Future Bingux policy may serialize manual, idle, logind, desktop-control, and
pre-suspend requests, expose compatibility APIs, and honour idle inhibitors.
If it implements pre-suspend locking, it must hold a logind **delay inhibitor**
until compositor presentation is confirmed. A timeout, process start, or visual
animation cannot substitute for the protocol's `locked` event.

GNOME Shell's `ScreenShield`, its D-Bus owner and its idle/sleep listeners are
removed **only in the Gnoblin session** after the replacement is verified.
The regular GNOME login continues using GNOME's lock screen. GDM,
`gnome-session` and settings-daemon are retained. Gnoblin's own bridge and
recovery controls must query the new lock state before exposing operations
that `Main.sessionMode.isLocked` currently protects.

## Shell cutover contract

GNOME Shell keeps its normal path whenever the compositor has not proved a
secure lock protocol. In the Gnoblin session, Shell reads Mutter's native
`get_gnoblin_session_lock_capability()` accessor at startup and skips
constructing `ScreenShield` only when it reports:

1. `CapabilityVersion >= 1`;
2. a server with compositor-enforced `unlocked`, `covering`, `locked`, and
   `failsafe` states is installed; and
3. all presentation, input isolation, client-death, and capture gates are
   secure.

The native capability defaults to zero while the protocol global is hidden. A
missing accessor or false result retains GNOME ScreenShield. The regular GNOME
session always retains its own ScreenShield.

The native manager will advertise the standard `ext-session-lock-v1` global to
any client in the same session. Bingux and compatible clients such as hyprlock
may acquire the same protocol role directly. The global remains hidden in the
current build until the compositor has passed the secure coverage, input
isolation, client-death, and presentation checks in this document. No runtime
compatibility claim is made before those tests pass.

The Wayland socket is the trust boundary for lock ownership. To support
ordinary third-party lockers without a Gnoblin-specific launch token, any
client already allowed on that socket may attempt a lock or take over after a
locker dies. A replacement can then unlock through the protocol; Gnoblin
cannot verify that client's password check. The lock protects against access
at the seat while the compositor enforces it, but does not isolate mutually
untrusted applications sharing one user's Wayland connection. Run untrusted
applications with separate Wayland socket access if that distinction matters.

The native seam supplies `get_gnoblin_session_lock_active()`, which is true
from `covering` through `failsafe`. Shell's bridge stops work as soon as Mutter
installs its input embargo rather than waiting for presentation confirmation.
Bingux may add policy, compatibility APIs, and logind integration later.
Gnoblin neither launches a locker nor owns session policy.

Gnoblin's bridge and developer console use the same adapter for their locked
state. It combines stock `sessionMode.isLocked` with the compositor's active
state, cancels bridge interaction when the compositor becomes active, and
refuses screenshots, previews and window operations while locked. A compositor
which is absent, too old, `covering`, or `unavailable` leaves GNOME's normal
ScreenShield untouched.

The Shell screenshot service applies that same predicate before creating a
`Shell.Screenshot`, opening screenshot or recording UI, interactive capture,
and area selection. It returns permission denied over
`org.gnome.Shell.Screenshot` from `covering` onward. This complements the
compositor's capture isolation and prevents Shell-owned screenshot paths from
leaking a frame during the handover.

The Shell suppresses GNOME's “Screen Lock disabled” warning only after that
authority check. Until Bingux explicitly integrates a Lock Screen action, the
Shell hides it in cutover rather than routing a lock request to an unspecified
native policy owner. Switch User is likewise unavailable in cutover because its
old path locks `ScreenShield`.

## Release gates

### Compositor protocol

Gnoblin may expose `ext-session-lock-v1` only after a fresh installed session
proves all of these paths:

1. A conforming client acquires one owner and repeated requests cannot create
   competing lock owners.
2. No app receives keyboard, pointer, touch, shortcut, clipboard or remote
   input while locked; capture and portal paths cannot reveal normal content.
3. Lock UI crash, forced kill, hang before its first buffer, output hotplug,
   scale/rotation changes and GPU reset stay opaque and locked.
4. Real multi-monitor hardware and a fresh session confirm the `locked` event
   follows presentation, and ordinary input/focus returns only on unlock.

### Optional Bingux policy

Manual shortcuts, `loginctl`, desktop controls, D-Bus compatibility, idle
timeouts and inhibition, and lock-before-suspend/hibernate/lid-close are
separate Bingux work. Each requires its own end-to-end evidence, including a
presentation-confirmed delay inhibitor before suspend. Authentication success,
failure, cancellation, and multi-prompt PAM or biometric conversations belong
to the selected lock client and must be tested before that client is promoted.

The previous GNOME lock remains the default while any gate is open. A nested
headless test can check protocol ordering but cannot prove display scanout,
hotplug, VT or resume behaviour.

## References

- [Wayland session-lock protocol and lifecycle](https://wayland.app/protocols/ext-session-lock-v1)
- [Hyprland session lock manager](https://github.com/hyprwm/Hyprland/blob/main/src/managers/SessionLockManager.cpp)
- [hyprlock client](https://github.com/hyprwm/hyprlock) and [hypridle policy](https://github.com/hyprwm/hypridle)
- [systemd logind session API](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.Session.html)
- [systemd inhibitor locks](https://github.com/systemd/systemd/blob/main/docs/INHIBITOR_LOCKS.md)
- [GNOME Shell `ScreenShield`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/js/ui/screenShield.js)

# Session locking design

Gnoblin is moving session locking from GNOME Shell's `ScreenShield` to a
compositor-enforced Wayland lock. This page records the security contract and
the rollout gates. **The running Gnoblin session still uses GNOME's lock screen
until the compositor and Bingux client pass the checks below.** A vendored
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

| Component | Owns |
| --- | --- |
| Gnoblin's Mutter fork | Lock state, opaque fallback on every output, input and capture isolation, lock surface placement, and the `ext-session-lock-v1` server |
| Gnoblin lock coordinator | Manual, idle, logind and sleep requests; timeout and inhibitor policy; launching one configured lock client; compatibility D-Bus APIs; reporting actual locked state |
| Bingux lock client | One lock surface per output, visual design, accessible prompts, authentication, and `unlock_and_destroy` after successful authentication |
| GDM and logind | Login/greeter and system session management; both remain installed |

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
trusted client while the cover remains, or requires ending the session from a
separate VT. The dead client's disappearance never counts as authentication.

Only the owning live lock object can send `unlock_and_destroy`, and only after
`locked`. The client waits for a `wl_display.sync` round trip before exiting.
The compositor removes covers and restores focus/input together. A lock client
must wait for each surface's first `configure`, acknowledge the serial, and
commit a buffer at the configured output-local size. Lock UI popups and other
ordinary surfaces do not get special treatment.

## Policy and system integration

The coordinator serializes `loginctl lock-session`, the lock shortcut, desktop
controls, idle timeout and pre-suspend lock into one request path. It preserves
the useful `org.gnome.ScreenSaver` and `org.freedesktop.ScreenSaver` methods for
applications and existing idle inhibitors. An idle inhibitor can defer an idle
lock; it cannot cancel an explicit manual or logind lock. `SetLockedHint` tells
logind the result, but does not itself lock anything.

For suspend, the coordinator holds a logind **delay inhibitor** before sleep is
requested, requests a lock on `PrepareForSleep(true)`, and releases that delay
only after secure presentation is confirmed. It renews the inhibitor after
wake. A timeout is a failure to report, not permission to expose the desktop.
This is why a fixed sleep, a process-start event, or a visual animation cannot
stand in for the protocol's `locked` event.

GNOME Shell's `ScreenShield`, its D-Bus owner and its idle/sleep listeners are
removed **only in the Gnoblin session** after the replacement is verified.
The regular GNOME login continues using GNOME's lock screen. GDM,
`gnome-session` and settings-daemon are retained. Gnoblin's own bridge and
recovery controls must query the new lock state before exposing operations
that `Main.sessionMode.isLocked` currently protects.

## Shell cutover contract

The first implementation keeps GNOME Shell's path by default. A developer can
set `GNOBLIN_SESSION_LOCK_CUTOVER=1` only for a fresh Gnoblin Shell process.
At startup, Shell reads Mutter's native
`get_gnoblin_session_lock_capability()` accessor. It does not synchronously
query D-Bus: Mutter and the Shell JavaScript share one process, so a startup
call into a service owned by that process can deadlock. Shell skips constructing
`ScreenShield` only when the native accessor reports all of the following:

1. `CapabilityVersion >= 1`;
2. a server with compositor-enforced `unlocked`, `covering`, `locked`, and
   `failsafe` states is installed; and
3. the coordinator already owns `org.gnome.ScreenSaver` and
   `org.gnome.Shell.ScreenShield` compatibility names.

The native seam also supplies `get_gnoblin_session_lock_active()` and
`request_gnoblin_session_lock(reason)`. Its future coordinator exposes `State`
(`unavailable`, `unlocked`, `covering`, `locked`, or `failsafe`), `Active`,
`StateChanged`, and `RequestLock(reason)` on `org.gnoblin.SessionLock` for
external clients. The native active accessor is true from `covering` through
`failsafe`, so Shell's bridge stops work as soon as Mutter installs its input
embargo rather than waiting for presentation confirmation.
`RequestLock` only confirms that acquisition began; it does not substitute for
the compositor presentation confirmation. The coordinator owns logind and the
two compatibility D-Bus names during cutover, while GNOME Shell owns all of
them during fallback. This single-owner rule prevents competing ScreenSaver
methods and sleep listeners.

Gnoblin's bridge and developer console use the same adapter for their locked
state. It combines stock `sessionMode.isLocked` with the coordinator's active
state, cancels bridge interaction when the compositor becomes active, and
refuses screenshots, previews and window operations while locked. A coordinator
which is absent, too old, `covering`, or `unavailable` leaves GNOME's normal
ScreenShield untouched.

## Release gates

The replacement is ready to become the default only when a fresh installed
Gnoblin session proves all of these paths:

1. Manual lock via shortcut, `loginctl`, desktop controls and D-Bus; repeated
   requests do not create competing lock owners.
2. Idle lock at configured timeout, idle inhibition, and lock before suspend,
   hibernate and lid-close without an unlocked frame on resume.
3. No app receives keyboard, pointer, touch, shortcut, clipboard or remote
   input while locked; capture and portal paths cannot reveal normal content.
4. Lock UI crash, forced kill, hang before its first buffer, output hotplug,
   scale/rotation changes and GPU reset stay opaque and locked.
5. Authentication success, failure, cancellation and the configured PAM/GDM
   conversation, including more than one prompt, behave correctly.
6. Real multi-monitor hardware and a fresh session confirm the `locked` event
   follows presentation, and ordinary input/focus returns only on unlock.

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

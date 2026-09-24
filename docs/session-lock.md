# Session locking

Gnoblin uses the standard `ext-session-lock-v1` Wayland protocol in the
Gnoblin session. The compositor owns the security boundary: it covers every
output, stops normal input, replaces desktop capture with its lock scene, and
stays locked if a locker exits. The locker owns its appearance and
authentication.

Gnoblin does not provide a default lock screen. Bingux has its own independent
lock client. Third-party lockers such as hyprlock, swaylock, gtklock and
waylock can connect directly to the same standard protocol; the first locker
to request a lock owns it, and concurrent lockers receive `finished`.

In Bingux, use its Lock action or run `bingux-lock`. With another locker
installed, run its command (for example, `hyprlock`) in your Gnoblin session.
Your shell or idle daemon decides when to run that command; Gnoblin does not
set a timeout or launch a locker for you.

The regular GNOME session is separate. It does not advertise this protocol and
continues to use GNOME ScreenShield.

## Portal and remote access while locked

Existing portal permission remains in force. A monitor stream already
authorised through the desktop portal receives the compositor lock scene while
the session is locked, rather than normal desktop content. Remote keyboard and
pointer input is available after `locked` and lock-scene presentation, and is
routed only to the active lock surface. It is refused during the transition and
failsafe states. This requires no lock-specific portal setting, opt-in, or
change to an existing authorised grant.

## Locker requirements

A locker must wait for `locked` before treating the session as secure. It must
create one lock surface for each output, acknowledge each configure, and commit
a buffer of the configured output-local size. After successful authentication,
only the owner calls `unlock_and_destroy`. A locker crash leaves the compositor
covered and locked; another client may acquire the lock afterwards.

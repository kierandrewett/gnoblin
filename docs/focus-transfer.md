# Application focus transfer

Gnoblin honours application activation requests. Activating a normal application
window raises it, restores it from minimisation, switches to its workspace, and
transfers keyboard focus. An old timestamp or a Wayland activation token without
an input serial does not turn the request into a demands-attention indicator.
Wayland activation requests without a recognised startup sequence also work.
New windows are not denied focus merely because input occurred during startup.
Explicit no-focus hints and the existing restrictions for non-focusable and
special-purpose windows remain in place.

The policy lives in Mutter and is enabled only when the effective
`GNOME_SHELL_SESSION_MODE` is `gnoblin`. Other session modes retain upstream focus
prevention. Rebuilding and installing Mutter requires a new compositor session;
reloading shell scripts cannot replace an already loaded native library.

Bingux also activates the matching app window before invoking a notification's
default action. This covers actions whose sender handles the notification without
presenting its window. The sender can subsequently select a more specific window.

## Verification

After building and installing the local prefix:

```sh
GNOBLIN_TEST_CLIENT="$PWD/scripts/test-focus-transfer.py" scripts/run-gnome-shell.sh
EXPECT_FOCUS_TRANSFER=0 GNOBLIN_TEST_CLIENT="$PWD/scripts/test-focus-transfer.py" scripts/run-gnome-shell.sh
```

The private compositor test restores a minimised Foot window on another workspace
using a stale activation timestamp and verifies actual keyboard input. A GTK
Wayland client requests an activation token without a seat/input serial and checks
keyboard delivery after activation. The negative run changes the private
compositor's policy selector to `gnome` and checks that both requests are denied;
it is a policy regression test, not a full GNOME session test.

The tests require Foot, Python, GTK 3 development files, a C compiler,
`wayland-scanner`, and `wayland-protocols`.

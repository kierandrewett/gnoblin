# Application activation

In Gnoblin, activating an application raises its window, restores it if minimised,
switches workspace and transfers keyboard focus.

Old timestamps and Wayland tokens without an input serial do not reduce the
request to an attention indicator. Explicit no-focus hints and restrictions on
special windows still apply.

This policy is native Mutter code and only applies in Gnoblin mode.
Stock GNOME keeps upstream focus prevention. Native changes need a new session.

## Notification actions

Bingux activates the matching app window before invoking a notification's default
action. The app can then select a more specific window.

## Test

After building the local prefix:

```sh
GNOBLIN_TEST_CLIENT="$PWD/tests/test-focus-transfer.py" scripts/run-gnome-shell.sh
EXPECT_FOCUS_TRANSFER=0 GNOBLIN_TEST_CLIENT="$PWD/tests/test-focus-transfer.py" scripts/run-gnome-shell.sh
```

The positive run checks stale timestamps and token-without-serial activation
using real keyboard input. The negative run changes the policy selector; it is
not a complete stock GNOME test.

Dependencies: Foot, Python, GTK 3 development files, C compiler,
`wayland-scanner` and `wayland-protocols`.

# Check startup and dock animations

Run these checks from the repository root against an existing disposable
QEMU run. Both require an installed Gnoblin session with Bingux and the
standard guest user prefixes described in [the QEMU guide](../../design/qemu-e2e-validation.md).

## Record fresh logins

```sh
python3 tests/qemu-e2e/session-startup.py RUN_DIRECTORY
```

The runner restarts GDM only inside that VM. It records five starts, ten
seconds each, with up to 20 captures per second. It saves timestamped frames,
guest journals and executable identities without classifying desktop readiness.

Use `--sample NAME:X:Y` to record a named pixel in each frame. Use
`--output DIRECTORY` to choose a new artifact directory. The directory must
not already exist.

Restart timing includes old-session shutdown. Compare the journals with frames
to identify session handoff and first visible surfaces. This autologin capture
does not measure time from a credential click; its sampling interval limits
the precision of visible-transition timings.

## Click the dock

```sh
python3 tests/qemu-e2e/bingux-dock-animations.py RUN_DIRECTORY
```

This check requires Foot, one monitor of at least 1000 by 700 pixels, and
Bingux's explicit minimise and restore animation rules. It clicks the installed
dock using QEMU input events to minimise and restore its own Foot window.

It checks the published icon rectangle, restored focus, and uncancelled
compositor animation events. It saves transition frames and JSON evidence,
then removes its window and event listener. Those frames show the transition;
they do not establish hardware frame pacing.

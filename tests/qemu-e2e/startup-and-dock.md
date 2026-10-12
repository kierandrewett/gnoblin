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

To also observe startup generations and public control IPC, add
`--observe-readiness`:

```sh
python3 tests/qemu-e2e/session-startup.py RUN_DIRECTORY --observe-readiness
```

For each cycle, the runner records pre-restart native compositor identities
whose command line includes `--gnoblin-runtime-fd=` and the unique Quickshell
process whose command line names the `/bingux/shell` configuration. A
background poll runs during frame capture and records guest monotonic process
start times, host request and response times, `gnoblinctl ping`, and
`binguxctl ipc shell status`.

The probe supplies the guest session bus and
runtime directory and searches the guest user's Gnoblin and Bingux executable
directories.

A cycle is marked `observed` only when one poll contains:

- Exactly one new native compositor and one new main shell generation.
- A successful `gnoblinctl ping` response (`pong`).
- Shell status JSON with boolean `controls`, `calendar`, and `notifications` fields.

The old compositor and shell generations must be absent from that poll.
A missing or ambiguous baseline, failed probes, or absent new generations is
recorded as `incomplete`.

These milestones show that new processes and their control interfaces respond.
They do not establish that any surface is displayed, that wallpaper changed
visibly, or that the desktop accepts user input. A wallpaper already retained
on screen is not observable as a new wallpaper appearance in this capture.
The default capture behavior is unchanged when `--observe-readiness` is not
specified.

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

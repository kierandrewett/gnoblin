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

## Dock first paint after a fresh login

```sh
python3 tests/qemu-e2e/dock-first-paint.py RUN_DIRECTORY --logins 5 --seconds 60
```

The check correlates the compositor's dock layer state with the displayed pixels.

Each login restarts the native compositor and Bingux shell, then samples the
session for `--seconds`. Every tick records:

- the layer surfaces from `gnoblinctl layer list`;
- the compositor and shell process identities;
- the dock layout from `binguxctl ipc shell layoutSnapshot`;
- a VNC frame taken at the same time.

After the window it runs `gnoblinctl config reload`, grabs a reference frame,
and compares the dock band to it.

Only ticks after the new session starts count. That is the later of the new
compositor and new shell start. The old dock stays on screen until its
compositor exits, so earlier ticks prove nothing about the new dock.

| Option | Default | Meaning |
| --- | --- | --- |
| `run` | required | Prepared QEMU run directory with a synced prefix. |
| `--logins` | `5` | Fresh GDM logins to observe. |
| `--seconds` | `60` | Observation window per login, in seconds. |
| `--interval` | `0.5` | Seconds between ticks. |
| `--grace` | `2.0` | Seconds a mapped dock may take to be drawn. |
| `--threshold` | `0.9` | Dock band match fraction that counts as drawn. |
| `--tolerance` | `24` | Per-channel pixel tolerance for the band match. |
| `--repaint-wait` | `2.0` | Seconds to wait after `config reload` before the reference frame. |
| `--output` | dated directory in RUN | New artifact directory. |

Outcomes per login:

- `drawn`: the dock matched within `--grace` seconds of mapping.
- `drawn-late`: the dock matched after more than `--grace` seconds. Reports
  `draw_latency` and does not fail the run.
- `invisible-mapped`: the dock is still mapped and undrawn at window end. Fails
  the run.
- `not-mapped`: the new dock never mapped. Fails the run.
- `incomplete`: the dock mapped but never matched the reference, or no new
  session start was observed. Fails the run.

Evidence goes to a dated directory: `summary.json`, per-login `ticks.json` and
`result.json`, the reference frame, and frames for flagged or sampled ticks.

QMP screendumps fail under `virtio-vga-gl`, so frames come from VNC. A fade-in
can keep a mapped dock undrawn for several seconds.

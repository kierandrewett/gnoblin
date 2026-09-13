# SSD/CSD regression verification — 13 September 2026

## Installed changes

- Client-side decorated windows keep CSD. The normal Lua config selects Bingux
  SSD only through `frame.mode = "auto"`, with extents `[36, 0, 0, 0]`
  and Bingux's `--compact` GTK decoration mode, per the user's height preference.
- The compositor border is stacked above the complete SSD root, including its
  titlebar. There is no extra side/bottom client-frame strip.
- Same-size external-renderer updates retain the presented buffer while awaiting
  the new serial. Focus/title changes no longer briefly reveal the native frame.
  Size changes, renderer failure and timeouts retain the native fallback.
- CSD alpha detection reads the client shaped texture into an independent
  framebuffer, not the decorated actor. It does not hide decorations or toggle
  active effects. Inconclusive startup probes retry on client damage, bounded
  to eight attempts; successful probes remain cached during movement/focus.
- Empty/unavailable shaped-texture snapshots and out-of-bounds image requests
  return a valid empty image instead of a null Cairo foreign pointer. The new
  byte-readback API reports unavailable buffers through a catchable GError.
- Script startup awaits async initializers and cleans up rejected initializers.
  Event dispatch contains rejected async handlers. Reload unwinds scripts and
  their disposers in reverse order, once.
- Unclean script sessions leave a recovery marker. The next session quarantines
  user scripts, keeps core controls available, and requires explicit
  `gnoblinctl reload` to retry. Quarantine persists until that retry.

This is crash-loop recovery, **not a sandbox for arbitrary native GJS calls**.
Scripts still execute in the compositor process and have access to native GI.
Full fault isolation would require an out-of-process scripting boundary.

## Normal-configuration test environment

The disposable session used copies of `~/.config/gnoblin`, `~/.config/bingux`
and the user's dconf database. It ran the full Bingux shell using the installed
Fedora-compatible `gnoblin-quickshell` launcher. Its D-Bus, compositor socket,
runtime/cache/data/state directories were separate from the host session.

The visible devkit stopped producing frames while its viewer was inactive
(`after-paint` count remained zero). Its stale screenshots were rejected. A
second isolated headless-output session with the same normal configuration was
used for the recorded pixel/input results. This was not a blank-rule fixture.

Snapshot root: `/tmp/gnoblin-user-config.HbXysO`.
Persistent proof images: `~/.local/state/gnoblin/proofs/2026-09-13-ssd/`.

## Results

| Check | Result |
| --- | --- |
| GTK startup and three script/config reload cycles | Four corner pixel checks passed every cycle |
| Ghostty CSD sampling | Insets `[9,9,9,9]`, CSD retained |
| RustDesk CSD sampling | Insets `[7,7,7,7]`, CSD retained |
| Real Spotify | Negotiated SSD; both native and Bingux titlebars rendered |
| Native right-click menu | Real pointer input opened Bingux menu; Minimize targeted the original window |
| Bingux right-click menu | Same end-to-end input/action check passed |
| Bingux focus transitions | 30 changes, zero native-fallback visibility transitions |
| RustDesk real drag | Zero differing pixels versus a full repaint |
| Ghostty real drag | 21 differing pixels, no stale window trail |
| Native readback failure cases | Catchable empty-buffer error; valid empty Cairo image; valid real client image; empty out-of-bounds image |
| Script native-crash recovery | After SIGABRT of only the disposable compositor, next process reported safe mode, zero loaded scripts, quarantine present, `ping = pong`, developer console open |
| Targeted Node regressions | Nine checks passed across script lifecycle, window-rule lifecycle, border edges and frame policy |

Drag verification captures the framebuffer **before swap**, using Mutter's
test-exported stage watcher. Reading it from Clutter's later `after-paint`
signal can capture the next backbuffer and is not a valid damage test. Panel
clock and dock animation regions were excluded from the image comparison.

The focus/menu/CSD checks were repeated after retiring the compatibility
scripts. The drag test ran without the temporary move-redraw script.

## Configuration cleanup

Active load order is `init.lua` → `bingux.lua` → Bingux integration plus
`settings.lua`. Bingux remains selected. Eight obsolete compatibility scripts
were moved, not deleted, to:

`~/.config/gnoblin/scripts/retired/2026-09-13-ssd/`

They are the CSD bridge, SSD border/focus bridges, move-damage workaround,
forced-clip bridge, old lifecycle bridge and two no-op border/shadow scripts.

## Verification entry points

Start with `bash scripts/run-normal-config-devkit.sh`. It refuses to start
without the user's normal `init.lua`, copies the configuration and prints the
snapshot path. For unattended capture use `GNOME_DEVKIT_HEADLESS=1` with the
same command. The unsafe Eval API is enabled only on this isolated test bus.

- `python3 tests/nested-desktop-check.py SNAPSHOT_ROOT`
- `python3 tests/nested-ssd-check.py SNAPSHOT_ROOT`
- `python3 tests/nested-drag-check.py SNAPSHOT_ROOT`
- `node --test tests/script-lifecycle.test.mjs tests/window-frame-policy.test.mjs tests/maximized-border-edges.test.mjs tests/window-rule-lifecycle.test.mjs`

The nested helpers reject a non-devkit display and scrub host X11/display
activation variables. Flatpak tests must additionally use `--nosocket=x11
--socket=wayland --env=GDK_BACKEND=wayland`; RustDesk's manifest otherwise
forces its X11 default. Only the accidentally host-launched test instance was
terminated; the user's pre-existing RustDesk instance was left untouched.

## Handoff boundary

Changes are built and installed in `/home/kieran/dev/gnoblin/install`.
The host is currently running Gnoblin. The live Bingux renderer was restarted
after the final painter change. The double-click compositor change is installed
but requires the next Gnoblin session/login because the running Mutter process
cannot reload its native frame event handler.

Similarity review found no duplicate JavaScript functions in the component
scope. Python matches were reviewed as intentionally parallel test fixtures;
no unrelated refactor was performed.

The interactive Bingux SSD regression also verifies that compositor hover over
a real control changes rendered pixels, and that two titlebar double-clicks
toggle maximize then restore. The earlier explanation that the hidden host had
no frame clock was incorrect. Keeping its widgets alive and servicing GTK's main
context restores GTK's CSS transitions. The custom hover tint and compact header
class have been removed.

Normal GTK comparison: `/tmp/gnoblin-user-config.9pfMZs/comparison` contains a
presented GTK4/libadwaita header alongside the actual SSD painter. Idle, hover,
pressed, pointer-leave, backdrop and restored-focus endpoints each differed by
zero bytes. The animated phases produced 10–13 changing SSD frames, matching
GTK's behavior; focus restoration was immediate in both for this theme.
`comparison.png` and `comparison.gif` show the captured headers side by side.

The normal-config nested transport check additionally passed actual compositor
hover-in, hover-out, header focus-color fade, double-click maximize/restore,
right-click menu targeting and 30 focus changes without native fallback flashes.
GTK CSD corner sampling/pixels survived three script/config reloads. The renderer
consumed zero CPU ticks during a two-second settled idle measurement.

After the user confirmed the animations but requested the shorter titlebar, the
desktop was returned to 36px using GTK's `default-decoration` class via
`--compact`. `/tmp/gnoblin-user-config.9pfMZs/compact-comparison` repeats the
normal-GTK reference comparison in that same compact mode: all six endpoints
still match byte-for-byte and animated phases still produce 11–12 changing
frames. The persistent widget tree and GTK event-loop integration are unchanged.

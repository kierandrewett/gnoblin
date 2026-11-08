# End-to-end: real toolkit clients against Gnoblin

Status: partial. Scale-1 runs are complete for foot, GTK4, Qt6 and weston-simple-shm. The scale-2 (HiDPI) runs did not apply the output scale. Their HiDPI results are invalid, not passes. Keyboard input is broken for every text client. Pointer input could not be exercised.

Logs on the workstation: `~/dev/lanes/ToolkitE2E-logs/tk/out/foot-s1/` (`results.tsv`, `session.log`, `client.log`, `probe.log`, PPMs). Run summaries are `/tmp/tk-runlog-<client>-s<scale>.txt` (not yet copied into the lane logs). Scale-2 session log: `/tmp/tk-foot-s2-session.log` (pulled from instance `3oqx4dtpud`, which is deleted at the end of the run).

## Setup

- Build: `/home/node/target/debug/gnoblin` on Agent37, from the lane source.
- Session: `GNOBLIN_CONFIG=<out>/init.lua GNOBLIN_LOG='gnoblin=debug,warn' gnoblin`, headless backend, 1024x768 output.
- Config: `gnoblin.configure{ monitors = { global_scale = N } }`.
- Window control: `gnoblinctl window {close,maximise,fullscreen,move,resize,minimise}`.
- Clients: `foot`, `weston-simple-shm`, PyQt6 probe (`qt6-probe.py`), GTK4 probe (`gtk4-probe.py`, python3-gi). Probes log size, scale, active state, close and exit to `TK_OUT`.
- Keyboard: `wtype` on the privileged socket. Pixels: `grim`, converted to PPM; region checks in `analyze.py`.
- Harness: `ToolkitE2E-logs/tk/run.sh CLIENT SCALE`.

## Results (scale 1)

PASS, FAIL, BLOCKED, INFO, INVALID. Evidence: per-run summary files and `results.tsv` on the instance.

| Check                         | foot        | GTK4        | Qt6         | shm         | Evidence and notes                                                                                                                                                              |
| ----------------------------- | ----------- | ----------- | ----------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| session starts                | PASS        | PASS        | PASS        | PASS        | socket up, `output scale=1.0`                                                                                                                                                   |
| (1) maps                      | PASS        | PASS        | PASS        | PASS        | `window list`: app, 800x600 at 0,0, focused                                                                                                                                     |
| (1) draws (not black)         | PASS        | PASS        | PASS        | PASS        | region black fraction 0.04 to 0.87; shm mostly black (weak)                                                                                                                     |
| (2) keyboard via wtype        | FAIL        | FAIL        | FAIL        | BLOCKED     | no client text change. foot's log confirms the keymap error (defect 1). GTK4/Qt6 show the same symptom (pixels changed 17 and 14) but their logs were not checked for the error |
| (3) pointer click and focus   | BLOCKED     | BLOCKED     | BLOCKED     | BLOCKED     | no virtual-pointer protocol on the server; EIS needs a portal consent grant (`crates/gnoblin-compositor/src/eis.rs`)                                                            |
| (4) scroll                    | BLOCKED     | BLOCKED     | BLOCKED     | BLOCKED     | same as pointer                                                                                                                                                                 |
| (5) resize via verb           | PASS        | PASS        | PASS        | PASS        | `window now 500x400`; GTK4 client reported 500x400                                                                                                                              |
| (5) resize: client re-renders | PASS        | PASS        | FAIL        | BLOCKED     | Qt reported 494x367 for a 500x400 window. Probably its own client-side frame (6px sides, 33px title). Not checked against pixels; the check is too strict                       |
| (5) maximise                  | PASS        | PASS        | PASS        | PASS        | `window now 1024x768`; client re-renders 1018x735 then 1024x768                                                                                                                 |
| (5) fullscreen                | PASS (weak) | PASS (weak) | PASS (weak) | PASS (weak) | size only: the window list does not expose fullscreen state                                                                                                                     |
| (5) move                      | PASS        | PASS        | PASS        | PASS        | window at 120,90                                                                                                                                                                |
| (5) minimise                  | PASS (weak) | PASS (weak) | PASS (weak) | FAIL        | pixel change only: no minimised flag in the window list. shm changed 62500 of 200000 pixels, possibly its animation                                                             |
| (6) close                     | PASS        | PASS        | PASS        | PASS        | window gone from list; client exit rc=0                                                                                                                                         |
| (7) decoration                | INFO        | INFO        | INFO        | INFO        | compositor always answers ClientSide (`zxdg_decoration_v1.rs:43`). GTK4 and Qt6 report decorated=True                                                                           |
| (8) cursor shape              | BLOCKED     | BLOCKED     | BLOCKED     | BLOCKED     | `wp_cursor_shape_v1` recorded, never drawn (`crates/gnoblin-wayland/src/lib.rs:116`); no pointer to test with                                                                   |
| (9) HiDPI                     | INVALID     | INVALID     | INVALID     | n/a         | output stayed at scale 1.0 (below)                                                                                                                                              |
| session stop, panics          | PASS        | PASS        | PASS        | PASS        | 0 panic lines                                                                                                                                                                   |

## Results (scale 2)

INVALID. `output-scale 1.0 != requested 2` in all three runs (foot-s2, gtk4-s2, qt6-s2). No HiDPI result for Gnoblin can be claimed.

Cause, from `/tmp/tk-foot-s2-session.log`:

- The state directory persisted between runs. Every start logged `configuration not readable: the saved configuration was accepted for /home/node/tk/out/shm-s1/init.lua, not for this config path`. Every subsystem then fell back to defaults, including `monitors`.
- The scale-2 document was committed only after startup (`configuration committed ... path=/home/node/tk/out/foot-s2/init.lua revision=1`). The monitor settings had already been read at startup from the store (`crates/gnoblin-compositor/src/monitors.rs:24`), so `global_scale` was not applied to the output.
- Two issues. The harness should use a fresh state directory per run (this is a harness defect; not re-run). Separately, a committed `monitors.global_scale` does not reach the output until restart; this is a compositor finding to verify.

## Defects and findings

1. **Keyboard: virtual-keyboard keymap compile failed for wtype (fixed).** wtype sends a keymap of 449 bytes of `xkb_v1` text with no NUL, leaves the descriptor's file position at EOF, and sends a size one larger (the counted NUL). The old reader read from the current position and got 0 bytes, so the keymap was refused as shorter than its size and every key failed with `no_keymap`. Fixed in Gnoblin, not smithay: `crates/gnoblin-wayland/src/protocols/zwp_virtual_keyboard_v1.rs` reads from offset 0 and cuts at the first NUL (commits `af78308b2`, `3bf7789c6`). Unit tests cover the NUL case; `a_keymap_sent_the_way_wtype_sends_it_is_accepted` covers the wtype case. Real run on a Debian 12 instance (foot 1.13.1, wtype 0.4, grim) after the fix: `wtype` exited 0, foot received the typed text, the session logged no keymap warning and no panic, and the two grim captures differ (1206 bytes). Gnoblin still pins smithay at `65aa86fa` (`Cargo.toml:201`); no smithay patch is carried.
2. **Pointer: no test route.** No virtual-pointer protocol on the server; EIS needs a portal consent grant. Pointer click, scroll and cursor-shape checks cannot run from a harness.
3. **Cursor shape recorded, never drawn** (`crates/gnoblin-wayland/src/lib.rs:116`).
4. **Committed monitor scale does not apply until restart** (see scale 2). Needs verifying with a fresh-state run.
5. **Minimise and fullscreen have no observable state** in the window list, so both are checked by pixel change or size only.

## Not done

- Keyboard for GTK4 and Qt6: confirm the keymap error in their session logs.
- Scale-2 runs: redo with a fresh state directory per run.
- Qt resize: add a `frameGeometry()` probe and compare against pixels.
- No pixel-level HiDPI sharpness check.

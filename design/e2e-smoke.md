# End-to-end smoke: Rust compositor as real processes

Status: first run of the `rust-compositor` binaries as real processes. Earlier
evidence was unit and harness tests only.

Logs are on the workstation in `~/dev/lanes/BootSmoke-logs/`:

- `run3/`: one clean sequence with one guardian (`start.txt`, `ctl.txt`,
  `clients.txt`, `kill-runtime.txt`, `sigterm.txt`, `kill-compositor.txt`,
  `session.stdout`, `cleanup.txt`).
- `run4/`: the same checks split per step, with each step's session log
  (`session-after-rt-kill.stdout`, `session-after-sigterm.stdout`,
  `session-after-comp-kill.stdout`), plus `capture-probe.txt`, `grim.txt` and
  `instance-config.txt`.

Both runs used Agent37 instance `bqrqo7h8ag`. It is deleted at the end of the
run.

## Setup

- Build: `cargo build --workspace --locked`, debug, on Agent37 through
  `~/dev/lanes/a37-gate.sh`. The running binary is `/home/node/target/debug/gnoblin`
  (`run4/kill-runtime.txt` process list).
- Session: the guardian `gnoblin` with the headless backend. It starts the
  children `gnoblin --role compositor`, `--role services` and `--role runtime`
  (`run4/kill-runtime.txt`).
- Runtime dir: `/home/node/smoke/run` (`ipc.sock`, `gnoblin-wayland.sock`).
- Config: `/home/node/smoke/init.lua` (`run4/instance-config.txt`). It sets
  `accessibility.magnifier = true`, `magnifier-zoom = 2`, and a window rule
  `maximise = true` matching app_id `^smoke%-foot$`.
- Clients: `weston-simple-shm` and `foot --app-id=smoke-foot` (`run3/clients.txt`).

## Test machine prerequisites

The real-process test suites need no display, seat or GPU. The system program the guardian and the D-Bus
suites start is `dbus-daemon`; the Agent37 `gnoblin-rust` template (Debian 12) has it, so the template needs no new
package. On another Debian or Ubuntu machine:

```
apt-get install dbus-daemon
```

- `dbus-daemon` (Fedora: `dbus-daemon`): the guardian suite (`crates/gnoblin/tests/guardian.rs`) and the
  `gnoblin-compat-dbus-*` suites start a private session bus per test and point the session at it with
  `DBUS_SESSION_BUS_ADDRESS`; nothing reads the ambient bus. The guardian suite fails at once with a message
  naming the package when the program is missing.
- Present on the template and used by the build and other suites: `xwayland`, `fontconfig` and `pipewire`,
  with `libpipewire-0.3-dev` and `libfontconfig-dev` to build.
- Not needed: `libei`/`liboeffis` (the EIS tests use the `reis` crate in-process), a seat, `udev` rules,
  portal packages (the portal under test is the workspace's own `xdg-desktop-portal-gnoblin`), or a
  real `XDG_RUNTIME_DIR` (each test makes its own).

Two failures were waved through as environment gaps for a long time and were not: the guardian tests
(gnoblin-iv5p) started `xdg-desktop-portal-gnoblin` whenever a build had left it beside `gnoblin`, and gave
it no bus (the test's own `XDG_RUNTIME_DIR` held none), so the portal failed three restarts and the session
never reached `session ready`; they now start their own bus. The EIS pointer tests (gnoblin-7x2q) aimed at the
centre of a window's configured 800x600 geometry, but the test client commits a 200x150 buffer, and since
pointer picking went through the scene hit test (`2e68c8e2c`) only drawn content takes the pointer; they now
aim at the drawn rectangle.

## Results

| #   | Check                                                         | Result                | Evidence                                                                              | Notes                                                                                                                                                                                                                                                                                               |
| --- | ------------------------------------------------------------- | --------------------- | ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Session starts; guardian reports ready                        | PASS                  | `run3/session.stdout`                                                                 | `configuration revision 1 accepted`, `session ready`                                                                                                                                                                                                                                                |
| 2   | `gnoblinctl ping`                                             | PASS                  | `run3/ctl.txt`                                                                        | `pong`, exit 0                                                                                                                                                                                                                                                                                      |
| 3   | `gnoblinctl status`                                           | PASS                  | `run3/ctl.txt`                                                                        | `state running`, `revision 25`, `lock unlocked`, exit 0                                                                                                                                                                                                                                             |
| 4   | `gnoblinctl version`                                          | PASS                  | `run3/ctl.txt`                                                                        | `gnoblin 0.2.0`, `ipc 0.2.0`, exit 0                                                                                                                                                                                                                                                                |
| 5   | `gnoblinctl capabilities`                                     | PASS                  | `run3/ctl.txt`                                                                        | lists six capabilities; all `no` for the CLI connection                                                                                                                                                                                                                                             |
| 6   | `config validate` on JSON                                     | PASS                  | `run3/ctl.txt`                                                                        | file `{}`; `valid`, exit 0                                                                                                                                                                                                                                                                          |
| 7   | `config validate` on `init.lua`                               | PASS (run6)           | `run6/run6.txt`; bad file fails with a Lua parse error                                |
| 8   | `config get` and `config reload` from the CLI                 | FAIL (denied)         | `run3/ctl.txt`, `run4/`                                                               | `denied: config.current needs the commit-config capability, which this connection does not hold`. Denial at `crates/gnoblin-compositor/src/ipc/server.rs:803`. The CLI holds no `commit-config`, so this is by design (F1). Reload was not run with a capability that could commit                  |
| 9   | Real Wayland clients map                                      | PASS                  | `run3/clients.txt`                                                                    | `weston-simple-shm` at 0,0 800×600, focused; `foot` at 224,168 800×600                                                                                                                                                                                                                              |
| 10  | `gnoblinctl window list` shows the clients                    | PASS                  | `run3/ctl.txt`, `run3/clients.txt`, `run4/kill-runtime.txt`                           | both windows listed with app_id, geometry and focus                                                                                                                                                                                                                                                 |
| 11  | `gnoblinctl output list` shows the output                     | PASS (run6)           | `run6/run6.txt` (`HEADLESS-1`, 1024x768@60)                                           |
| 12  | Output is published to the object graph                       | PASS (run6)           | `run6/run6.txt`: 0 refusals in `session.stdout` after F3                              |
| 13  | Window rule `maximise` for `smoke-foot` takes effect          | PASS (run6)           | `run6/run6.txt`: foot 1024x768 at 0,0 (was 800x600 at 224,168)                        |
| 14  | Runtime role restarts when killed                             | PASS                  | `run4/kill-runtime.txt`, `run4/session-after-rt-kill.stdout`                          | `restarting runtime in 250 ms (attempt 1 of 5)`, `started runtime (generation 2, pid 16617)`, `configuration revision 2 accepted`. `ping` after the restart: `pong`, exit 0                                                                                                                         |
| 15  | Compositor role SIGKILL ends the session                      | PASS (matches design) | `run4/kill-compositor.txt`, `run4/session-after-comp-kill.stdout`                     | `compositor exited (signal SIGKILL); the session cannot continue`, then `session ended: compositor-lost`, `every child stopped`. The guardian does not restart it. `ping` then fails with `cannot-connect`, exit 3. Matches `design/rust-compositor.md:176` ("A crash ends the session")            |
| 16  | SIGTERM to the guardian exits cleanly                         | PASS                  | `run4/sigterm.txt`, `run4/session-after-sigterm.stdout`                               | `elapsed=3s`, `EXITED`, `stray=0`. Log: `session ended: signal`, `every child stopped`                                                                                                                                                                                                              |
| 17  | Screenshot of the headless output to PNG (privileged socket)  | PASS                  | `r3-logs.tgz`: `logs/B/grim-priv.stdout`, `logs/B/priv.ppm`, `logs/B/hash.txt`        | `grim` on `gnoblin-wayland-privileged.sock` exits 0, 1024x768 PPM. Pixels: 30000 `ff0000` (red client, 200x150), 30000 `808080` (masked secret client), rest `000000`. Same hash in `logs/A`                                                                                                        |
| 18  | Magnifier zoom changes the captured frame (PNG hash compare)  | FAIL (no change)      | `logs/A/hash.txt` and `logs/B/hash.txt` both `8438d98e…`                              | B has `magnifier = true`, `magnifier-zoom = 2` (`init-B.lua`); its capture is byte-identical to A (no magnifier). Either capture bypasses the output view (`apply_visual_accessibility`, `crates/gnoblin-wayland/src/state.rs:3045`) or the view is never set on this headless output. Not resolved |
| 19  | Accessibility config takes effect                             | PARTIAL               | `r3-logs.tgz`: `logs/B/session.stdout`; row 18                                        | config accepted; the pixel effect is not visible in captures (row 18)                                                                                                                                                                                                                               |
| 20  | Ordinary socket grim is refused                               | PASS                  | `logs/B/grim-ord.stdout`, `logs/A/grim-ord.stdout`, `logs/C/grim-ord.stdout`          | `compositor doesn't support wlr-screencopy-unstable-v1`, exit 1 on all three runs                                                                                                                                                                                                                   |
| 21  | `wayland-info` on both sockets                                | PASS                  | `logs/B/wayland-info-ordinary.txt`, `logs/B/wayland-info-privileged.txt`              | ordinary: 40 interfaces; privileged: 54. Every ordinary interface is also on the privileged socket. Privileged adds `zwlr_layer_shell_v1`, `ext_foreign_toplevel_list_v1`, `zwlr_data_control_manager_v1` and others                                                                                |
| 22  | `wf-recorder` on the privileged socket records                | PASS                  | `logs/B/wf-priv.stdout`, `logs/B/wf-priv.debug`, `logs/B/rec-priv.mp4`                | uses `zwlr_screencopy` on the privileged socket; 8066-byte mp4; `selected region 0,0 0x0` in its log; stopped by timeout SIGINT (exit 124)                                                                                                                                                          |
| 23  | `wf-recorder` on the ordinary socket is refused               | PASS                  | `logs/B/wf-ord.stdout`                                                                | `compositor doesn't support wlr-screencopy-unstable-v1`, exit 1                                                                                                                                                                                                                                     |
| 24  | `ext-image-copy-capture` client                               | BLOCKED               | none                                                                                  | no client on the instance speaks `ext_image_copy_capture`; `wf-recorder` uses screencopy only                                                                                                                                                                                                       |
| 25  | `secure = true` window rule masks the client in captures      | PASS                  | `logs/A/analyze.txt`, `logs/C/analyze.txt`, `logs/A/hash.txt`, `logs/C/hash.txt`      | with the rule (A, B) the secret client's area is `808080`; with no rule (C, `init-C.lua` empty) it is `0000ff`, unmasked. The red client is `ff0000` in all three runs                                                                                                                              |
| 26  | Session lock: capture shows only the cover                    | PARTIAL (fail closed) | `logs/A/status-locked.txt` (`lock locked`, revision 30), `logs/A/grim-lock.stdout`    | privileged `grim` during the lock is refused (`failed to copy output HEADLESS-1`), so no client pixels leak. No lock-cover client was run, so the cover itself is untested. `logs/A/lock-list.txt` says `gnoblin.lock locked=false` while status says locked; unexplained                           |
| 27  | Unresponsive flag on a client that ignores `xdg_wm_base` ping | PASS (after rebuild)  | `logs/P/ping-timeline.txt`, `logs/P/client-nopong.stdout`, `logs/P/client-shm.stdout` | `capclient-nopong` (`CAPCLIENT_NO_PONG`) shows `unresponsive=true` by the third sample. `weston-simple-shm` (answers pings) stays `unresponsive=false` in every sample. The first P run was invalid: the instance binary lacked `CAPCLIENT_NO_PONG`, so nothing could be flagged                    |

## Findings

- **F1 (fixed)** `config get` and `config reload` used to need `commit-config`,
  which the CLI connection does not hold, so the documented edit-then-reload flow
  was denied. Fixed by splitting the authority: `read-config` and `reload-config`
  are granted to the session owner's socket connection (peer uid from
  `SO_PEERCRED`), and neither implies `commit-config`, which stays with the
  `runtime` role. See `design/rust-compositor.md`, "Capabilities and who holds
  them". The row 8 denial above is from before this change.
- **F2 (fixed, 11ff3b74f)** `gnoblinctl config validate FILE` parsed JSON only.
  `validate_lua` in `crates/gnoblinctl/src/verbs.rs` now evaluates `.lua` through
  the runtime's Lua host, the same way the session loads `init.lua`. JSON is
  unchanged. Run6: good `init.lua` validates, a truncated one fails with a Lua error.
- **F3 (fixed, d15dbf046)** The output object is refused in every session snapshot
  (8 refusals in `run3/session.stdout`, 25 in `run4/session-after-rt-kill.stdout`).
  `gnoblin.output` requires `make` and the other mode and geometry properties.
  `session.rs:598-604` publishes only `name` and `layout`, and `ObservedOutput`
  (`observation.rs:195`) has no such fields. Effects: `gnoblinctl output list` is
  empty, and the window rule's maximise has no output to size against
  (`wm.rs:1240-1248`). Fixed: `ObservedOutput` carries every declared property,
  `session.rs` publishes the full set, and `tests/output-object-publish.py` checks
  that a published output passes `graph.rs` validation. Run6: 0 refusals, output listed.
- **F4 (fixed, 847c585be)** Cursor provider probes log `default/index.theme: unreadable: Permission
denied (os error 13)` for every shape: 231 lines in `run3/session.stdout`, 915
  in `run4/session-after-rt-kill.stdout`. Source `crates/gnoblin-cursor/src/chain.rs:112`.
  Fixed: a failing provider is logged once per (provider, path) and the chain falls
  through to the next provider. Run6: 0 unreadable lines. The denial itself is an
  instance HOME permission issue.
- **F5 (fixed, 47a919682, f1d5657da, 8f9c67da8)** First boot with no `last-good-config.json` logs
  `configuration not readable ... could not inspect ...: No such file or directory`
  (14 lines in `run3/session.stdout`). The source is 15 call sites that log at
  WARN, for example `crates/gnoblin-compositor/src/cursor.rs:28`,
  `crates/gnoblin-compositor/src/bell.rs:26`, `crates/gnoblin-compositor/src/accessibility.rs:39`,
  `crates/compat/gnoblin-compat-core/src/settings.rs:258`. The I/O error is produced
  at `crates/gnoblin-config/src/last_good.rs:118` (`load` at line 114). A missing
  file is a normal first-boot state. Fixed: `NotFound` logs at debug once; real read
  errors keep WARN. Run6: 0 `not readable` warnings.
- **F6** Xwayland was not started on the instance. Environment, not a code bug.
- **F8 (partly fixed, r3)** Screenshots and recording work on the privileged socket
  (rows 17, 22). The ordinary socket refuses `wlr-screencopy` (rows 20, 23), as
  intended. Still open: the magnifier does not show in captures (row 18). Either the
  capture path bypasses the output view or the view is not set headless. The
  `ext-image-copy-capture` path is untested (row 24). Owner: portal and capture lane.

## Services stall (fixed)

Symptom: the guardian killed the services child after 10 s with "did not report
ready". The services log showed the keyring, displayconfig and idle agents
starting, then nothing.

Root cause: the idle monitor adapter ran `block_on(watch_callers(..))` inside
`start()`. `watch_callers` returns only when its bus connection ends, so
`start()` never returned and services never reported ready.

Fix: the caller watch runs on its own thread, as the other adapters already do
(`crates/compat/gnoblin-compat-dbus-mutter-idlemonitor/src/agent.rs`, commit
add424516). `start()` returns once the name is exported and the counter is
published.

Proof: the keyring rerun in `design/e2e-keyring.md` reached `session ready` in
every scenario. In `~/dev/lanes/KeyringE2E-logs/rerun/off-first/session.log` the
services role starts at line 12 and `session ready` is at line 28.

## Not covered

- Real DRM and KMS output. Only the headless backend ran.
- GPU rendering and the GBM and EGL paths.
- PAM and the real login path.
- Real seat, logind and VT switching.
- Magnifier pixel verification (row 18 fails, F8). Cover content of the session lock (row 26).
  `ext-image-copy-capture` (row 24).
- Xwayland (F6).

## Xwayland 23.2 on an Agent37 instance

The `gnoblin-rust` template is Debian bookworm, whose Xwayland is 22.1.9. That server does not bind
`xwayland-shell-v1`: it names a window's surface by a Wayland object id that Smithay does not let a compositor
pair with the window, so an X11 window never gets content and the tests that need a real X11 window node skip
with a printed reason (`an_x11_window_node_reaches_the_shell_like_a_wayland_window`, and the window-request
tests in `crates/gnoblin-xwayland/tests/real_xwayland.rs`). Bookworm-backports has no `xwayland`, and trixie's
24.1.6 needs glibc 2.38, which bookworm lacks. Building 23.2.7 from source needs one newer package, `xorgproto`
(2024.1) for `presentproto >= 1.3`, and installs under `/opt` so the system server is left alone. As root on the
instance (`a37 exec ID --root -- ...`; a37 drops a leading `cd`, so put the commands in a script):

```
# deb-src for bookworm (the signed-by line avoids "conflicting values set for option Signed-By")
echo "deb-src [signed-by=/usr/share/keyrings/debian-archive-keyring.gpg] http://deb.debian.org/debian bookworm main" \
    > /etc/apt/sources.list.d/src.list
apt-get update && DEBIAN_FRONTEND=noninteractive apt-get build-dep -y xwayland
apt-get install -y meson ninja-build xz-utils curl
mkdir -p /opt/xwsrc && cd /opt/xwsrc
curl -sSfLO https://www.x.org/releases/individual/proto/xorgproto-2024.1.tar.xz
tar xf xorgproto-2024.1.tar.xz && (cd xorgproto-2024.1 && meson setup build --prefix=/opt/xwayland-23.2 && ninja -C build install)
curl -sSfLO https://www.x.org/releases/individual/xserver/xwayland-23.2.7.tar.xz
tar xf xwayland-23.2.7.tar.xz && cd xwayland-23.2.7
PKG_CONFIG_PATH=/opt/xwayland-23.2/share/pkgconfig meson setup build --prefix=/opt/xwayland-23.2 \
    -Dxvfb=false -Dxdmcp=false -Dsecure-rpc=false -Ddocs=false
ninja -C build -j4 && ninja -C build install
/opt/xwayland-23.2/bin/Xwayland -version        # 23.2.7
```

The build takes about 35 seconds on a 4-CPU instance. The tests find the server on `PATH`, so put it first for the
run:

```
a37-gate.sh in ID WORKTREE env PATH=/opt/xwayland-23.2/bin:/usr/local/cargo/bin:/usr/bin:/bin \
    cargo test -p gnoblin-xwayland --test real_xwayland
```

Proof, instance `yshe8wpmsn` (lane X11States): all 11 tests in `real_xwayland.rs` pass with no skip line, among them
`an_x11_clients_own_state_requests_are_answered_by_the_shell`. The instance is deleted at the end of a lane, so
the recipe has to be run again on the next one.

## Reproduce

```
gnoblin                                   # guardian, headless backend, XDG_RUNTIME_DIR=/home/node/smoke/run
gnoblinctl ping
gnoblinctl status
gnoblinctl version
gnoblinctl capabilities
gnoblinctl window list
gnoblinctl output list                    # empty, F3
gnoblinctl config validate valid.json     # valid
gnoblinctl config validate init.lua       # fails, F2
gnoblinctl config reload                  # denied, F1
kill -KILL <runtime pid>                  # guardian restarts runtime (row 14)
kill -KILL <compositor pid>               # session ends (row 15)
kill -TERM <guardian pid>                 # session exits (row 16)
WAYLAND_DISPLAY=gnoblin-wayland-privileged.sock grim -t ppm priv.ppm   # works (row 17)
WAYLAND_DISPLAY=gnoblin-wayland.sock grim shot.ppm                     # refused (row 20)
```

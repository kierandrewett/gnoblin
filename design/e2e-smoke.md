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

## Results

| #   | Check                                                        | Result                | Evidence                                                                   | Notes                                                                                                                                                                                                                                                                                    |
| --- | ------------------------------------------------------------ | --------------------- | -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Session starts; guardian reports ready                       | PASS                  | `run3/session.stdout`                                                      | `configuration revision 1 accepted`, `session ready`                                                                                                                                                                                                                                     |
| 2   | `gnoblinctl ping`                                            | PASS                  | `run3/ctl.txt`                                                             | `pong`, exit 0                                                                                                                                                                                                                                                                           |
| 3   | `gnoblinctl status`                                          | PASS                  | `run3/ctl.txt`                                                             | `state running`, `revision 25`, `lock unlocked`, exit 0                                                                                                                                                                                                                                  |
| 4   | `gnoblinctl version`                                         | PASS                  | `run3/ctl.txt`                                                             | `gnoblin 0.2.0`, `ipc 0.2.0`, exit 0                                                                                                                                                                                                                                                     |
| 5   | `gnoblinctl capabilities`                                    | PASS                  | `run3/ctl.txt`                                                             | lists six capabilities; all `no` for the CLI connection                                                                                                                                                                                                                                  |
| 6   | `config validate` on JSON                                    | PASS                  | `run3/ctl.txt`                                                             | file `{}`; `valid`, exit 0                                                                                                                                                                                                                                                               |
| 7   | `config validate` on `init.lua`                              | PASS (run6)           | `run6/run6.txt`; bad file fails with a Lua parse error                     |
| 8   | `config get` and `config reload` from the CLI                | FAIL (denied)         | `run3/ctl.txt`, `run4/`                                                    | `denied: config.current needs the commit-config capability, which this connection does not hold`. Denial at `crates/gnoblin-compositor/src/ipc/server.rs:803`. The CLI holds no `commit-config`, so this is by design (F1). Reload was not run with a capability that could commit       |
| 9   | Real Wayland clients map                                     | PASS                  | `run3/clients.txt`                                                         | `weston-simple-shm` at 0,0 800×600, focused; `foot` at 224,168 800×600                                                                                                                                                                                                                   |
| 10  | `gnoblinctl window list` shows the clients                   | PASS                  | `run3/ctl.txt`, `run3/clients.txt`, `run4/kill-runtime.txt`                | both windows listed with app_id, geometry and focus                                                                                                                                                                                                                                      |
| 11  | `gnoblinctl output list` shows the output                    | PASS (run6)           | `run6/run6.txt` (`HEADLESS-1`, 1024x768@60)                                |
| 12  | Output is published to the object graph                      | PASS (run6)           | `run6/run6.txt`: 0 refusals in `session.stdout` after F3                   |
| 13  | Window rule `maximise` for `smoke-foot` takes effect         | PASS (run6)           | `run6/run6.txt`: foot 1024x768 at 0,0 (was 800x600 at 224,168)             |
| 14  | Runtime role restarts when killed                            | PASS                  | `run4/kill-runtime.txt`, `run4/session-after-rt-kill.stdout`               | `restarting runtime in 250 ms (attempt 1 of 5)`, `started runtime (generation 2, pid 16617)`, `configuration revision 2 accepted`. `ping` after the restart: `pong`, exit 0                                                                                                              |
| 15  | Compositor role SIGKILL ends the session                     | PASS (matches design) | `run4/kill-compositor.txt`, `run4/session-after-comp-kill.stdout`          | `compositor exited (signal SIGKILL); the session cannot continue`, then `session ended: compositor-lost`, `every child stopped`. The guardian does not restart it. `ping` then fails with `cannot-connect`, exit 3. Matches `design/rust-compositor.md:176` ("A crash ends the session") |
| 16  | SIGTERM to the guardian exits cleanly                        | PASS                  | `run4/sigterm.txt`, `run4/session-after-sigterm.stdout`                    | `elapsed=3s`, `EXITED`, `stray=0`. Log: `session ended: signal`, `every child stopped`                                                                                                                                                                                                   |
| 17  | Screenshot of the headless output to PNG                     | FAIL (blocked, F8)    | `run6/run6.txt`: `wlr-screencopy` unsupported; `capture list` empty        |
| 18  | Magnifier zoom changes the captured frame (PNG hash compare) | NOT RUN (blocked, F8) | depends on 17                                                              |
| 19  | Accessibility config takes effect                            | PARTIAL               | `run6/run6.txt`: init.lua accepted (magnifier keys); pixel effect needs F8 |

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
- **F8 (open)** No screenshot path exists (row 17). Run6: `gnoblinctl capture list`
  is empty and the compositor reports `doesn't support wlr-screencopy-unstable-v1`.
  Recorded as a gap in the parity docs; checks 17 and 18 are skipped until it lands. Owner: portal and capture lane.
  Fix: implement the portal `Screenshot` against `Backend::capture`, and expose a
  capture or screencopy path that a client can call. Either implement
  `zwlr_screencopy` or the image-copy-capture protocol, or extend the capture
  schema.

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
- Screenshot and magnifier pixel verification (blocked by F8).
- Xwayland (F6).

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
grim shot.png                             # fails, no wlr-screencopy (row 17)
```

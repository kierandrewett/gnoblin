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

| #   | Check                                                        | Result                | Evidence                                                                              | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --- | ------------------------------------------------------------ | --------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Session starts; guardian reports ready                       | PASS                  | `run3/session.stdout`                                                                 | `configuration revision 1 accepted`, `session ready`                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| 2   | `gnoblinctl ping`                                            | PASS                  | `run3/ctl.txt`                                                                        | `pong`, exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| 3   | `gnoblinctl status`                                          | PASS                  | `run3/ctl.txt`                                                                        | `state running`, `revision 25`, `lock unlocked`, exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| 4   | `gnoblinctl version`                                         | PASS                  | `run3/ctl.txt`                                                                        | `gnoblin 0.2.0`, `ipc 0.2.0`, exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| 5   | `gnoblinctl capabilities`                                    | PASS                  | `run3/ctl.txt`                                                                        | lists six capabilities; all `no` for the CLI connection                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| 6   | `config validate` on JSON                                    | PASS                  | `run3/ctl.txt`                                                                        | file `{}`; `valid`, exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| 7   | `config validate` on `init.lua`                              | FAIL                  | `run3/ctl.txt`                                                                        | `invalid-config: invalid number at line 1 column 2`, exit 1. Entry point `crates/gnoblinctl/src/verbs.rs:255` (`validate`). The file is parsed as JSON (F2)                                                                                                                                                                                                                                                                                                                                                            |
| 8   | `config get` and `config reload` from the CLI                | FAIL (denied)         | `run3/ctl.txt`, `run4/`                                                               | `denied: config.current needs the commit-config capability, which this connection does not hold`. Denial at `crates/gnoblin-compositor/src/ipc/server.rs:803`. The CLI holds no `commit-config`, so this is by design (F1). Reload was not run with a capability that could commit                                                                                                                                                                                                                                     |
| 9   | Real Wayland clients map                                     | PASS                  | `run3/clients.txt`                                                                    | `weston-simple-shm` at 0,0 800×600, focused; `foot` at 224,168 800×600                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 10  | `gnoblinctl window list` shows the clients                   | PASS                  | `run3/ctl.txt`, `run3/clients.txt`, `run4/kill-runtime.txt`                           | both windows listed with app_id, geometry and focus                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| 11  | `gnoblinctl output list` shows the output                    | FAIL (empty)          | `run3/ctl.txt`, `run3/clients.txt`                                                    | empty, exit 0. Root cause: F3                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| 12  | Output is published to the object graph                      | FAIL                  | `run3/session.stdout` (8 refusals), `run4/session-after-rt-kill.stdout` (25 refusals) | `the object graph refused an object in a session snapshot kind="output" ... gnoblin.output requires an initial value for property make`. Publisher `crates/gnoblin-compositor/src/ipc/session.rs:598-604` (`publish_output`) sets only `name` and `layout`. Struct `crates/gnoblin-wayland/src/observation.rs:195` (`ObservedOutput`) has only `id`, `name`, `layout`. Schema `crates/gnoblin-schema/schema/output.toml` names `make` and more. Refusal at `crates/gnoblin-model/src/graph.rs:307` (`MissingProperty`) |
| 13  | Window rule `maximise` for `smoke-foot` takes effect         | FAIL                  | `run3/ctl.txt`, `run3/clients.txt`                                                    | the rule matches (app_id `smoke-foot`), but `foot` stays 800×600 at 224,168. The maximise branch in `apply_outcome` (`crates/gnoblin-wm/src/wm.rs:1240-1248`) runs only if `window.output` resolves; with no output published, it skips silently. That `window.output` is `None` here is [INFERENCE] from F3, not a logged fact                                                                                                                                                                                        |
| 14  | Runtime role restarts when killed                            | PASS                  | `run4/kill-runtime.txt`, `run4/session-after-rt-kill.stdout`                          | `restarting runtime in 250 ms (attempt 1 of 5)`, `started runtime (generation 2, pid 16617)`, `configuration revision 2 accepted`. `ping` after the restart: `pong`, exit 0                                                                                                                                                                                                                                                                                                                                            |
| 15  | Compositor role SIGKILL ends the session                     | PASS (matches design) | `run4/kill-compositor.txt`, `run4/session-after-comp-kill.stdout`                     | `compositor exited (signal SIGKILL); the session cannot continue`, then `session ended: compositor-lost`, `every child stopped`. The guardian does not restart it. `ping` then fails with `cannot-connect`, exit 3. Matches `design/rust-compositor.md:176` ("A crash ends the session")                                                                                                                                                                                                                               |
| 16  | SIGTERM to the guardian exits cleanly                        | PASS                  | `run4/sigterm.txt`, `run4/session-after-sigterm.stdout`                               | `elapsed=3s`, `EXITED`, `stray=0`. Log: `session ended: signal`, `every child stopped`                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 17  | Screenshot of the headless output to PNG                     | FAIL (blocked)        | `run4/grim.txt`, `run4/capture-probe.txt`                                             | `grim`: `compositor doesn't support wlr-screencopy-unstable-v1`, exit 1; no PNG written. `wayland-info` lists no screencopy or image-copy-capture global. The `capture` IPC `list` is empty, and `call open` returns `needs remote_session`, exit 2. The portal `Screenshot` is the `Missing` stub at `crates/xdg-desktop-portal-gnoblin/src/backends.rs:274`. The trait method `Backend::capture` is at `crates/gnoblin-backend/src/lib.rs:499`. No IPC or portal caller was found. Root cause: F8                    |
| 18  | Magnifier zoom changes the captured frame (PNG hash compare) | NOT RUN               | depends on 17                                                                         | no capture output exists to hash                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| 19  | Accessibility config takes effect                            | NOT VERIFIED          | depends on 11-13                                                                      | no output is published, so no visible effect could be checked                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

## Findings (not fixed here)

- **F1 (fixed)** `config get` and `config reload` used to need `commit-config`,
  which the CLI connection does not hold, so the documented edit-then-reload flow
  was denied. Fixed by splitting the authority: `read-config` and `reload-config`
  are granted to the session owner's socket connection (peer uid from
  `SO_PEERCRED`), and neither implies `commit-config`, which stays with the
  `runtime` role. See `design/rust-compositor.md`, "Capabilities and who holds
  them". The row 8 denial above is from before this change.
- **F2** `gnoblinctl config validate FILE` expects JSON. A Lua config fails with
  `invalid number at line 1 column 2`. Entry point `crates/gnoblinctl/src/verbs.rs:255`.
  Owner: CLI. Fix: route `.lua` files to the Lua parser, or document JSON only.
- **F3 (blocking)** The output object is refused in every session snapshot
  (8 refusals in `run3/session.stdout`, 25 in `run4/session-after-rt-kill.stdout`).
  `gnoblin.output` requires `make` and the other mode and geometry properties.
  `session.rs:598-604` publishes only `name` and `layout`, and `ObservedOutput`
  (`observation.rs:195`) has no such fields. Effects: `gnoblinctl output list` is
  empty, and the window rule's maximise has no output to size against
  (`wm.rs:1240-1248`). Owner: output and session lane. Fix: widen `ObservedOutput`
  to carry the required properties, publish them from `session.rs`, and add a
  test that a published output passes `graph.rs` validation. This is bigger than
  a small fix, so it is not done here.
- **F4** Cursor provider probes log `default/index.theme: unreadable: Permission
denied (os error 13)` for every shape: 231 lines in `run3/session.stdout`, 915
  in `run4/session-after-rt-kill.stdout`. Source `crates/gnoblin-cursor/src/chain.rs:112`.
  Owner: cursor lane. The chain should record a failed provider once rather than
  re-log it per shape. The denial itself is an instance HOME permission issue.
- **F5** First boot with no `last-good-config.json` logs
  `configuration not readable ... could not inspect ...: No such file or directory`
  (14 lines in `run3/session.stdout`). The source is 15 call sites that log at
  WARN, for example `crates/gnoblin-compositor/src/cursor.rs:28`,
  `crates/gnoblin-compositor/src/bell.rs:26`, `crates/gnoblin-compositor/src/accessibility.rs:39`,
  `crates/compat/gnoblin-compat-core/src/settings.rs:258`. The I/O error is produced
  at `crates/gnoblin-config/src/last_good.rs:118` (`load` at line 114). A missing
  file is a normal first-boot state, so it should be a quiet default. Owner:
  config lane. Fix: map `NotFound` to a debug-level default at the load site.
  This touches 15 sites, so it is not done here.
- **F6** Xwayland was not started on the instance. Environment, not a code bug.
- **F8** No screenshot path exists (row 17). Owner: portal and capture lane.
  Fix: implement the portal `Screenshot` against `Backend::capture`, and expose a
  capture or screencopy path that a client can call. Either implement
  `zwlr_screencopy` or the image-copy-capture protocol, or extend the capture
  schema.

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

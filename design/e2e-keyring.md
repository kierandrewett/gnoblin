# End-to-end keyring unlock: run record

Status: BLOCKED. The keyring unlock flow was never reached. Every run on the
current `rust-compositor` build (origin tip `523ff0079`) stalled before the
services role reported ready, so no `secret-tool store` call completed and no
`prompt.secret` was offered or answered.

Logs are on the workstation in `~/dev/lanes/KeyringE2E-logs/`. The run used
Agent37 instance `rp6h517btk`. It is deleted at the end of the run.

## Plan (from the brief)

1. Start a session with the guardian and the headless backend, as in
   `design/e2e-smoke.md`.
2. Store a libsecret item with `secret-tool store`. Expected D-Bus flow:
   `OpenSession`, `ReadAlias(default)`, `CreateItem`, which returns
   `('/', prompt)`, then `Prompt.Prompt`, then `prompt.secret` offered to the
   claimant, which answers, then `Completed(false, item path)`.
3. Run `secret-tool lookup` after the store has unlocked the collection.
   Known limit, recorded as a finding and not a failure: `SearchItems` while the
   collection is locked returns nothing, so a lookup before the first unlock
   finds nothing. This limit was not verified in this run.

## Runs

| Run | Scenario                            | Result                                                                | Evidence                                                    |
| --- | ----------------------------------- | --------------------------------------------------------------------- | ----------------------------------------------------------- |
| r2  | main, with store                    | BLOCKED: `secret-tool store` rc=124, services never ready             | `r2-main-stdout.txt`                                        |
| r4  | main, with store                    | BLOCKED: same stall                                                   | `r4-session.log`, `r4-sample-7s.txt`, `r4-main-console.txt` |
| r5  | no store step                       | BLOCKED: same stall, so the stall is not caused by the keyring prompt | `r5-nostore-services.txt`                                   |
| r6  | no store step, probe                | BLOCKED: same stall                                                   | `r6-nostore-probe.txt`, `r6-nostore-session.log`            |
| r7  | no store step, current build        | BLOCKED: same stall                                                   | `r7-nostore-session.log`                                    |
| r8  | fresh IPC client during stall       | compositor answers                                                    | `r8-subscribe-probe.txt`                                    |
| r9  | services process state during stall | all threads sleeping, fds listed                                      | `r9-stall-diag.txt`                                         |

Build for every run: `a37-gate.sh run rp6h517btk ... build -p gnoblin --locked`,
which was already current. The full `--workspace` build was not needed for the
runs.

## What the stall looks like

- The guardian kills the services child after 10 s with "did not report
  ready". The services log shows the keyring, displayconfig and idle agents
  starting, then nothing.
- The stall is in the introspection adapter's start
  (`crates/compat/gnoblin-compat-dbus-gnome-shell-introspect`, `agent.rs` start
  calls `link_from_environment`). That path runs `UnixTransport::connect`,
  `Client::connect` (no handshake timeout) and `IntrospectLink::connect`, which
  calls `subscribe(Filter::All)`.
- The compositor is not the blocked side. Compositor CPU rises during the stall.
  The services process is idle, and its threads are in state `S` (sleeping).
- r8: a separate client with the same connect, handshake and subscribe calls
  connects, handshakes and subscribes in under 1 ms while services is stalled
  (`t=546.752us handshake accepted`, `t=859.413us snapshot objects=7`). The IPC
  socket is therefore accepting and answering.
- So the blocked call is specific to the services process. The blocked syscall
  is not identified. `strace` and `gdb` are not installed on Agent37, so the
  blocking call was not captured. `/proc/<pid>/syscall` and `wchan` read empty.

## Open

- Identify the syscall the introspection adapter blocks in during services
  start, which needs `strace` or `gdb` on Agent37 or an in-process trace.
- After that is fixed, rerun the plan above. The keyring unlock flow, the
  `prompt.secret` answer and the `SearchItems` limit remain unverified.

## Diagnostics kept out of the tree

The probe `subscribe_probe.rs` and the harness scripts under the lane's `.e2e/`
were used for these runs. They are not committed, because they are diagnostic
only.

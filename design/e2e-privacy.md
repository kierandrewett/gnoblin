# PipeWire privacy monitor: end-to-end run record

Status: the bus layer and the PipeWire thread of `org.gnoblin.Privacy` run against a real
PipeWire 1.x daemon and WirePlumber on a real Gnoblin session. The microphone path is
proven. The camera path and the screen-capture path are BLOCKED on this instance and
remain unproven. No product defect was found. Four findings are recorded below.

Built on Agent37 with `a37-gate.sh`, lane `PrivacyE2E`, origin `17e4f626c`. Instance
`jelho4ao8c`, deleted at the end of the run. Logs: `~/dev/lanes/PrivacyE2E-logs/`
(`harness/` holds the scripts, `out/` the raw output of each run).

## Method

- Session: the guardian `gnoblin` with the headless backend, as in `design/e2e-smoke.md`.
  Runtime dir `/home/node/priv/run`. Config `keyring = { enabled = false }`, which is
  the only setting the run uses.
- Private session bus: `dbus-daemon --session --fork`. Bus address exported as
  `DBUS_SESSION_BUS_ADDRESS`.
- PipeWire: `setsid pipewire`, then `setsid wireplumber`, both under the same
  `XDG_RUNTIME_DIR`. Logs to `out/pipewire.log` and `out/wireplumber.log`.
- Virtual microphone: `pw-cli create-node adapter '{ factory.name=support.null-audio-sink
node.name=virtmic media.class=Audio/Source/Virtual object.linger=true audio.position=[ FL FR ] }'`.
- Recorders: `pw-record --target virtmic <file.wav>`.
- Monitor: `gdbus monitor --session --dest org.gnoblin.Privacy --object-path /org/gnoblin/Privacy`,
  each line stamped with `date +%T.%N`.
- `GetActivity`: `gdbus call --session --dest org.gnoblin.Privacy --object-path /org/gnoblin/Privacy --method org.gnoblin.Privacy.GetActivity`.
- Ready time: wall clock from starting `gnoblin` to `gnoblinctl ping` succeeding.

The `(available, microphone, camera)` tuple is the value of `GetActivity` and the
`ActivityChanged` payload. `available` is true when PipeWire is connected. `microphone`
is true when a running `Audio/Source` capture node exists. `camera` is true when a
running `Video/Source` node exists.

## Steps

| #   | Step                                                                                              | Result  | Evidence                                                                                                                                                                                                                                                                               |
| --- | ------------------------------------------------------------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Idle: `GetActivity` is empty                                                                      | PASS    | `(true, false, false)`, zero `ActivityChanged` lines (`out/s3-run.log`, step1).                                                                                                                                                                                                        |
| 2   | One recorder: `ActivityChanged` fires once, `(true, true, false)`; `GetActivity` agrees           | PASS    | One signal at 09:46:01.648, payload `(true, true, false)`. `r1.wav` grows (585772 bytes after 3 s). `out/signals.log`.                                                                                                                                                                 |
| 3a  | Second recorder: no new signal, state unchanged                                                   | PASS    | Still one signal; `r2.wav` 585772 bytes, `r1.wav` 1175596 bytes.                                                                                                                                                                                                                       |
| 3b  | Stop recorder 1: no change; recorder 2 keeps the mic on                                           | PASS    | Still one signal, `(true, true, false)`; `r2.wav` keeps growing (1179692 bytes).                                                                                                                                                                                                       |
| 3c  | Stop recorder 2: one signal, `(true, false, false)`                                               | PASS    | Second signal at 09:46:10.895, `(true, false, false)`. `GetActivity` agrees.                                                                                                                                                                                                           |
| 4a  | Kill the PipeWire daemon (TERM) during a recording                                                | PASS    | Signal `(false, false, false)` 0.02 s after the kill.                                                                                                                                                                                                                                  |
| 4b  | 20 s outage: no busy loop, one log line                                                           | PASS    | Services process 14 CPU ticks over 20 s (about 0.14 s of CPU, under 1% of a core). One `PipeWire is not available` line added.                                                                                                                                                         |
| 4c  | Restart PipeWire: state rebuilt, nothing stuck                                                    | PASS    | Signal `(true, false, false)` at 09:46:39.2, about 1.1 s after the restart. `GetActivity` agrees. No microphone node was running afterwards, so the state is not stuck on an old entry.                                                                                                |
| 5a  | Camera: a `Video/Source` capture node is reported as camera                                       | BLOCKED | No video source on the instance. The classification is covered only by the model unit tests (`crates/gnoblin-service-privacy/src/model.rs`).                                                                                                                                           |
| 5b  | Screen capture through PipeWire                                                                   | BLOCKED | The Gnoblin screencast producer is not built. `design/pipewire-capture-spike.md` lists building it as the next step, so the portal screencast path cannot run.                                                                                                                         |
| 6   | PipeWire absent at session start: services reaches ready, monitor logs one named line and retries | PASS    | Three cold starts with no PipeWire running. `GetActivity` `(false, false, false)`. One `PipeWire is not available, so privacy is unknown` line per start (`out/s4-run.log`). After PipeWire was started, the monitor reached `(true, false, false)` within the 6 s observation window. |
| 6b  | Recorder after the late start                                                                     | PASS    | `(true, true, false)` while recording, then `(true, false, false)` after the recorder stopped.                                                                                                                                                                                         |
| 7   | Services ready gate does not block on PipeWire                                                    | PASS    | With PipeWire absent, ready in 0.278 s, 0.284 s, 0.292 s (three cold starts) and 0.293 s (the start used for the monitor run). With PipeWire present, 0.276 s and 0.285 s.                                                                                                             |

Log lines seen in `session.log` for the whole run, in order:

- `services/privacy: privacy interface served`
- `services/services: serving ... privacy=true`
- `services/privacy: PipeWire connection lost, so privacy is unknown`, on the kill
- `services/privacy: PipeWire is not available, so privacy is unknown error=CreationFailed`, once per outage

There is no success line on reconnect. The reconnect is silent, which is the intended
behaviour, but it means the log cannot show when the monitor came back. The
`ActivityChanged` signal shows it instead.

## Findings

- F1 (limitation, not a defect): `ActivityChanged` and `GetActivity` carry only three
  booleans. The application name and pid of a recorder are not on the bus. The docs
  (`docs/dbus/privacy.md`) already say the interface reports three booleans, so this
  is the designed scope. An indicator that names the app would need a wider payload.
- F2 (limitation, not a defect): the bus cannot tell "one recorder" from "two recorders"
  on one microphone. The state is one boolean, so the second recorder produces no
  signal (step 3a).
- F3 (harness defect, not in the product): a plain `Audio/Source` from
  `support.null-audio-sink` fails to start with `Operation not supported` and a pw node
  state error. `Audio/Source/Virtual` captures correctly (about 577 KB per 3 s). All runs
  use the virtual source. An earlier "microphone anomaly" was this broken node, not the
  monitor.
- F4 (limitation of the ready measurement): ready is the moment `gnoblinctl ping`
  succeeds. The run did not check which role answers that ping. The `serving` line in
  `session.log` is the services role's own confirmation, and its timestamp was not
  recorded against the ready time.

## Not run

- Camera (5a). No video source on Agent37. Needs a real camera or a virtual `Video/Source`
  node, which this instance cannot create.
- Screen capture (5b). Needs the Gnoblin screencast producer, which is not built.
- A recorder that survives a daemon restart. The step 4 recorder was not checked after
  the kill, and no recorder was running when the restart state was read.
- Real hardware microphone. The virtual source stands in for it.

## Changes

- `design/e2e-privacy.md`: this record.
- `design/services-status.md`: the PipeWire privacy row and gap now point here.
- `docs/dbus/privacy.md`: the test coverage section now records the real-daemon run.

No product code changed. The run found no defect in the monitor, so no test was added.

## Cleanup

Instance `jelho4ao8c` was brought down with `~/.local/bin/a37 down jelho4ao8c` and
confirmed absent with `~/.local/bin/a37 list`.

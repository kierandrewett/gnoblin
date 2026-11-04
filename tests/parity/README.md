# Parity scenarios

The parity harness runs one declarative scenario against several targets and
reports, per scenario, whether the behaviour matched the reference, differed, or
could not be run yet. `design/rust-compositor.md` ("The parity harness") is the
design; this page is the working guide.

A scenario is a TOML file in `scenarios/`. A recording that a scenario replays is
a JSON file in `fixtures/`. Both are checked in, so a change to an expectation is
reviewable in the same diff as the code it describes.

## Run the scenarios

From the workspace root:

```console
$ cargo run -p gnoblin-parity -- run tests/parity/scenarios
[parity] 3 scenarios: 1 pass, 1 fail, 1 skip
  fail layer-surface-stacking (recorded:../fixtures/layer-surface-stacking/candidate.json)
       layers.list [0].layer: expected "overlay", got "top"
       layers.list [0].namespace: expected "osd", got "bar"
       layers.list [1].layer: expected "top", got "overlay"
       layers.list [1].namespace: expected "bar", got "osd"
  skip nested-session-ipc (mutter-gnoblin): needs a live headless Mutter session with gnoblin_session_host_v1; the Mutter target arrives with lane P2
  pass toplevel-configure (command), 2 observation(s) matched
```

The binary exits `0` when no scenario failed, `1` when at least one differed, and
`2` when the request itself was wrong. A skip does not fail the run: the scenario
carries its reason, and holding CI red for a lane that has not landed would hide
the differences that matter.

`just ci` runs these scenarios as part of `cargo test --workspace`, through
`crates/gnoblin-parity/tests/scenarios.rs`.

## Add a scenario

1. Create `scenarios/<name>.toml` with a `name`, a one-line `description`, a
   `target`, and one `[[steps]]` table per stimulus.
2. Put the value the observation must match in the step's `expect`. Leave `expect`
   out to collect and report an observation without comparing it, which is how a
   new scenario is explored before its reference value is known.
3. Run the directory. A failure prints the leaf path that differs and both values.

The fields:

| Field                 | Meaning                                                                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`                | Unique scenario name, printed in the report.                                                                                               |
| `description`         | What the scenario proves, in one sentence.                                                                                                 |
| `target`              | The target to run, as one of the tables below.                                                                                             |
| `skip`                | Why the scenario does not run yet. Present means the report says `skip` and the target is never started.                                   |
| `steps[].id`          | Observation name, unique in the scenario. A live target treats it as the probe to run; the recorded target treats it as the key to replay. |
| `steps[].description` | What the stimulus does, for the reader.                                                                                                    |
| `steps[].expect`      | The reference value, as a plain JSON shape. Absent means record without comparing.                                                         |
| `steps[].command`     | How the `command` target realises the step: `program`, `args`, and `format` (`text`, the default, or `json`).                              |

A step's `expect` is compared after normalisation:

- A key that names an object (`id`, `window_id`, `surface_id`) has its value
  replaced by `<id:N>`, numbered in first-seen order, so two recordings that
  allocate objects in the same order agree.
- A key that names a serial, a timestamp or a process id (`serial`, `timestamp`,
  `pid`, and any `*_serial`, `*_timestamp`, `*_msec`, `*_nsec`, `*_usec`, `*_pid`)
  is removed entirely.
- Array order is kept, because a scenario that compares a stacking order depends
  on it.

The normaliser runs once per observation and once per side, so an expectation's
labels never depend on how many other steps the scenario compares.

## Targets

A target is a way to obtain the observations a scenario asks for. Two are
implemented today, and both run without a compositor:

| Target                             | How it gets observations                                                                                                                                                                                  |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `target = { command = {} }`        | Runs each step's `command` on this machine and captures its standard output. `format = "text"` trims the trailing newlines; `format = "json"` parses the output, so an expectation compares structurally. |
| `target = { recorded = "<path>" }` | Replays observations from a JSON recording, resolved relative to the scenario file. The recording looks like `{ "target": "<who produced it>", "observations": { "<step id>": <value> } }`.               |

The three live targets are named in the plan and are not implemented yet:

| Target                             | What it will be                                                     |
| ---------------------------------- | ------------------------------------------------------------------- |
| `target = { mutter-51 = {} }`      | Stock Mutter 51 with the headless backend: core protocol reference. |
| `target = { mutter-gnoblin = {} }` | Gnoblin on the patched Mutter: reference for Gnoblin's own API.     |
| `target = { candidate = {} }`      | The Rust candidate with `gnoblin-backend` headless.                 |

A scenario that names a live target without a `skip` reason fails loudly with
"target `...` is not implemented". There is no stand-in that pretends to drive a
compositor, so a scenario cannot pass on behaviour nothing exercised.

### Adding a live target

A live target implements the `Target` trait in `crates/gnoblin-parity/src/target.rs`:
`observe` starts a session with a fixed virtual output layout, applies the
scenario's steps, reads the observations, and stops the session before returning.
The shared pieces are in `gnoblin-testkit`: `RuntimeDir` gives the session its own
`XDG_RUNTIME_DIR` to pass with `Command::env`, `Clock` stamps observations
deterministically, and `Recorder` writes a trace whose `record_secret` calls can
never leak a secret into a checked-in recording.

When a live target lands:

1. Add its variant to `TargetSpec`, or fill in the existing one.
2. Return the target from `TargetSpec::build_target` instead of the
   `NotImplemented` error.
3. Record one run of each scenario on it into `fixtures/<scenario>/`, and remove
   the scenario's `skip` reason.

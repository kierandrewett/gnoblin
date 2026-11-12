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
[parity] 11 scenarios: 6 pass, 1 divergent, 0 fail, 4 skip
  pass candidate-kill-unknown-window (candidate), 1 observation(s) matched
  pass candidate-window-lifecycle (candidate), 2 observation(s) matched
  pass candidate-workspace-list (candidate), 1 observation(s) matched
  divergent focus-toplevels (candidate), 3 observation(s) compared
       focus.second-toplevel: focus_new_windows defaults to prevent in the candidate, ...
  skip focus-toplevels (mutter51): no headless Mutter 51 is configured: set GNOBLIN_PARITY_MUTTER51 to a launcher ...
  skip nested-session-ipc (mutter-gnoblin): needs a live headless Mutter session with gnoblin_session_host_v1; ...
  pass popup-lifecycle (candidate), 3 observation(s) matched
  skip popup-lifecycle (mutter51): no headless Mutter 51 is configured: ...
  pass popup-placement (candidate), 5 observation(s) matched
  skip popup-placement (mutter51): no headless Mutter 51 is configured: ...
  pass toplevel-configure (command), 2 observation(s) matched
```

A scenario that names references prints one row per target. Without a Mutter launcher the `mutter51` rows are skips,
with the variable to set; "Run the Mutter reference" below runs them.

The binary exits `0` when no scenario failed, `1` when at least one differed, and
`2` when the request itself was wrong. A skip does not fail the run: the scenario
carries its reason, and holding CI red for a lane that has not landed would hide
the differences that matter. `--require mutter51` makes a skipped `mutter51` row fail the run, which is how
the reference job (`.github/workflows/parity-reference.yml`) refuses to pass without having run Mutter.

`scenarios/` is a gate and stays green: a scenario that can never pass lives in `known-failing/`, which only the runner's
own tests run (`layer-surface-stacking`, kept as a refused expectation).

`just ci` runs these scenarios as part of `cargo test --workspace`, through
`crates/gnoblin-parity/tests/scenarios.rs`, with no Mutter launcher in the environment.

## Run the Mutter reference

`target = { mutter51 = {} }`, or `references = [{ mutter51 = {} }]` beside another target, runs stock Mutter 51 on its
headless backend, started by a launcher program the environment names. The scenario's steps run on it through the same
Wayland client the candidate uses, and both targets are compared with one `expect`.

| Variable                      | Meaning                                                                                    |
| ----------------------------- | ------------------------------------------------------------------------------------------ |
| `GNOBLIN_PARITY_MUTTER51`     | The launcher, called as `<launcher> <runtime-dir> <display>`. Unset: the rows are skipped. |
| `GNOBLIN_PARITY_RUNTIME_BASE` | Where the private runtime directory is made. Default: the system temporary directory.      |

`scripts/parity-mutter51` is a launcher for any machine with Mutter 51. It starts `dbus-run-session -- mutter --headless
--wayland --wayland-display <display> --virtual-monitor 1280x720` with `XDG_RUNTIME_DIR` set, and its own variables say
where Mutter is: `GNOBLIN_PARITY_MUTTER_PREFIX` (an install prefix), `GNOBLIN_PARITY_MUTTER_ARGS` (extra arguments,
default `--no-x11`; set it empty for a build without Xwayland) and `GNOBLIN_PARITY_MUTTER_ROOT` (run inside a root file
system, below).

On a Fedora 45 machine or container the whole sequence is `scripts/parity-reference.sh`: `deps` installs the build
requirements, `mutter` builds the upstream `51.0` tag into `/opt/mutter51`, and `run REPORT` runs this directory with
`--require mutter51`. The CI job `parity-reference.yml` runs those three steps and keeps the report as an artifact.

Mutter 51 needs glib 2.81, wayland 1.25, libinput 1.30 and pipewire 1.4.11, so it does not build on Debian 12 (the
Agent37 image: glib 2.74, wayland 1.21). Agent37 instances cannot run containers or mount, but they can `chroot`
as root, so the reference runs there from an extracted Fedora 45 root file system. The recipe, all as root through
`a37 exec --root`:

1. Download `Fedora-Container-Base-Generic-45-1.4.x86_64.oci.tar.xz` from `dl.fedoraproject.org`, unpack the OCI layer
   into `/home/node/fedora`. Mounts are refused, so `/proc` is absent and `/dev/null` is an ordinary file in the tree.
2. `chroot /home/node/fedora /bin/bash scripts/parity-reference.sh deps` and `... mutter`, with the repository copied in.
   `dnf` needs `--setopt=tsflags=nocaps` (file capabilities cannot be set, the script passes it). `rustup` and `rustc` do
   not start without `/proc`, so the runner is built outside, on the instance's own toolchain.
3. Run the runner built outside (`cargo build -p gnoblin-parity`) as root with
   `GNOBLIN_PARITY_MUTTER51=<repo>/scripts/parity-mutter51 GNOBLIN_PARITY_MUTTER_ROOT=/home/node/fedora
GNOBLIN_PARITY_MUTTER_PREFIX=/opt/mutter51 GNOBLIN_PARITY_MUTTER_ARGS=
GNOBLIN_PARITY_RUNTIME_BASE=/home/node/fedora/tmp/parity-rt`. The base must be inside the root file system, so the
   socket Mutter creates there is the path the runner connects to; the launcher strips the root from the path it passes
   in. Or copy the runner into the tree and run `scripts/parity-reference.sh run` there with `GNOBLIN_PARITY_BIN` set.

The stored reports are in `reports/`. Each says where it was produced and what Mutter it ran.

## Recorded divergences

A step may record that one target differs from the reference, and why:

```toml
[[steps]]
id = "focus.second-toplevel"
expect = { a_activated = false, b_activated = true }   # what the reference does

[steps.divergence.candidate]
expect = { a_activated = true, b_activated = false }   # what the candidate does instead
reason = "focus_new_windows defaults to prevent in the candidate ..."
```

The target is compared against the divergence, and its row reads `divergent` with the reason. A target that stops
diverging no longer matches the entry, so the row fails and the entry has to go: a divergence cannot outlive the
behaviour it describes. `divergences.toml` lists the scenarios that have no reference run at all, and why.

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
| `references`          | Further live targets (`[{ mutter51 = {} }]`) that run the same steps against the same `expect`, one report row each.                       |
| `skip`                | Why the scenario does not run yet. Present means the report says `skip` and the target is never started.                                   |
| `steps[].id`          | Observation name, unique in the scenario. A live target treats it as the probe to run; the recorded target treats it as the key to replay. |
| `steps[].description` | What the stimulus does, for the reader.                                                                                                    |
| `steps[].expect`      | The reference value, as a plain JSON shape. Absent means record without comparing.                                                         |
| `steps[].divergence`  | `[steps.divergence.<target>]` with `expect` and `reason`: what one target does instead of `expect`, as a reviewed difference.              |
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

A target is a way to obtain the observations a scenario asks for. Two run without a compositor:

| Target                             | How it gets observations                                                                                                                                                                                  |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `target = { command = {} }`        | Runs each step's `command` on this machine and captures its standard output. `format = "text"` trims the trailing newlines; `format = "json"` parses the output, so an expectation compares structurally. |
| `target = { recorded = "<path>" }` | Replays observations from a JSON recording, resolved relative to the scenario file. The recording looks like `{ "target": "<who produced it>", "observations": { "<step id>": <value> } }`.               |

The Rust candidate is live: `target = { candidate = {} }` starts `gnoblin-compositor`
headless on a thread, in a private temporary directory, and drives it over its IPC
socket (through the `gnoblinctl` library) and a real Wayland client. Stock Mutter 51 is live too
(`mutter51`, above) and answers the Wayland probes only. The step ids are a closed set of probes, so
an unknown id fails the run:

| Probe                                                                                            | Answered by             | What it observes                                                                        |
| ------------------------------------------------------------------------------------------------ | ----------------------- | --------------------------------------------------------------------------------------- |
| `workspace.list`, `window.open`, `window.close`, `window.kill.unknown`                           | `candidate`             | `gnoblinctl` output and exit status, and a toplevel a Wayland client maps and destroys. |
| `focus.first-toplevel`, `focus.second-toplevel`, `focus.close-focused`                           | `candidate`, `mutter51` | The `activated` state of each toplevel's configure: how a client learns it has focus.   |
| `popup.place.anchor-bottom-right`, `.gravity-top-left`, `.offset`, `.nested`, `popup.reposition` | `candidate`, `mutter51` | The position and size of the `xdg_popup` configure, and the `repositioned` token.       |
| `popup.map.parent-keeps-focus`, `popup.destroy.parent-keeps-focus`, `popup.grab.without-input`   | `candidate`, `mutter51` | Whether the parent stays activated, and whether the popup is configured or dismissed.   |

`mutter-gnoblin` (Gnoblin on the patched Mutter, the reference for Gnoblin's own API) is named in the plan and not
implemented. A scenario that names it without a `skip` reason fails loudly with "target `mutter-gnoblin` is not
implemented". There is no stand-in that pretends to drive a compositor, so a scenario cannot pass on behaviour nothing
exercised. Candidate scenarios that have no reference run are listed in `divergences.toml`.

### Adding a live target

A live target implements the `Target` trait in `crates/gnoblin-parity/src/target.rs`:
`observe` starts a session with a fixed virtual output layout, applies the
scenario's steps, reads the observations, and stops the session before returning.
The Wayland client the live targets share is `wayland_peer.rs` and the probes both answer are in `probes.rs`; a
new core-protocol scenario adds a probe there and runs on both. The shared pieces are in `gnoblin-testkit`: `RuntimeDir` gives the session its own
`XDG_RUNTIME_DIR` to pass with `Command::env`, `Clock` stamps observations
deterministically, and `Recorder` writes a trace whose `record_secret` calls can
never leak a secret into a checked-in recording.

When a live target lands:

1. Add its variant to `TargetSpec`, or fill in the existing one.
2. Return the target from `TargetSpec::build_target` instead of the
   `NotImplemented` error.
3. Record one run of each scenario on it into `fixtures/<scenario>/`, and remove
   the scenario's `skip` reason.

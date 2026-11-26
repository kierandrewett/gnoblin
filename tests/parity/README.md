# Parity scenarios

The parity harness runs one declarative scenario against several targets and
reports, per scenario, whether the behaviour matched the reference, differed, or
could not be run yet. This page is the working guide.

A scenario is a TOML file in `scenarios/`. A recording that a scenario replays is
a JSON file in `fixtures/`. Both are checked in, so a change to an expectation is
reviewable in the same diff as the code it describes.

## Run the scenarios

From the workspace root:

```console
$ cargo run -p gnoblin-parity -- run tests/parity/scenarios
[parity] 20 scenarios: 5 pass, 3 divergent, 0 fail, 12 skip
  divergent candidate-kill-unknown-window (candidate), 1 observation(s) compared
       window.kill.unknown: the refusal text is the candidate's object-model error, ...
  skip candidate-kill-unknown-window (mutter-gnoblin): no headless Gnoblin on the patched Mutter is configured: set GNOBLIN_PARITY_MUTTER_GNOBLIN to a launcher ...
  pass candidate-window-lifecycle (candidate), 2 observation(s) matched
  skip candidate-window-lifecycle (mutter-gnoblin): no headless Gnoblin on the patched Mutter is configured: ...
  divergent candidate-workspace-list (candidate), 1 observation(s) compared
       workspace.list: the candidate answers in the IPC 0.2.0 object model ...
  divergent focus-toplevels (candidate), 3 observation(s) compared
       focus.second-toplevel: focus_new_windows defaults to prevent in the candidate's schema; ...
  skip focus-toplevels (mutter51): no headless Mutter 51 is configured: set GNOBLIN_PARITY_MUTTER51 to a launcher ...
  pass input-eis (candidate), 4 observation(s) matched
  pass popup-lifecycle (candidate), 3 observation(s) matched
  pass popup-placement (candidate), 5 observation(s) matched
  pass toplevel-configure (command), 2 observation(s) matched
```

A scenario that names references prints one row per target. Without a launcher the `mutter51` and `mutter-gnoblin` rows
are skips, with the variable to set; "Run the Mutter reference" and "Run the Gnoblin reference" below run them. The
candidate rows of the output above are the ones a plain `cargo` run produces; the full report of all three targets is
stored in `reports/`.

The binary exits `0` when no scenario failed, `1` when at least one differed, and
`2` when the request itself was wrong. A skip does not fail the run: the scenario
carries its reason, and holding CI red for a lane that has not landed would hide
the differences that matter. `--require mutter51` (and `--require mutter-gnoblin`) makes a skipped row of that target fail the run, which is how
the reference job (`.github/workflows/parity-reference.yml`) refuses to pass without having run the references.

`scenarios/` is a gate and stays green: a scenario that can never pass lives in `known-failing/`, which only the runner's
own tests run (`layer-surface-stacking`, kept as a refused expectation).

`just ci` runs these scenarios as part of `cargo test --workspace`, through
`crates/gnoblin-parity/tests/scenarios.rs`, with no Mutter launcher in the environment.

## Run the Mutter reference

`target = { mutter51 = {} }`, or `references = [{ mutter51 = {} }]` beside another target, runs stock Mutter 51 on its
headless backend, started by a launcher program the environment names. The scenario's steps run on it through the same
Wayland client the candidate uses, and the targets are compared with one `expect`. Input reaches it the way it reaches
every live target, over EIS ("Input"), through `org.gnome.Mutter.RemoteDesktop` on the private session bus the launcher
starts at `<runtime-dir>/bus`.

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
requirements, `mutter` builds the upstream `51.0` tag into `/opt/mutter51`, `gnoblin` builds the Mutter-based Gnoblin into
`/opt/gnoblin` ("Run the Gnoblin reference") and `run REPORT` runs this directory with `--require mutter51
--require mutter-gnoblin`. The CI job `parity-reference.yml` runs those four steps and keeps the report as an artifact.
That job has not run on a GitHub runner yet; its steps ran on Agent37 as below.

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

The stored reports are in `reports/`. Each says where it was produced and what it ran: `2026-11-15-agent37-all-targets.txt`
is the full run on the candidate, `mutter51` and `mutter-gnoblin` (20 rows: 16 pass, 3 divergent, 0 fail, 1 skip).

## Run the Gnoblin reference

`target = { mutter-gnoblin = {} }`, or `references = [{ mutter-gnoblin = {} }]`, runs the Mutter-based Gnoblin on its patched
Mutter 51.0, headless, for real: the guardian (`gnoblin --devkit --wayland-display <display> --no-xwayland`), the
patched Mutter on its headless backend with one 1280x720 virtual monitor, the Lua runtime and the control socket
`<runtime-dir>/compositor-v1.sock`. It is the reference for Gnoblin's own API, which stock Mutter does not have, and it
answers every probe the other live targets answer (same Wayland client, same EIS door). Nothing stands in for it: a
scenario with this target is a skip, with the variable to set, when no launcher is configured, and a failure, with the
compositor's log, when the session does not come up.

| Variable                            | Meaning                                                                                                             |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `GNOBLIN_PARITY_MUTTER_GNOBLIN`     | The session launcher, `<launcher> <runtime-dir> <display>`: `scripts/parity-mutter-gnoblin`. Unset: rows skipped.   |
| `GNOBLIN_PARITY_MUTTER_GNOBLIN_CTL` | The control client, `<ctl> <runtime-dir> <gnoblinctl arguments>`. Default: `parity-gnoblinctl` beside the launcher. |
| `GNOBLIN_PARITY_GNOBLIN_PREFIX`     | Install prefix of a Gnoblin built with `GNOBLIN_DEVKIT=enabled`, read by both scripts. Required.                    |
| `GNOBLIN_PARITY_MUTTER_ROOT`        | Run both scripts in this root file system, as for `mutter51`.                                                       |
| `GNOBLIN_PARITY_PROC_SHIM`          | A path inside that root to a library built from `scripts/parity-proc-shim.c`, preloaded in the session (below).     |

`scripts/parity-gnoblinctl` runs the Mutter-based `gnoblinctl` against the session's socket (`GNOBLIN_COMPOSITOR_SOCKET`) and the
runner reads its output; `window kill <id>` and `workspace list` are the commands the API probes use. The launchers share
`scripts/parity-launcher-lib.sh`, which enters the root file system and starts the private session bus the EIS door uses.

`scripts/parity-reference.sh gnoblin` builds the prefix from the Mutter-based source tree that
`GNOBLIN_PARITY_GNOBLIN_SOURCE` names: a checkout of `main` (observed on `5bb523e9c`, API 1.93, which is the API of the
contract inventory). The launchers and the runner are this branch's. The CI job checks `main` out beside the branch.

On Agent37 the same Fedora 45 root file system as above builds and runs it, with four differences that are properties of
that sandbox, not of Gnoblin. None of them changes what the compositor does:

- The Mutter-based tree's `build.sh` runs its `scripts/list-tarball-sources.sh`, which uses bash process substitution (`< <(...)`). That needs
  `/dev/fd`, which needs `/proc`, which a chroot there does not have, so the step stops with "/dev/fd/63: No such file or
  directory". The throwaway checkout in the root had its three `<(...)` uses replaced by a here-string and a temporary
  file; the repository's script is unchanged.
- The Mutter-based tree's `scripts/install-fedora-build-deps.sh` printed "Transaction failed: Rpm transaction failed" for a systemd trigger
  (`Failed to import file '/usr/lib/systemd/catalog/dbus-broker-launch.catalog'`), after the packages were installed,
  except `gstreamer1`, which was missing and made the Mutter link fail with undefined `gst_*` references until it was
  installed by hand.
- The guardian finds itself with `readlink("/proc/self/exe")` to start its supervisor and worker. Without `/proc` that
  fails ("could not locate installed executable"). `scripts/parity-proc-shim.c` is a preload library that answers that one
  link with the path the process was executed by and passes every other `readlink` through. Build it in the root with
  `cc -shared -fPIC -O2 -o /opt/parity-proc-shim.so scripts/parity-proc-shim.c -ldl` and set `GNOBLIN_PARITY_PROC_SHIM`.
- `/dev/null` is an ordinary file in that tree, so something has to keep it empty (`while :; do : > /dev/null; sleep 1;
done` outside the root).

The recipe on the instance, as root: build the runner outside the root as for `mutter51`, then run it with
`GNOBLIN_PARITY_MUTTER_ROOT=/home/node/fedora-45 GNOBLIN_PARITY_RUNTIME_BASE=/home/node/fedora-45/tmp/parity-rt
GNOBLIN_PARITY_GNOBLIN_PREFIX=/opt/gnoblin GNOBLIN_PARITY_PROC_SHIM=/opt/parity-proc-shim.so
GNOBLIN_PARITY_MUTTER_GNOBLIN=<repo>/scripts/parity-mutter-gnoblin`, with the `mutter51` variables of the section above
beside them to run both references in one pass. The session log of a run shows the real Mutter 51.0 starting, a
`Devkit viewer exited; keeping the compositor session alive` line (the devkit's viewer has no display to open on; the
session outlives it) and warnings about the system bus and colord, which the sandbox does not have.

## Input

Every live target is given input the way a real client would give it: the runner is the EIS sender (`ei_peer.rs`, the
`reis` crate). Mutter and the Mutter-based Gnoblin hand out an EIS socket from `org.gnome.Mutter.RemoteDesktop`
(`CreateSession`, `ConnectToEIS`, then `Start`, in `remote_desktop.rs`); the candidate serves `eis.sock`. The sender binds
every capability the seat advertises and waits until the devices that carry the interfaces the probes send through
(`ei_pointer`, `ei_keyboard`, `ei_scroll`, `ei_button`) have resumed. Two things a Mutter seat taught the harness, both
fixed:

- The EIS server chooses the bit of each capability and names its interface in the same event, so the sender keeps
  the interface names and never reads a mask. Mutter's mask is `0xfc0`, not the bits a libei header suggests, and a
  sender that decoded it by position stopped waiting at the first device it saw: sometimes the pointer's, so the keyboard
  device was not there yet: the performance baseline of `mutter51` lost every key in some runs and none in others.
- A Mutter seat advertises a keyboard only once an input device exists, and the EIS sender is what makes one, so the EI
  client connects before the Wayland client binds the keyboard and waits for the seat to advertise what EIS offered.

The `input-eis` scenario holds the four probes: a key (`a`), three relative motions of (10, 5), a left button press and
release, and a vertical scroll. Their expectations are what stock Mutter 51.0 produced; the candidate and the Mutter-based
session produced the same, so no step records a divergence. The key arrives as evdev code 30 and the button as code 272.
Motion is compared as the step between two motions (the position depends on where each compositor starts the pointer) and a
scroll as its axis and direction (a compositor scales a scroll as it likes).

## Performance baseline

`gnoblin-parity perf <candidate|mutter51|mutter-gnoblin> [--json FILE]` measures one target from outside its process:
frame time of a redrawing window, key latency over EIS (to the client and to the next frame), idle CPU and the largest
process's RSS at 0, 10 and 50 windows, with the same environment as `run`. The stored baseline is
`reports/2026-11-15-agent37-perf-baseline.txt`, with its command lines, exit codes, host and limits: headless, software
rendering, gVisor, one run per target, so it is a baseline for later comparison on the same kind of machine and not a
hardware measurement. The candidate runs in the runner's process (`serve-candidate`), so its RSS and the Mutter targets'
are not the same thing, and Mutter paces its frame callbacks at 60 Hz where the candidate's come faster.

## Contract recording

`contracts/` holds the contract read out of the Mutter-based C sources. `scripts/contract-record.py` records what the Mutter-based
session answered to it:
it connects to a running Mutter-based session (API 1.93), asks every operation of the inventory once with an empty argument
table, subscribes to every event the session offers and writes `contracts/recorded-session.json`. A subscription takes at
most 64 names and a later one replaces an earlier one, so the recorder uses one connection per slice of 64; `windows` and
`monitors` in the hello list are streams started by an operation of that name, not subscribable names. `--drive CMD` runs
a client after the subscription (`gnoblin-parity hold-window <wayland-socket> <seconds>` maps one toplevel and closes it),
so window events arrive. The stored recording is from `main` at `5bb523e9c`: 136 operations (49 ok, 77 refused for want of
arguments or a target, 10 not asked because they log out, power off, lock or spawn), and 11 event names seen, including
`gnoblin.window.created`, `.focused` and `.closed`. `python3 scripts/contract-record.py --check` verifies it offline
and `tests/contract-record.test.py` runs that check.

## Record the reference session

`reference-session/` holds what a stock Mutter 51 headless session puts on the session bus and on its Wayland socket,
recorded by `scripts/reference-session-record.sh` from a Fedora 45 root file system as the Mutter-reference steps above
set one up (`GNOBLIN_PARITY_MUTTER_ROOT=<root> scripts/reference-session-record.sh <root>/tmp/out`; it also needs
`systemd` for `busctl`, `wayland-utils`, `dbus-tools`, `python3-dbus`, `pipewire` and `gtk4-devel-tools` in the root).
`run.txt` says which versions, which command lines and which exit codes. The files are the output of the commands, not
edits of it. `dbus-monitor-mutter.txt` is the part of `busctl monitor` that Mutter's own connection sent; the full stream
is 440 KB of the tool's own traffic and is not kept.

`scripts/reference-session-matrix.py` compares the recording with every adapter's `support-matrix.json` and writes
`tests/parity/adapter-support-matrices.md`; `--check` fails when that page is stale, and `tests/reference-session-matrix.test.py`
runs it. The page also lists what the session could not show: the stock GNOME portal refused every call here because its
sandbox check opens `/proc/PID/root`, so the redirect map's known consumers are still [INFERENCE].

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

| Field                 | Meaning                                                                                                                                       |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                | Unique scenario name, printed in the report.                                                                                                  |
| `description`         | What the scenario proves, in one sentence.                                                                                                    |
| `target`              | The target to run, as one of the tables below.                                                                                                |
| `references`          | Further live targets (`[{ mutter51 = {} }, { mutter-gnoblin = {} }]`) that run the same steps against the same `expect`, one report row each. |
| `skip`                | Why the scenario does not run yet. Present means the report says `skip` and the target is never started.                                      |
| `steps[].id`          | Observation name, unique in the scenario. A live target treats it as the probe to run; the recorded target treats it as the key to replay.    |
| `steps[].description` | What the stimulus does, for the reader.                                                                                                       |
| `steps[].expect`      | The reference value, as a plain JSON shape. Absent means record without comparing.                                                            |
| `steps[].divergence`  | `[steps.divergence.<target>]` with `expect` and `reason`: what one target does instead of `expect`, as a reviewed difference.                 |
| `steps[].command`     | How the `command` target realises the step: `program`, `args`, and `format` (`text`, the default, or `json`).                                 |

A step's `expect` is compared after normalisation:

- A key that names an object (`id`, `window_id`, `surface_id`) has its value
  replaced by `<id:N>`, numbered in first-seen order, so two recordings that
  allocate objects in the same order agree.
- A key that names a serial, a change counter, a timestamp or a process id
  (`serial`, `revision`, `timestamp`, `pid`, and any `*_serial`, `*_timestamp`,
  `*_msec`, `*_nsec`, `*_usec`, `*_pid`) is removed entirely.
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

| Probe                                                                                            | Answered by                               | What it observes                                                                        |
| ------------------------------------------------------------------------------------------------ | ----------------------------------------- | --------------------------------------------------------------------------------------- |
| `workspace.list`, `window.open`, `window.close`, `window.kill.unknown`                           | `candidate`, `mutter-gnoblin`             | `gnoblinctl` output and exit status, and a toplevel a Wayland client maps and destroys. |
| `focus.first-toplevel`, `focus.second-toplevel`, `focus.close-focused`                           | `candidate`, `mutter51`, `mutter-gnoblin` | The `activated` state of each toplevel's configure: how a client learns it has focus.   |
| `popup.place.anchor-bottom-right`, `.gravity-top-left`, `.offset`, `.nested`, `popup.reposition` | `candidate`, `mutter51`, `mutter-gnoblin` | The position and size of the `xdg_popup` configure, and the `repositioned` token.       |
| `popup.map.parent-keeps-focus`, `popup.destroy.parent-keeps-focus`, `popup.grab.without-input`   | `candidate`, `mutter51`, `mutter-gnoblin` | Whether the parent stays activated, and whether the popup is configured or dismissed.   |
| `input.key.reaches-focused-toplevel`, `input.pointer.relative-motion`, `.button`, `.scroll`      | `candidate`, `mutter51`, `mutter-gnoblin` | The `wl_keyboard` and `wl_pointer` events a fullscreen client receives for EIS input.   |

`mutter-gnoblin` (the Mutter-based Gnoblin on the patched Mutter, "Run the Gnoblin reference" above) answers every probe, and it
is the only target besides the candidate that answers the ones that read Gnoblin's own API. Its observations are what
the Mutter-based `gnoblinctl` printed, reduced to the keys the candidate's observation has where the two mean the same thing
(`window.open`, `window.close`) and the Mutter-based command's own JSON where they do not (`workspace.list`, an array against
the candidate's object model; `window.kill.unknown`, a fixed sentence against an error that names the identity). A
scenario records each such difference as a divergence of the candidate. There is no stand-in that pretends to drive a
compositor, so a scenario cannot pass on behaviour nothing exercised. The scenarios with no `mutter51` row are listed in
`divergences.toml`.

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

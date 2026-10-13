# Devkit viewer parent-death signal regression

Patch 0008 installs `PR_SET_PDEATHSIG(SIGTERM)` for the compositor's direct
viewer child. This supplies a termination signal independently of D-Bus
delivery and viewer main-loop progress. The real viewer already quits through
its RemoteDesktop disconnect callbacks during normal operation. After session
initialization it calls `setsid()`, leaving the compositor's process group.

This fixture tests the parent-death mechanism with a waiting helper that has
no real viewer disconnect callbacks. Its failing baseline does not establish
that an ordinary viewer fails to exit, or attribute the historical orphan in
gnoblin-f55 / GitHub #191.

## Run

Run from the Gnoblin root on Linux with Python 3, a C compiler, GIO development
files, `pkg-config`, Git, and `dbus-daemon`:

```sh
python3 tests/devkit-viewer-lifecycle/run.py --baseline
python3 tests/devkit-viewer-lifecycle/run.py
similarity-py tests/devkit-viewer-lifecycle
```

The baseline must exit 1: nine parent-death checks fail and the existing spawn
error check passes. The patched run must exit 0 with ten passing cases.
Compilation uses `-Wall -Wextra -Werror`. Both runs compile with and without
`HAVE_SYS_PRCTL`; the non-prctl build is a compile check only.

The runner reads `meta-mdk.c` from the Mutter commit in `gnome-versions.json`
and applies tooling patch 0007, then 0008 for the patched run, in a temporary
repository under this test directory. It compiles the actual lifetime callback
and launcher registration extracted from that source. It never modifies or
stages the shared submodule. Temporary sources and binaries are removed.

The fixture uses its own GSubprocess owner/viewer pairs and a private D-Bus
daemon. It does not start a compositor, viewer UI, or desktop service. Its
subreaper adopts orphaned fixture children. Every case has bounded event and
exit waits; teardown kills only that fixture's process group and reaps its
children, including survivors in the expected failing baseline.

## Failure cases

| Case                    | Required result                                                                                                   |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Normal owner exit       | Executed viewer exits from SIGTERM.                                                                               |
| Owner SIGTERM           | Executed viewer exits from SIGTERM.                                                                               |
| Owner SIGKILL           | Executed viewer exits from SIGTERM.                                                                               |
| Owner dies before setup | A pipe barrier delays setup; the parent-PID check exits 127 before exec.                                          |
| Owner dies after setup  | A pipe barrier delays exec; the installed signal terminates the child.                                            |
| Invalid executable      | GSubprocess returns an error and no subprocess.                                                                   |
| Injected prctl failure  | The extracted callback exits 127 before exec.                                                                     |
| Two independent pairs   | Killing A's owner ends A's viewer; B remains live until its own owner exits.                                      |
| Worker-thread creator   | The viewer dies when the creator thread exits, while the owner process remains alive.                             |
| Main-context creator    | Both private-bus callbacks, synchronous enabled signals, and spawn use the main TID; owner death ends the viewer. |

The barriers and failure injection wrap the extracted callback. The lifetime
logic is not reimplemented in the fixture. The exec target is the fixture
binary itself, which reports its PID, TID, and parent before waiting.

## Creator-thread evidence

Linux binds `PR_SET_PDEATHSIG` to the thread that created the child, rather
than the lifetime of every thread in the parent process. A worker that returns
after spawning would terminate the viewer too early.

The reviewed source path is:

1. `src/core/mutter.c:main()` calls context setup, start, and the main loop
   serially on the main thread.
2. `src/core/meta-context.c:meta_context_real_setup()` creates the backend.
   `src/backends/meta-backend.c` constructs screen-cast and remote-desktop
   services during backend initialization.
3. `src/backends/meta-dbus-session-manager.c:meta_dbus_session_manager_constructed()`
   registers each service with `g_bus_own_name()`. Its `on_name_acquired()` emits
   `enabled` synchronously.
4. `src/core/meta-context-main.c:meta_context_main_setup()` creates MetaMdk
   after backend setup. `meta_mdk_new()` connects both service signals directly
   to `maybe_launch_devkit()`.
5. `meta_context_start()` captures the thread-default context for its main
   loop. `meta_context_run_main_loop()` iterates it on the same main thread.
   `maybe_launch_devkit()` calls `g_subprocess_launcher_spawnv()` directly.

The `org.gnome.Mutter.Devkit` name registered in `init_api()` exports the
devkit API. The viewer spawn is triggered by the other two service owners.

[GLib's name-owner contract](https://docs.gtk.org/gio/func.bus_own_name.html)
dispatches callbacks in the registering thread's thread-default main context.
In GLib 2.86.5, `gio/gsubprocess.c` passes the child callback synchronously to
`g_spawn_async_with_pipes_and_fds()` with `G_SPAWN_DO_NOT_REAP_CHILD`.
`glib/gspawn-posix.c` uses `fork()` when child setup is supplied, without an
intermediate child on this path, and resets SIGTERM before the callback.
See the [GLib source](https://github.com/GNOME/glib/tree/2.86.5).

The private-bus fixture validates that dispatch pattern with the local GLib;
it does not run Mutter's callbacks. Main separately verified the native creator
with strace: compositor main TID 643782 forked viewer 643816, which installed
the SIGTERM death signal before exec. The callback chain above remains a
source review; the trace confirms the required creator thread. A future move
to worker-thread spawning must revisit this patch.

## Recorded local result

On 2026-10-13, with GIO 2.86.5, the pinned-source baseline failed all nine
parent-death cases and passed spawn failure. The patched fixture passed all ten.
Both compile configurations passed. All fixture process groups were reaped.

The main-context run recorded owner PID/TID 1935311, matching name-acquired,
enabled, and spawn TIDs, and direct child PID 1935317. The worker run recorded
owner 1935305 still alive after creator TID 1935306 exited and its child received
SIGTERM. These are fixture observations, not a running compositor claim.

## Native evidence reported by main

Main tested private guest artifacts with the wrapper and guardian paused so
their cleanup could not end the viewer during observation. An already-running
original viewer (`2bf1f...`) exited by itself 0.237 seconds after compositor
SIGKILL. The 0008 candidate exited in approximately 10 ms; its independent peer
was unaffected. Candidate normal-TERM exit was approximately 40 ms. Ordinary
compositor-loss shutdown therefore worked in that original control.

Main also paused the actual viewer in an `LD_PRELOAD` barrier at
`ei_setup_backend_fd()`, after EIS fd delivery and before the real setup call.
After killing the verified compositor and releasing the barrier:

- Original `2bf1f...` stayed alive for the two-second observation and consumed
  1.97 CPU seconds.
- 0008 candidate `21424...` received the parent-death SIGTERM and exited
  immediately, with zero CPU time reported for the observation.
- Both viewers were reaped.

The supplied evidence paths are
`/home/luna/astra-native-20261013/startup-race-baseline.json` and
`/home/luna/astra-native-20261013/startup-race-parent-signal.json` in the guest,
and `astra-devkit-startup-race.log` under main's host `RUN` directory. Artifact
identifiers here are the supplied abbreviations. These are main-reported
results; this worker neither accessed the VM nor inspected those logs.

This reproduces a native startup race and shows that 0008 ends it when its
compositor parent dies. It does not attribute the historical long-lived
PPID-1 viewer to that race or establish its resolution.

The separate [0009 fixture](../devkit-ei-startup/README.md) covers handling a
real `EI_EVENT_DISCONNECT` before the first seat. Its native post-setup
transport-disconnect check also passed: the viewer exited through its error
handler while the compositor remained alive. The earlier pre-setup injection
produced no event in the native viewer and remains documented as a limitation.
0008 cannot cover a disconnect while its creator stays alive.

## Repeat native checks

Main owns these repeat checks in its disposable VM, using its isolated, built
candidate prefix. This worker ran no VM or host desktop commands. The fixture
does not build, install, or change the shared source tree.

First verify the exact artifact and its normal nested interaction. From the
guest's Wayland desktop, use a prefix that already contains the devkit-enabled
build so the wrapper does not trigger a build:

```sh
candidate_prefix=/absolute/path/to/candidate/prefix
test -x "$candidate_prefix/libexec/mutter-devkit"
test -f "$candidate_prefix/share/gnoblin/mutter-devkit-enabled"
stat -c '%a %U:%G %n' "$candidate_prefix/libexec/mutter-devkit"
getcap "$candidate_prefix/libexec/mutter-devkit"
GNOBLIN_PREFIX="$candidate_prefix" scripts/run-clean-devkit.sh
```

The viewer must have no set-ID bits or file capabilities. The
[Linux contract](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html)
clears the death signal across those credential-changing execs. Confirm normal
rendering/input and terminal-close cleanup before testing abrupt owner death.

For a run held open for inspection, start this in the guest terminal:

```sh
probe_dir=$(mktemp -d /var/tmp/devkit-owner.XXXXXX)
GNOBLIN_PREFIX="$candidate_prefix" \
GNOBLIN_DEVKIT_EXEC='printf "runtime_pid=%s runtime_dir=%s\n" "$GNOBLIN_DEVKIT_HOST_PID" "$XDG_RUNTIME_DIR"; exec sleep 600' \
  scripts/run-clean-devkit.sh >"$probe_dir/preview.log" 2>&1 &
wrapper_pid=$!
cat "$probe_dir/preview.log"
ps -eo pid,ppid,tid,stat,args --forest
```

Record the compositor PID and its direct `mutter-devkit` child from this
private run, including process start times. Check `/proc/PID/exe` against the
candidate prefix and the viewer's `PPid` against that compositor. Do not select
by process name alone or assume a runtime supervisor PID is the compositor.

With `compositor_pid` and `viewer_pid` set to that verified pair, trace the
viewer in a second guest terminal:

```sh
strace -tt -o "$probe_dir/viewer.trace" \
  -e trace=exit,exit_group -e signal=SIGTERM -p "$viewer_pid"
```

Record the private run's guardian PID as `guardian_pid`, alongside the verified
wrapper, compositor and viewer. Pause both cleanup processes during the
SIGKILL check so cleanup cannot mask a missing kernel lifetime link. Keep a
resume trap:

```sh
trap 'kill -CONT "$wrapper_pid" "$guardian_pid" 2>/dev/null || true' EXIT INT TERM
kill -STOP "$wrapper_pid" "$guardian_pid"
kill -KILL "$compositor_pid"
python3 - "$viewer_pid" <<'PY'
from pathlib import Path
import sys
import time

process = Path("/proc") / sys.argv[1]
deadline = time.monotonic() + 5
while process.exists() and time.monotonic() < deadline:
    time.sleep(0.05)
if process.exists():
    raise SystemExit("FAIL: viewer still present after five seconds")
print("PASS: recorded viewer is gone")
PY
kill -CONT "$wrapper_pid" "$guardian_pid"
wait "$wrapper_pid"
```

Require the recorded viewer to exit from SIGTERM in the trace and disappear
after reaping. Record a timeout as failure before doing targeted cleanup. Repeat
with a fresh pair and compositor SIGTERM. Keep a second private pair running
to verify isolation, then close it normally. Repeat several cycles and check
the recorded private runtime directories and child PIDs for leftovers.

For future changes to the spawn path, repeat main's fork/child-setup/exec trace
to verify the creator is still the compositor main thread. The local fixture
covers death before signal installation; the native startup barrier above
covers owner death after EIS fd delivery. Retain the unmodified normal-startup
control and both artifact identities when repeating the native race.

## Scope and limits

The patch affects only the direct viewer spawn and retains the existing
`GNOBLIN_DEVKIT_KEEP_SESSION` handling of viewer-first exit. No wrapper,
public setting, or process-name cleanup is added. Without `HAVE_SYS_PRCTL`,
the existing behavior remains. This is a Linux signal-delivery safeguard.

SIGTERM remains catchable. The inspected viewer source has no SIGTERM handler,
and the native candidate exited in main's observed runs. This does not promise
unconditional bounded exit, supervise daemonized descendants, or itself reap
zombies. GLib reaps while the owner lives, and the adopting reaper handles
owner death. The safeguard alone does not establish that #191 is resolved.

Main owns build integration, the real compositor/VM proof, issue updates, and
any shared-document edits. The later
`python3 scripts/manage-patches.py check mutter` passed; the previously
reported unrelated 0123 author-header failure is no longer outstanding.

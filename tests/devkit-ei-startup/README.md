# Devkit EIS startup disconnect

This regression covers the actual `mdk_ei_new()` constructor receiving a
disconnect before its first seat. Patch 0009 makes that constructor return a
connection-closed error instead of spinning. It applies independently of
0008's parent-death signal safeguard.

## Run

From the Gnoblin root, with Python 3, Git, a C compiler, `pkg-config`, GLib/GIO,
libei and libeis development files installed:

```sh
python3 tests/devkit-ei-startup/run.py --baseline --case closed-peer
python3 tests/devkit-ei-startup/run.py --baseline --case rejected
python3 tests/devkit-ei-startup/run.py --baseline --case seat
python3 tests/devkit-ei-startup/run.py
CC=clang python3 tests/devkit-ei-startup/run.py --sanitize
similarity-py tests/devkit-ei-startup tests/devkit-viewer-lifecycle
```

The first two commands must fail after observing a real
`EI_EVENT_DISCONNECT`: the baseline constructor does not return within the
fixture's three-second bound. The baseline seat-success control must pass.
The patched run must pass all five cases. The timeout bounds the test process;
the production patch adds no timeout or delay.

The runner reads the pinned Mutter sources from `src/gnome-versions.json`, applies
only 0009 in a temporary repository inside this test directory, and compiles
the complete `mdk-ei.c` with its real headers. It neither changes the shared
submodule nor applies 0008. Temporary files are removed, and timed-out fixture
processes are killed and waited for. No compositor, display, bus, or installed
artifact is used.

## Failure mode and error propagation

`mdk-context.c:mdk_context_activate()` calls `init_session()` before registering
its RemoteDesktop name watcher. Session initialization obtains an EIS fd from
the compositor and transfers it to `mdk_ei_new()` with `g_steal_fd()`.

The constructor waits synchronously for `default_seat`. Previously,
`process_events()` consumed and ignored `EI_EVENT_DISCONNECT`; subsequent
dispatches could not produce a seat, so the constructor never returned. The
normal name-vanished shutdown callback was not yet registered.

libei documents disconnect as the terminal event, including when the server
vanishes without sending one. Once received, the EI context should be
released. The local libei 1.5.0 experiment confirmed that backend setup can
succeed on a socket whose peer has already closed, then deliver only this
event. See the `EI_EVENT_DISCONNECT` and `ei_setup_backend_fd()` contracts in
[`libei.h`](https://libinput.pages.freedesktop.org/libei/api/libei_8h.html).

0009 makes `process_events()` return false on disconnect and makes the
constructor return `NULL` with `G_IO_ERROR_CONNECTION_CLOSED`. An automatic
object reference releases the partial MdkEi, attached source, seat table and
libei context. libei owns the transferred fd and closes it. Successful
construction transfers the object to the caller with `g_steal_pointer()`.
The existing backend-setup failure path gains the same cleanup.

The caller API already propagates this result:

1. `mdk-session.c:init_session()` stores the result in `session->ei` and returns
   false if it is null, preserving the error.
2. `mdk_session_initable_init()` returns false to `g_initable_new()`, which
   releases the failed session. Its existing finalizer releases the D-Bus
   proxies and stops the remote session; `session->ei` remains null.
3. `mdk-context.c:init_session()` emits the existing `error` signal with that
   error. `mdk-main.c:on_context_error()` reports it and exits with failure.

No caller or public signature changes are needed. Post-startup source dispatch
keeps its existing behavior, including the D-Bus shutdown path. A live server
that never supplies a seat or a disconnect still waits as before; this patch
handles a terminal disconnect, not arbitrary startup delays.

## Fixture coverage and boundaries

| Case          | Evidence required                                                                                                                        |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `closed-peer` | A real UNIX socket peer closes before backend setup; constructor returns null and connection-closed error.                               |
| `null-error`  | The same failure returns null and cleans up when the caller supplies no error destination.                                               |
| `repeat`      | Thirty-two failures finalize all thirty-two objects and remove each source and original socket.                                          |
| `rejected`    | A live libeis server rejects the handshake before advertising a seat; constructor returns the same error while the server remains alive. |
| `seat`        | A real libeis server advertises a seat; constructor succeeds and the caller can release it normally.                                     |

The fixture observes real libei events, object finalization and source removal.
Socket cleanup compares descriptor identity because GLib may reuse a closed
socket's fd number for its main-context eventfd.

In `rejected`, the server sends `eis_client_disconnect()` and continues its
event loop. The fixture stops that server only after the constructor has
returned and cleanup assertions have passed. No parent process or creator
thread dies to trigger the result, and 0008 is not present.

Only the UI-facing MdkSeat wrapper is replaced with a small GObject that owns a
real `ei_seat` reference. The server creates no input devices. libei dispatch,
socket setup, event retrieval, the constructor and its finalizer are real.
The fixture does not execute GTK, the full MdkSession caller, or the application
error handler; their propagation is source-reviewed and remains a native check.

On 2026-10-13, with libei/libeis 1.5.0 and GIO 2.86.5, the closed-peer and
rejected baselines observed disconnect and timed out. The baseline seat control
passed. All five patched cases passed, including Clang address/undefined
sanitizers with leak detection. The local default compiler's sanitizer runtime
libraries were missing; no packages were installed. Python similarity review
across this fixture and the 0008 fixture found no duplicate functions.

## Native VM evidence

On 2026-10-13, the actual viewer was paused at `ei_setup_backend_fd()` after
receiving the compositor's EIS fd. An `LD_PRELOAD` barrier called the real setup
function, then used `shutdown(fd, SHUT_RDWR)` before returning. No libei return
value or event was substituted. The compositor stayed alive, and patch 0008 was
absent from its loaded library. `GNOBLIN_DEVKIT_KEEP_SESSION=1` prevented the
viewer exit from closing this diagnostic compositor.

The original viewer consumed 1.94 CPU seconds during a two-second observation
and never returned from startup. Three patched trials each exited in about
10 ms, logged the real `EI_EVENT_DISCONNECT`, and reported
`EIS disconnected before providing a seat` through the application error path.
Every viewer was reaped. The same patched executable started normally without
the injected shutdown and acquired its default seat. The installed original
viewer was restored after the trials.

Artifact hashes:

- Original viewer: `268069a44dd1b842ef9b19785634cb8baac6402cc10c6e9b20e4c14ae69e3e66`.
- Patched viewer: `7d0507689d9e8ba00bf0c1925d912ba8ed7262de0fede8af4d2657e57cb920ac`.
- Unchanged compositor library: `2bf1f1725cbcedcc9ce600b8a53b2817814497b877f15fc7de9d61551a467167`.

The host evidence directory is
`~/.local/state/gnoblin-qemu-e2e/20261012T140506Z/`.
`astra-eis-native-after-final.log` records the three passing trials and normal
startup control. Guest JSON, process maps and runtime logs are under
`/home/luna/astra-native-20261013/eis-disconnect-after-candidate-{2,3,4}.*`.
`eis-disconnect-after-baseline-1.*` records the original failure.

An earlier injection shut down the fd _before_ backend setup. In this native
path, setup returned zero but neither executable received a libei event, so
both spun. The same operation on a local socket-pair fixture produced an event.
That discrepancy remains unexplained; it is not evidence against the observed
terminal-event fix, nor a claim that every possible transport failure is covered.
The earlier failed logs are retained. A separate first post-setup run exited
correctly but read the asynchronously forwarded error log too early; subsequent
runs waited for the log and passed all assertions.

Patch 0008 separately handles compositor death during this startup interval.
Its native baseline used 1.97 CPU seconds in two seconds after parent death;
the parent-death signal candidate exited immediately. See the
[lifecycle fixture](../devkit-viewer-lifecycle/README.md). Neither reproduction
establishes the cause of the historical long-lived viewer in issue #191.

To repeat the native check, retain the barrier before real backend setup, but
perform the injected shutdown **after** the real call returns. Record the real
return value, event and application error. Pause only the wrapper, leaving the
guardian available to forward logs. Verify the compositor PID and start time
remain unchanged through the viewer exit. Resume and reap recorded children in
cleanup, restore the original artifact, and repeat without injected shutdown.

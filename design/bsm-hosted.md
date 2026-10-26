# Hosted mode for an external session manager

Status: implemented on `feat/bsm-hosted`. User-facing description:
[docs/hosted.md](../docs/hosted.md). This note records how Gnoblin starts today,
why hosted mode is shaped as it is, what was measured, and what is not verified.

## How Gnoblin starts

One executable, `gnoblin`, plays four roles. `src/main.c` picks the role from the
arguments.

| Role       | Chosen by                       | Job                                                           |
| ---------- | ------------------------------- | ------------------------------------------------------------- |
| Guardian   | no internal flag                | Owns the session for its whole life. `session_guardian_main`. |
| Supervisor | `--internal-session-supervisor` | Restartable host of the Lua worker. Performs power actions.   |
| Worker     | `--internal-runtime-worker`     | Runs the Lua configuration and talks to Mutter.               |
| Compositor | `--gnoblin-runtime-fd=N`        | Mutter plus Gnoblin's patches. `gnoblin_compositor_main`.     |

The guardian (`src/session/gnoblin-runtime.c`):

1. Parses `--config`, `--xwayland`, `--greeter`, `--devkit` and
   `--wayland-display`, and exports the session class.
2. Unless devkit, prepares the environment (`prepare_session_environment`): it
   requires a fresh Wayland login, then points `PATH`, `LD_LIBRARY_PATH` and
   `XDG_DATA_DIRS` at the install prefix.
3. Claims `org.gnoblin.SessionSupervisor` on the session bus.
4. Creates a `SOCK_SEQPACKET` pair for the private runtime protocol and spawns the
   compositor (the same binary, `--wayland --gnoblin-runtime-fd=198`).
5. Unless devkit, runs `activate_session` while the compositor starts: it pushes
   the desktop identity into the D-Bus activation environment and the user's
   systemd manager and starts `gnoblin-session.target`.
6. Spawns the supervisor, which spawns the worker. It restarts the supervisor up
   to five times and keeps Mutter alive if recovery fails.
7. Reads one-byte status messages: `STARTED`, `READY`, `LOGOUT` and
   `SESSION_LIFECYCLE_SUPPORTED`. `LOGOUT` ends the guardian with status 0. A
   compositor exit ends it with a failure status. `SIGTERM` gives 143.

### What `READY` means

The worker sends `GUARDIAN_STATUS_READY` in `handle_runtime_packet`
(`gnoblin-runtime-compositor.c`) when it accepts Mutter's `HELLO`: role
`compositor`, document version 1, and the compositor's session environment
(`WAYLAND_DISPLAY`, `DISPLAY`, `XAUTHORITY`). It sends it before the user's Lua
configuration is loaded; loading follows in an idle callback. So `READY` means:

- Mutter completed the private runtime handshake and serves its Wayland socket;
- the worker has the compositor's environment and has queued the autostart
  snapshot.

It does not mean the user's configuration is active, that autostart entries
started, or that any shell surface exists. The guardian then publishes session
lifecycle `running` to Mutter, which raises `gnoblin.session.state-changed`. That
event has no snapshot read, so a late subscriber cannot learn the state from it.
Hosted mode therefore publishes `Ready` as a readable property.

### The devkit path

`--devkit --wayland-display NAME` skips `prepare_session_environment` and
`activate_session`. The compositor runs with `--virtual-monitor 1280x720@60
--devkit`. Mutter's devkit option selects the headless native backend and creates
`MetaMdk` (`src/core/meta-mdk.c`), which exports `org.gnome.Mutter.Devkit` and
launches `mutter-devkit` as a client of the host Wayland display
(`WAYLAND_DISPLAY` at start). The viewer talks to ScreenCast and RemoteDesktop.
When it exits the compositor ends, unless `GNOBLIN_DEVKIT_KEEP_SESSION` is set.

### Headless backend, virtual monitors, input

Mutter's own `--headless` option (`meta-context-main.c`) selects
`META_BACKEND_NATIVE_MODE_HEADLESS`: no launcher, no logind seat, no DRM master,
and a virtual input seat. It still renders on a GPU render node, observed as
`/dev/dri/renderD128`. With `--headless` alone, no `MetaMdk` exists and no
viewer starts.

- `org.gnome.Mutter.ScreenCast.CreateSession` and `RecordVirtual` add a virtual
  monitor. The PipeWire consumer's negotiated size sets the monitor size.
  Connectors are `Meta-0`, `Meta-1`. A stream that joins an already started
  session needs `Stream.Start` (reported by the bridge).
- `org.gnome.Mutter.RemoteDesktop.ConnectToEIS` returns the EIS socket for input.
- `org.gnome.Mutter.DisplayConfig` reads and sets positions and logical scale.
  The scales a mode allows are in `GetCurrentState`; 1280x720 allowed 1.0, 1.25
  and 1.333.

No Gnoblin or Mutter access list stands in the way of a same-user client:
`MetaDbusAccessChecker` is used only by the accessibility manager. ScreenCast,
RemoteDesktop and DisplayConfig have no sender checks, and the session bus admits
only the user's own processes. No allow-list change was needed.

## Decisions

**A distinct flag, one enum.** `--hosted` and `--devkit` cannot be combined with
each other or with `--greeter`. The guardian, supervisor and worker carry a
`GnoblinNestedMode` instead of a `devkit` boolean, and validate the display name in
one function. Reusing the devkit boolean would have implied a viewer and the
devkit's fixed monitor.

**No Mutter patch.** Hosted mode passes Mutter's existing `--headless` and
`--wayland-display`. It adds no new Mutter behaviour, so the patch series and
the pinned fork are unchanged.

**Environment steps kept, seat steps skipped.** The user's `systemd --user` and
D-Bus session still need the display name, `graphical-session.target` and the
portal restart, so hosted mode runs `prepare_session_environment` and
`activate_session`. Neither takes a seat or a device. Only the requirement for
`XDG_SESSION_ID` is relaxed, because a manager that opens the session itself may
not export it to this process. Devkit mode skips both functions, because its
launcher provides the environment.

**Readiness on the session bus, not a file or fd.** The manager and Gnoblin are
separate processes started side by side, so an inherited fd does not fit. The
guardian already owns a session-bus connection. `org.gnoblin.Hosted1` is exported
from a thread with its own main context, because the guardian loop does not
iterate the default context. A hosted guardian that cannot publish it exits.
`Ready` never returns to false: a supervisor restart leaves Mutter serving.

**Surfaces through the existing control API.** `layers.list` and the layer
lifecycle events already report namespace, output and mapped state. A second
interface would duplicate them. `layer.list` (singular) is a legacy shape with
only `id`, `namespace` and `title` (`legacy_layer_list_from_lua`), so integrators
must call `layers.list`.

**Lock stays an event.** Gnoblin never starts a locker. Every lock source ends in
`gnoblin.session.lock-requested` for subscribed clients, so a manager that
subscribes receives it unchanged; with no subscriber the request fails. Hosted
mode does not remove the `ext-session-lock-v1` global. Removing it is the
protocol setting `ext_session_lock = false`, which the shell's hosted
configuration sets. Removing it in code would need a Mutter patch, and standalone
behaviour must stay as it is.

**Logout and power are reported, not performed.** Two paths exist:

- `gnoblinctl session suspend|hibernate|reboot|poweroff` call logind from the CLI
  and never reach the compositor, so `gnoblinctl` checks for `org.gnoblin.Hosted1`
  first and calls its `RequestAction` method.
- The Lua operations (`gnoblin.session.*`) finish in the worker. In a hosted
  session `handle_completion` calls `RequestAction` instead of exiting the worker
  with a logout or power status.

`RequestAction` validates the action and re-emits `ActionRequested` from the
guardian, so the name owner speaks for the session. Any process of the user can
call it. That is the same trust as `gnoblinctl` itself.

## Verification

`tests/hosted-session.py` ran 55 checks, all passing, against a stage built from
this branch (private bus, private PipeWire, system bus pointed at a missing
socket):

- the command line rejects bad hosted invocations;
- `org.gnoblin.Hosted1` appears, `Ready` flips false to true with
  `PropertiesChanged`, and `WaylandDisplay` and the sockets exist;
- no viewer name or process; the compositor runs with `--headless`;
- two `RecordVirtual` monitors (1280x720 and 800x600) deliver frames; layer-shell
  wallpaper, bar and dock and a `foot` toplevel appear in the frame pixels at
  logical scale 1.25, with monitor B positioned at the scaled logical edge;
- `layers.list` and the created and removed events report namespace, output and
  mapped; removing a monitor removes its surfaces;
- RemoteDesktop hands out an EIS socket;
- lock fails with no subscriber and succeeds once subscribed, and reboot,
  power off, suspend, hibernate and logout arrive as `ActionRequested` while the
  session keeps running.

Other checks that ran: `tests/session-environment.test.py`,
`tests/greeter-mode.test.sh` against the staged binary, and the fast checks that
need no portal source (config seed, frame renderer policy, package isolation,
build dependency, keybinding catalog, GNOME version manifest).

## Not verified

- Input delivered to a client through EIS. The test proves the socket is
  handed out; the bridge's own test covers delivery.
- A real `systemd --user` manager. The private bus has none, so
  `session_systemd_available` was false and the target start was not run. In that
  case startup waits 10 seconds for the user manager before it spawns Mutter.
- DMA-BUF consumers. The test consumer negotiates memory-mapped buffers.
- WirePlumber linking. The private WirePlumber could not link the video stream
  (a script error on `media.type`), so the test links ports itself with `pw-link`.
- The `--devkit` path at runtime. The stage was built without the viewer; the
  devkit code path compiles and its argument handling is unchanged.
- The portal build. The stage was built with `--without-portal`, so
  `tests/test-rpm-sources.sh` and `tests/gnoblinctl-cli.test.py` (which expect
  portal sources) did not run.
- Monitor hotplug under load, GPUs other than amdgpu, and several hosted
  sessions of different users at once.

## Observed behaviour to plan around

- A DisplayConfig layout change after shell surfaces exist can move or resize
  them. A bar moved from `Meta-1` to `Meta-0` once. Apply scale and position
  before the shell starts.
- A virtual monitor sends frames only when its content changes.
- Surfaces on a removed monitor disappear from `layers.list`.

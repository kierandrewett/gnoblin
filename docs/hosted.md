# Hosted sessions

Run Gnoblin as one user's session inside a program that owns the display, such
as a system compositor with a login screen. The hosting program decides which
session is visible and who gets input. Gnoblin keeps managing windows,
workspaces, shell surfaces and applications for that user.

Use `--hosted` when another program creates the outputs and forwards input. To
try Gnoblin in a window on your desktop, use the [devkit](/devkit). To run it
as a normal login, see [installation](/installation).

## Start a hosted session

Start Gnoblin as the session user, with that user's session bus and runtime
directory:

```sh
gnoblin --hosted --wayland-display my-session
```

The session has no monitors until the hosting program adds them. Programs in
the session connect to `$XDG_RUNTIME_DIR/my-session`.

The command line takes these options:

| Option                   | Accepted values                 | Purpose                                                   |
| ------------------------ | ------------------------------- | --------------------------------------------------------- |
| `--hosted`               | None                            | Run headless for an external manager.                     |
| `--wayland-display NAME` | 1 to 128 of `A-Z a-z 0-9 _ . -` | Name of the session's Wayland socket.                     |
| `--config PATH`          | Lua file                        | Load this configuration instead of the default.           |
| `--xwayland`             | None                            | Start Xwayland. This is the default when it is installed. |
| `--no-xwayland`          | None                            | Do not start Xwayland.                                    |

`--hosted` requires `--wayland-display`. `.` and `..` are not valid names. It
cannot be combined with `--devkit` or `--greeter`.

## Environment

Set these before starting Gnoblin:

- `XDG_RUNTIME_DIR`, the user's runtime directory.
- `DBUS_SESSION_BUS_ADDRESS`, a running session bus for the same user.
- `XDG_SESSION_TYPE=wayland`.

Leave `WAYLAND_DISPLAY` and `DISPLAY` unset. Gnoblin refuses to start when
either is set. `XDG_SESSION_ID` is optional because a hosted session has no
seat.

### Startup waits

The first start on a loaded machine can take over 20 seconds, mostly in Mutter.
A hosted session waits up to 60 seconds for each step of startup instead of 10.
If a step still times out, Gnoblin restarts its Lua runtime with the built-in
defaults, so your autostart entries do not run. Set
`GNOBLIN_HOSTED_STARTUP_WAIT_MS` to change the wait:

| Variable                         | Accepted values               | Default |
| -------------------------------- | ----------------------------- | ------- |
| `GNOBLIN_HOSTED_STARTUP_WAIT_MS` | 2000 to 900000 (milliseconds) | 60000   |

A value outside the range is ignored with a message on standard error.

## How hosted mode differs

| Mode               | Outputs                                | Viewer      | Seat and logind device control |
| ------------------ | -------------------------------------- | ----------- | ------------------------------ |
| Standard (default) | Monitors the session owns              | None        | Taken                          |
| `--devkit`         | One 1280x720 monitor shown in a window | Opens       | Not taken                      |
| `--hosted`         | Virtual monitors added by the manager  | Never opens | Not taken                      |

A hosted session still joins the user's systemd and D-Bus session. It
registers its Wayland display for D-Bus-activated programs and runs
autostart entries.

These differ from a standard session:

- Logout, restart, power off, suspend and hibernate requests do not end the
  session or call logind. They are reported to the manager, which decides.
- Gnoblin starts no lock screen. A lock request is reported as an event.

## Learn that the session is ready

Gnoblin owns `org.gnoblin.Hosted1` on the session bus at
`/org/gnoblin/Hosted1`. The name appears when Gnoblin starts. Wait for `Ready`
before treating the session as usable.

| Member                    | Type    | Meaning                                                                               |
| ------------------------- | ------- | ------------------------------------------------------------------------------------- |
| `Ready`                   | boolean | The compositor finished its startup handshake.                                        |
| `WaylandDisplay`          | string  | The `--wayland-display` name.                                                         |
| `ActionRequested(action)` | signal  | A session program asked for `logout`, `reboot`, `poweroff`, `suspend` or `hibernate`. |
| `RequestAction(action)`   | method  | Report one of those actions. `gnoblinctl session` calls it.                           |

`Ready` changes through `org.freedesktop.DBus.Properties.PropertiesChanged`.
When it is true, the compositor accepts Wayland clients on the session socket.
It stays true while Gnoblin restarts its Lua runtime.

`Ready` does not mean the shell has painted anything. A shell reports that
itself, and the surface list below shows what is mapped. If the name leaves the
bus, the session has ended.

## Add monitors and input

The manager uses the Mutter services on the session bus. Gnoblin applies no
extra access list to a client of the same user.

- `org.gnome.Mutter.ScreenCast.RecordVirtual` creates a virtual monitor. The
  size the PipeWire consumer negotiates becomes the monitor size. Monitors are
  named `Meta-0`, `Meta-1` and so on. Ending the ScreenCast session removes the
  monitor.
- `org.gnome.Mutter.DisplayConfig` sets each monitor's position and logical
  scale. Call `GetCurrentState` to read the scales a monitor allows.
- `org.gnome.Mutter.RemoteDesktop.ConnectToEIS` returns a socket for keyboard
  and pointer input.

A virtual monitor sends a frame when its content changes. A still desktop does
not produce a steady frame rate.

## Read shell surfaces

The compositor control socket at `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`
lists layer-shell surfaces. Request API 1.37 or newer and call `layers.list`.
Each record has these fields:

- `namespace`, the surface's layer-shell namespace.
- `monitor_id`, the name of its output, such as `Meta-0`.
- `layer` and `geometry`, its stacking layer and logical rectangle.
- `mapped`, whether it has content on screen.

The [bridge reference](/bridge/session-and-windows#layer-surfaces) describes the
connection and the remaining fields. Do not use `layer.list`; it is a legacy
call that returns only the ID, namespace and title.

To get changes, subscribe to `gnoblin.layer.created`, `gnoblin.layer.changed`
and `gnoblin.layer.removed`. Mapping and unmapping arrive as `changed`. See
[layer lifecycle events](/bridge/shortcuts-and-events#api-version-171-layer-lifecycle-events).

To tell whether a shell is ready on every output, check that each required
namespace has a mapped record whose `monitor_id` matches each monitor name.

## Handle lock and power requests

These requests ask the session to lock:

- `gnoblinctl session lock`
- `gnoblin.session.lock()` in Lua
- `org.freedesktop.ScreenSaver.Lock`
- logind's lock signal

Gnoblin sends `gnoblin.session.lock-requested` to clients subscribed on the
control socket. With no subscriber the request fails, so subscribe before the
session starts. See [session locking](/session-lock).

Hosted mode does not change the `ext-session-lock-v1` protocol. If the manager
locks the session itself, hide the protocol so no program in the session starts
a second locker. Add this to the session's configuration, as described in the
[protocol settings](/config/configure/protocols):

```lua
gnoblin.configure { protocols = { ext_session_lock = false } }
```

For the other requests, subscribe to `ActionRequested`:

```sh
gdbus monitor --session --dest org.gnoblin.Hosted1
gnoblinctl session logout
```

The monitor prints a line that ends with `ActionRequested ('logout')`. A lock
request does not use this signal; subscribe on the control socket instead.

## Stop the session

Send `SIGTERM` to the `gnoblin` process. It stops the compositor and removes
`org.gnoblin.Hosted1`. If the compositor stops on its own, `gnoblin` exits with
a failure status.

## Troubleshooting

| Problem                                       | Next step                                                                              |
| --------------------------------------------- | -------------------------------------------------------------------------------------- |
| `--hosted needs a valid Wayland display name` | Pass `--wayland-display` with an allowed name.                                         |
| `start it from a fresh Wayland session`       | Unset `WAYLAND_DISPLAY` and `DISPLAY`, and set `XDG_SESSION_TYPE=wayland`.             |
| `could not publish org.gnoblin.Hosted1`       | Another Gnoblin session of this user holds the name, or the bus is missing.            |
| Startup waits about 10 seconds                | The system uses systemd but the session bus has no user manager. Gnoblin waits for it. |
| A lock request fails                          | Subscribe to `gnoblin.session.lock-requested` on the control socket.                   |
| No frames from a new monitor                  | Check that a PipeWire consumer is linked to the stream node.                           |

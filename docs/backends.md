# Backends

A backend is where Gnoblin gets its outputs and its input. Gnoblin has three, and one binary carries all of them:

| Backend    | What it is                                                                            | Use it for                                                     |
| ---------- | ------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `drm`      | Your GPU and your monitors, with libinput for the keyboard, mouse and touch.          | Running Gnoblin as your desktop on real hardware.              |
| `nested`   | A window (one per output) of another Wayland compositor.                              | Trying Gnoblin, or a build of it, on the desktop you have now. |
| `headless` | Virtual outputs with no window and no hardware. What it draws can still be read back. | Servers, containers, tests, and any machine with no display.   |

## Choose a backend

The backend is chosen once, when the compositor starts. Two things name it:

- The `GNOBLIN_BACKEND` environment variable, for a session wrapper or a test. It wins over everything else.
- The `compositor.backend` setting of the accepted configuration, which holds the same words. `init.ts` has no object for
  the `compositor` section yet (see [settings with no TypeScript home](/js-settings#dropped-settings)), so the variable is the way to choose.

Both take `auto`, `headless`, `nested` or `drm`. Anything else stops the start and names the accepted values:

```text
GNOBLIN_BACKEND=x11: not a backend; accepted values are auto, headless, nested, drm
```

### What `auto` does

`auto` is the default. It takes the first of these that holds:

1. **A Wayland compositor is serving the display** (the socket that `WAYLAND_DISPLAY`, or `compositor.nested-display`,
   names exists): `nested`. A running compositor owns the screen, so Gnoblin becomes a window of it. This is what
   happens when you start `gnoblin` from a terminal on a GNOME, Sway or KDE desktop.
2. **A login session with a seat, and a GPU** (`XDG_SEAT` or `SEATD_SOCK` is set, and `/dev/dri/card*` exists): `drm`.
   This is the session a display manager or a login on a virtual terminal starts.
3. **Otherwise**: `headless`.

An X11 desktop does not count: `DISPLAY` alone selects nothing, because a nested session is a Wayland client and an X11
server cannot host it.

`auto` never changes its mind after a start fails: a `drm` choice that cannot open the seat is refused with the reason,
not replaced by a session you did not ask for. An explicit value is never second-guessed.

The compositor logs its choice and the reason when it starts (`backend selected ... reason="auto: ..."`), and the
session object carries it:

```sh
gnoblinctl session list
gnoblinctl session get ID backend
```

```text
"nested"
```

`compositor.backend` and the settings below are read when the compositor starts. A change needs a new session.

### Settings for a nested session

| Setting                     | Default           | What it does                                                                                           |
| --------------------------- | ----------------- | ------------------------------------------------------------------------------------------------------ |
| `compositor.nested-display` | `WAYLAND_DISPLAY` | The socket of the compositor that hosts the session: a name in `XDG_RUNTIME_DIR`, or an absolute path. |
| `compositor.nested-outputs` | `1`               | How many windows (outputs) to open, from 1 to 8. They are named `NESTED-1`, `NESTED-2` and so on.      |
| `compositor.nested-width`   | `1024`            | The width an output starts at when the host leaves the size to the session.                            |
| `compositor.nested-height`  | `768`             | The height, likewise. A size the host states always wins, and resizing the window resizes the output.  |

## Run on a desktop

Start a whole session in a window of the compositor you are using:

```sh
GNOBLIN_BACKEND=nested gnoblin
```

The host decides the window. You can move, resize and fullscreen it. Closing the last window logs the session out;
closing one of several only removes that output.

If the host is not reachable, the start stops and names the socket:

```text
the host display /run/user/1000/wayland-9 does not exist: no Wayland compositor is serving it
```

## Run on hardware

```sh
GNOBLIN_BACKEND=drm gnoblin
```

The hardware backend needs four things, and when one is missing it stops with the step that failed and what to do:

| Missing         | What you see                                          | What to do                                                                                   |
| --------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| A seat          | `no seat session: ...` and `there is no seat to take` | Start the session from a display manager, or log in on a virtual terminal (logind or seatd). |
| A GPU           | `this machine has no DRM card node`                   | Load the kernel's KMS driver, or choose `nested` or `headless`.                              |
| Permission      | `the seat refused the GPU node`                       | Make sure the session is the active one on its seat and that you may open the device.        |
| GLES on the GPU | `the GPU cannot give the GLES context ...`            | Use `nested` or `headless` on this machine.                                                  |

It draws with the OpenGL ES renderer only: `compositor.renderer = software` or `vulkan` with the `drm` backend is
refused, naming the setting. To drive a different GPU than the default, set `GNOBLIN_DRM_DEVICE=/dev/dri/cardN`.

**Changing a monitor's mode is off by default.** A mode change blanks a real display and has only been tested without
one, so a mode change on a monitor is refused with a message naming the switch until the session starts with
`GNOBLIN_DRM_MODESET=1`. Changing only the scale needs no switch.

**Check it on your hardware.** The hardware backend depends on your GPU driver and your displays. The steps to
confirm a session on real hardware are in [verify a desktop installation](real-hardware-verification.md).

## What each backend does

A **refused** entry is deliberate, and every backend that refuses a request answers it the same way. Nothing is
faked: a nested session does not pretend to have a gamma table, because a monitor's table is shared by every window on
it.

<!-- backend-status:start -->

| Capability                                                       | Headless                                                  | Nested                                                                            | DRM                                                                          |
| ---------------------------------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Outputs with a mode and a scale at start                         | supported                                                 | supported: one per host window                                                    | supported: one per connected connector                                       |
| Outputs added and removed while running                          | supported                                                 | supported: host windows opened and closed at run time                             | supported: monitors plugged and unplugged are followed                       |
| Modes an output can be driven in, listed                         | supported: the one virtual mode                           | supported: the host window's size is the one mode                                 | supported: the kernel's list                                                 |
| Changing an output's mode                                        | supported: any mode with pixels                           | refused: the host owns the window size                                            | supported: listed modes only, and only with GNOBLIN_DRM_MODESET=1            |
| Choosing an output's scale                                       | supported                                                 | refused: the scale follows the host window                                        | supported                                                                    |
| Composing larger than the display and averaging down             | supported                                                 | supported: the window needs a whole scale or a host viewport                      | supported                                                                    |
| Following the host's scale                                       | n/a: there is no host                                     | supported: fractional scale, or the integer scale of the host output              | n/a: there is no host                                                        |
| The software renderer                                            | supported                                                 | supported                                                                         | refused: scan-out draws with GLES only                                       |
| The OpenGL ES renderer                                           | supported                                                 | supported                                                                         | supported: needs GBM and EGL on a real GPU                                   |
| The Vulkan renderer                                              | supported                                                 | supported                                                                         | refused: scan-out draws with GLES only                                       |
| One frame drawn and presented                                    | supported                                                 | supported: after the host asks for a frame                                        | supported: one frame in flight at a time                                     |
| A frame read back as pixels                                      | supported                                                 | supported                                                                         | refused: scan-out only, the frame is never read back                         |
| The same scene gives the same pixels                             | supported: the reference                                  | supported: equal to headless within the stated tolerance                          | n/a: there is no readback to compare                                         |
| Importing client GPU buffers                                     | supported: GLES and Vulkan; the software renderer refuses | supported: as headless, the same code                                             | supported: the GPU imports the buffers                                       |
| Views drawn on the GPU                                           | supported: GLES                                           | supported: GLES                                                                   | supported: GLES                                                              |
| The renderer memory reported                                     | supported                                                 | supported                                                                         | refused: the scan-out path reports no footprint yet                          |
| Pointer motion and buttons                                       | supported                                                 | supported                                                                         | supported                                                                    |
| Relative pointer motion                                          | supported                                                 | supported: while the session holds a pointer lock                                 | supported                                                                    |
| Scrolling                                                        | supported                                                 | supported                                                                         | supported                                                                    |
| Keys                                                             | supported                                                 | supported                                                                         | supported                                                                    |
| Touch                                                            | supported                                                 | supported                                                                         | supported                                                                    |
| Devices announced when they join and leave                       | supported                                                 | supported                                                                         | supported                                                                    |
| Touchpad swipe and pinch                                         | supported                                                 | supported: when the host offers pointer gestures                                  | supported: gestures come from libinput                                       |
| Tablets                                                          | refused: no input event carries tablet data               | refused: no input event carries tablet data                                       | refused: no input event carries tablet data                                  |
| Held keys, buttons and touches released when input is taken away | n/a: nothing takes the input away                         | supported: the host's keyboard leave                                              | supported: switching away from the session releases held buttons and touches |
| The host's shortcuts yielded to the session                      | n/a: the session owns the seat                            | supported: while the window has the keyboard and the host offers it               | n/a: the session owns the seat                                               |
| Pointer lock and confine for the session's clients               | n/a: there is no host pointer                             | refused: the host's pointer lock is never requested                               | n/a: the compositor itself holds the pointer                                 |
| Gamma tables                                                     | supported: a virtual table, recorded                      | refused: a host monitor's table is shared by every window                         | supported: a table of the wrong size is refused                              |
| Output power                                                     | supported: virtual, recorded                              | refused: a host monitor's power is shared by every window                         | supported                                                                    |
| Clipboard and primary selection shared with the host             | n/a: there is no host                                     | refused: a copy in the session does not reach the host                            | n/a: there is no host                                                        |
| Window title, app id and icon                                    | n/a: there is no window                                   | supported: title, app id and icon (xdg_toplevel_icon_v1 where the host offers it) | n/a: there is no window                                                      |
| Presenting through the host's dmabuf                             | n/a: there is no host                                     | refused: frames reach the host through wl_shm after a readback                    | n/a: there is no host                                                        |
| The person ending the session through the backend's own surface  | supported: asked for through its control handle           | supported: closing the last window                                                | n/a: there is no window to close                                             |
| The device or the host going away is reported once               | supported                                                 | supported                                                                         | supported: the GPU being removed                                             |
| Dropping the backend releases what it holds                      | supported                                                 | supported: the host window goes                                                   | supported: the seat is released                                              |

<!-- backend-status:end -->

## What a nested session shares with its host

A nested session is a Wayland client of the host, with the host's permissions and no more. These follow the host:

- **Input.** The pointer, keyboard and touch of the window arrive as the session's own devices. Keys held when the host
  moves the keyboard focus elsewhere are released.
- **Pointer lock.** When a client in the session locks or confines the pointer, the session asks the host to hold its
  pointer the same way, so a game does not lose the mouse, and the host's relative motion reaches it.
- **Shortcuts.** While the window has the keyboard, the host is asked to leave its own shortcuts (such as the Super key)
  to the session, where the host offers it.
- **Size and scale.** The window is the output: its size is the mode, and the scale follows the host's, fractional where
  the host offers it and whole otherwise.
- **Identity.** The window has a title, an application id and an icon, so the host's window list shows the session.

What does not cross, and why:

- **The clipboard and the primary selection.** A copy in the session does not reach the host, and the reverse. Sharing
  them would hand the host's clipboard to every client of the session.
- **Tablets.** No backend carries tablet data.
- **Gamma and power.** Refused: they belong to the host's monitor, not to the window.
- **Drawing with the host's GPU buffers.** Frames reach the host as shared memory after a readback. A GPU renderer
  therefore costs one readback per frame that a hardware session does not pay.

## What a different host changes

A nested session follows the protocols its host offers. A host that lacks one (fractional scale, pointer lock, window
icons, shortcut inhibition) simply gets less: the session still runs, and the matching row of the table is the thing
that is missing.

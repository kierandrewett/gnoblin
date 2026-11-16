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
- The `compositor.backend` setting of the accepted configuration, which holds the same words. `shell.ts` has no object for
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

| Setting                         | Default           | What it does                                                                                                                               |
| ------------------------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `compositor.nested-display`     | `WAYLAND_DISPLAY` | The socket of the compositor that hosts the session: a name in `XDG_RUNTIME_DIR`, or an absolute path.                                     |
| `compositor.nested-outputs`     | `1`               | How many windows (outputs) to open, from 1 to 8. They are named `NESTED-1`, `NESTED-2` and so on.                                          |
| `compositor.nested-width`       | `1024`            | The width an output starts at when the host leaves the size to the session.                                                                |
| `compositor.nested-height`      | `768`             | The height, likewise. A size the host states always wins, and resizing the window resizes the output.                                      |
| `compositor.nested-decorations` | `auto`            | Whether the session draws a window frame of its own when the host does not: `auto` or `off`. See [The window's frame](#the-windows-frame). |

## Run on a desktop

Start a whole session in a window of the compositor you are using:

```sh
GNOBLIN_BACKEND=nested gnoblin
```

The host decides the window. You can move, resize and fullscreen it. Closing the last window logs the session out;
closing one of several only removes that output. A host that draws no window frame of its own gets a small one from the
session ([The window's frame](#the-windows-frame)).

If the host is not reachable, the start stops and names the socket:

```text
the host display /run/user/1000/wayland-9 does not exist: no Wayland compositor is serving it
```

## The window's frame

A nested session asks its host to draw each window's frame (the title bar, the border and the buttons) and follows the
answer, so the window looks like any other window of the host and no frame is drawn twice:

| The host                                                | The window gets                                                    |
| ------------------------------------------------------- | ------------------------------------------------------------------ |
| Answers server-side                                     | The host's frame, and none of the session's.                       |
| Answers client-side, or does not offer `xdg-decoration` | A small frame the session draws under `auto`; nothing under `off`. |
| Holds the window full screen                            | No frame at all.                                                   |

`compositor.nested-decorations` is read when the compositor starts, and takes one of two values:

| Value            | What the session does when the host leaves the frame to it                              |
| ---------------- | --------------------------------------------------------------------------------------- |
| `auto` (default) | Draws the frame below. A host that draws its own, and a full-screen window, get none.   |
| `off`            | Draws none: the window is bare where the host draws none, as it was before the setting. |

Both ask the host for a server-side frame first, so a host that draws one always does. Any other value stops the
configuration from loading and names the two.

The frame the session draws has:

- a title bar with the window's title and the minimise, maximise and close buttons;
- a thin border to resize from.

You can:

- drag the title bar to move the window;
- double click it to maximise or restore the window;
- right click it for the host's window menu;
- drag a border or a corner to resize.

The frame lies around the session, not over it: the session's output is the window less the frame, and the pointer
positions the session sees are unchanged. A maximised window keeps its title bar and loses its border, and a
full-screen one has none.

The cursor over the frame is the host's own arrow and resize cursors, asked for with `wp_cursor_shape_v1`. Without
that protocol the session sets no cursor over the frame, so the host chooses it.

### What each host does

| Host       | Answer to the server-side request                                                                                |
| ---------- | ---------------------------------------------------------------------------------------------------------------- |
| Gnoblin    | Server-side while its shell draws decorations (`startDecorations()` in the stock desktop), client-side otherwise |
| Sway 1.7   | Server-side: Sway draws its own title bar and the session draws none                                             |
| KDE (KWin) | Server-side, from its protocol support; not run here                                                             |

A Gnoblin host whose shell draws decorations frames the window itself (a title bar, or none under the shell's own
policy), and the session draws nothing.

A Gnoblin host whose shell draws no decorations leaves the frame to the window, so under `auto` the session draws its
own. To get no frame at all, run the session full screen or set `compositor.nested-decorations = "off"`.

::: details GNOME session

GNOME's window manager offers no `xdg-decoration`, so under `auto` the session draws its frame there. That comes from
its source: a GNOME session cannot be run headlessly, so the frame has not been looked at on a GNOME screen.

:::

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

| Capability                                                                            | Headless                                                  | Nested                                                                                               | DRM                                                                                |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Outputs with a mode and a scale at start                                              | supported                                                 | supported: one per host window                                                                       | supported: one per connected connector                                             |
| Outputs added and removed while running                                               | supported                                                 | supported: host windows opened and closed at run time                                                | supported: monitors plugged and unplugged are followed                             |
| Modes an output can be driven in, listed                                              | supported: the one virtual mode                           | supported: the host window's size is the one mode                                                    | supported: the kernel's list                                                       |
| Changing an output's mode                                                             | supported: any mode with pixels                           | refused: the host owns the window size                                                               | supported: listed modes only, and only with GNOBLIN_DRM_MODESET=1                  |
| Choosing an output's scale                                                            | supported                                                 | refused: the scale follows the host window                                                           | supported                                                                          |
| Composing larger than the display and averaging down                                  | supported                                                 | supported: the window needs a whole scale or a host viewport                                         | supported                                                                          |
| Following the host's scale                                                            | n/a: there is no host                                     | supported: fractional scale, or the integer scale of the host output                                 | n/a: there is no host                                                              |
| The software renderer                                                                 | supported                                                 | supported                                                                                            | refused: scan-out draws with GLES only                                             |
| The OpenGL ES renderer                                                                | supported                                                 | supported                                                                                            | supported: needs GBM and EGL on a real GPU                                         |
| The Vulkan renderer                                                                   | supported                                                 | supported                                                                                            | refused: scan-out draws with GLES only                                             |
| One frame drawn and presented                                                         | supported                                                 | supported: after the host asks for a frame; one drawn while it is behind is sent when it answers     | supported: one frame in flight, a change made meanwhile is drawn after it          |
| A frame read back as pixels                                                           | supported                                                 | supported                                                                                            | refused: scan-out only, the frame is never read back                               |
| The same scene gives the same pixels                                                  | supported: the reference                                  | supported: equal to headless within the stated tolerance                                             | n/a: there is no readback to compare                                               |
| Importing client GPU buffers                                                          | supported: GLES and Vulkan; the software renderer refuses | supported: as headless, the same code                                                                | supported: the GPU imports the buffers                                             |
| Views drawn on the GPU                                                                | supported: GLES                                           | supported: GLES                                                                                      | supported: GLES                                                                    |
| The renderer memory reported                                                          | supported                                                 | supported                                                                                            | supported: the scan-out buffers and the GLES context are counted                   |
| Pointer motion and buttons                                                            | supported                                                 | supported                                                                                            | supported                                                                          |
| Relative pointer motion                                                               | supported                                                 | supported: while the session holds a pointer lock                                                    | supported                                                                          |
| Scrolling                                                                             | supported                                                 | supported                                                                                            | supported                                                                          |
| Keys                                                                                  | supported                                                 | supported                                                                                            | supported                                                                          |
| Touch                                                                                 | supported                                                 | supported                                                                                            | supported                                                                          |
| Devices announced when they join and leave                                            | supported                                                 | supported                                                                                            | supported                                                                          |
| Input device settings from the document: speed, tap, scroll, handedness               | supported: recorded per virtual device                    | refused: the host compositor owns the devices and applies its own settings                           | supported: fake libinput devices prove the calls; unproven on hardware             |
| Touchpad swipe and pinch                                                              | supported                                                 | supported: when the host offers pointer gestures                                                     | supported: gestures come from libinput                                             |
| Tablet tools: proximity, motion with pressure, tilt and distance, the tip and buttons | supported: a synthetic stylus, injected like any input    | supported: where the host offers zwp_tablet_v2                                                       | supported: libinput's tool events                                                  |
| Tablet pads: buttons, rings and strips                                                | supported: injected like any input                        | supported: where the host offers zwp_tablet_v2                                                       | refused: the toolkit's libinput source drops pad events, so a pad is not announced |
| Held keys, buttons and touches released when input is taken away                      | n/a: nothing takes the input away                         | supported: the host's keyboard leave                                                                 | supported: switching away from the session releases held buttons and touches       |
| The host's shortcuts yielded to the session                                           | n/a: the session owns the seat                            | supported: while the window has the keyboard and the host offers it                                  | n/a: the session owns the seat                                                     |
| Pointer lock and confine for the session's clients                                    | n/a: there is no host pointer                             | supported: the host is asked to lock or confine its pointer where it offers the protocols            | n/a: the compositor itself holds the pointer                                       |
| Gamma tables                                                                          | supported: a virtual table, recorded                      | refused: a host monitor's table is shared by every window                                            | supported: a table of the wrong size is refused                                    |
| Output power                                                                          | supported: virtual, recorded                              | refused: a host monitor's power is shared by every window                                            | supported                                                                          |
| Clipboard and primary selection shared with the host                                  | n/a: there is no host                                     | supported: while the window has the host's keyboard; text and images, never a secret, 16 MiB a paste | n/a: there is no host                                                              |
| Window title, app id and icon                                                         | n/a: there is no window                                   | supported: title, app id and icon (xdg_toplevel_icon_v1 where the host offers it)                    | n/a: there is no window                                                            |
| Presenting through the host's dmabuf                                                  | n/a: there is no host                                     | supported: through the host's dmabuf when the session draws with GLES, shared memory otherwise       | n/a: there is no host                                                              |
| The person ending the session through the backend's own surface                       | supported: asked for through its control handle           | supported: closing the last window                                                                   | n/a: there is no window to close                                                   |
| The device or the host going away is reported once                                    | supported                                                 | supported                                                                                            | supported: the GPU being removed                                                   |
| Dropping the backend releases what it holds                                           | supported                                                 | supported: the host window goes                                                                      | supported: the seat is released                                                    |

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
- **The clipboard and the primary selection.** See [the clipboard](#the-clipboard-and-what-it-can-leak) below.
- **Input settings.** The `input` section of your configuration is not applied: the devices are the host's, and the
  host applies its own pointer and touchpad settings to them. See [input device settings](#input-device-settings).
- **Tablets.** A drawing tablet the host has is a tablet of the session: the pen's position, pressure, tilt, distance,
  tip and buttons, and the pad's buttons, rings and strips, where the host offers `zwp_tablet_v2`. A pen moves no pointer.
- **GPU buffers.** When the session draws with the GLES renderer and the host can import GPU buffers
  (`zwp_linux_dmabuf_v1`), each frame is drawn into a buffer on the host's device and handed over, with no readback. A
  frame the GPU path cannot take (a supersampled output, a buffer the host refuses, a machine with no render node) goes
  through shared memory, so the window never goes blank.

What does not cross, and why:

- **Gamma and power.** Refused: they belong to the host's monitor, not to the window.
- **Anything the clipboard rules below keep back**: file lists, application types and secrets.
- **A tablet pad on the hardware backend.** The toolkit's libinput source drops a pad's events, so a pad is not
  announced there. The pen is.
- **GPU buffers with the software or Vulkan renderer.** Only GLES draws into a GPU buffer, so those two read each frame
  back into shared memory.

## Input device settings

The `input` section of your configuration (`mouse`, `touchpad`, `trackball` and `pointing-stick`) is given to each
input device the session has when it starts, again to every device after a reload, and to a device when you plug it in.
A line you remove from the section puts that setting back to what the device started with, and a device you unplug is
forgotten.

- **The hardware backend** applies it through libinput. Each setting a device does not have (a mouse has no tap) is left
  out, and a setting a device refuses is written to the log once for that device (`gnoblinctl log`, area `input`)
  while the rest still apply.
- **A nested session** does not apply it. Its devices are the host's, and the host applies its own settings to them.
- **A headless session** has no devices; the settings are recorded for each virtual device a test announces.

What is not applied yet:

| Setting                                           | Why                                                                              |
| ------------------------------------------------- | -------------------------------------------------------------------------------- |
| `mouse.double-click-time`, `mouse.drag-threshold` | The toolkits read them, not a device, and nothing publishes them to the toolkits |
| `mouse.accel-curve`, `touchpad.accel-curve`       | A custom acceleration curve needs libinput 1.23; this build is made for 1.19     |
| `touchpad.disable-while-typing-timeout`           | The timeout needs libinput 1.31                                                  |
| `touchpad.click-method = none`                    | The libinput binding has no value for "no software buttons"                      |
| `tablets`, `styluses`                             | No tablet or stylus override is applied                                          |
| `orientation-lock`                                | There is no automatic rotation to lock                                           |

Every reload names the settings it cannot apply, so `gnoblinctl config reload` says so when you change one of them. Two
details of the touchpad: `scroll-speed` multiplies how far the touchpad scrolls and is not a libinput setting, and
`edge-scrolling-enabled` and `two-finger-scrolling-enabled` choose one scroll method between them (two fingers wins when
both are on).

A trackball and a pointing stick are found by the tags udev gives them (`ID_INPUT_TRACKBALL`, `ID_INPUT_POINTINGSTICK`).
On a machine without those tags they are mice.

What stays unproven without real hardware: the order and values of the libinput calls are proven against recorded fake
devices, and a device that really refuses a setting is not exercised.

## The clipboard, and what it can leak

A nested session shares its clipboard and its primary selection with the host, both ways, under these rules. They are
the answer to one question: a program in the session must not be able to read or change what you copied on the host
without you having it in front of you.

- **Only while the window has the host's keyboard.** The host offers its clipboard to the window it is typing into, so
  the session sees it from the moment you click into the window until you leave. Leaving takes it away.
- **A copy belongs to the moment it was made.** A program in the session that sets the clipboard while you are elsewhere
  does not touch the host's, not when it sets it and not later, when you come back to the window.
- **A copy made as you leave can be lost.** A copy reaches the host a moment after the program makes it, and a host may
  refuse one that arrives after the keyboard has gone elsewhere. This repository's compositor does.
- **Nothing is read until somebody pastes.** The host's clipboard is a list of types until a program asks for one, so a
  gigabyte copy that nobody pastes costs nothing.
- **Only these types cross:** `text/plain` (with or without a charset), `UTF8_STRING`, `STRING`, `TEXT`, `text/html`,
  `image/png` and `image/jpeg`. File lists (`text/uri-list`), application types and anything else stay where they were
  made, because they carry host paths and identities.
- **A secret never crosses.** A password manager marks a copy as secret by offering the `x-kde-passwordManagerHint`
  type. A copy that offers it is not shared at all, in either direction.
- **At most 16 MiB a paste**, and a paste that goes 10 seconds without a byte ends. The program reading gets what moved
  before the limit, as it would from any pipe that closed.
- **Not while the session is locked.**

Every program of the session that can read the clipboard (any program in the focused window, and any clipboard manager
you run in the session) can read what crosses while the window is focused. Do not copy something you would not paste into
the session's windows.

## Real hosts

The nested backend is tested against this repository's own compositor, and against Sway 1.7 and Weston 10 running
headless (`scripts/real-host-conformance.sh sway`, or `weston`). A headless real host can show:

- the window's size and scale, and frame pacing;
- the title and application id the host's window list shows;
- the integer scale: Sway states a fractional scale as the whole scale above it, and the session follows that;
- the clipboard from the host to the session. The other way needs an input device for the serial a copy names, and a
  headless host has none.

It cannot show tablets, a pointer of yours to lock, or the dmabuf path, which has no GPU here. Those are proven against
the repository's host only.

Weston's headless backend has no input device and so offers no `wl_seat`. A nested session needs one to take input from,
and a host without it is refused by name (`the parent does not offer wl_seat`). Against Weston only that refusal is
checked. Sway's headless backend does offer a seat.

## What a different host changes

A nested session follows the protocols its host offers. A host that lacks one (fractional scale, pointer lock, window
icons, shortcut inhibition) simply gets less: the session still runs, and the matching row of the table is the thing
that is missing.

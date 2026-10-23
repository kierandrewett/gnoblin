# Portal screenshot regression

Issue [#142](https://github.com/kierandrewett/gnoblin/issues/142) is an absent
GNOME Shell capture service after screenshot consent. These fixtures run on
private buses and do not operate the host or VM desktop.

## Cases defined before implementation

- The current Shell transport fails and leaves its request pending. Run
  `python3 tests/portal-screenshot/reproduce-shell.py`; exit 1 is the expected
  pre-fix result. It compiles the real upstream capture call and callback,
  replacing only image presentation with an observation.
- Capture succeeds only with a complete owned image and a private PNG file.
- Cancellation during CreateSession, RecordArea, Start, or frame acquisition
  releases the session, including a session whose creation reply arrives late.
- A closed or inhibited compositor session, unavailable service, lost
  connection, or frame timeout yields failure without a file.
- Malformed dimensions, strides, offsets, short/corrupted chunks, unsupported
  formats, and unexpected crop metadata cannot expose out-of-bounds pixels.
- Every Screenshot request shows the consent dialog; no policy or frontend hint
  skips it. Close completes once, unexports the request, and cancels capture.
  Another bus peer cannot close a request it does not own.

The pixel fixture uses real SPA buffer layouts and the production reader.
Transport and request fixtures must distinguish mocks from a native-session
proof. Main owns the latter; no passing local fixture establishes VM success.

## Run the local checks

The C fixtures require GTK4, GLib/GIO, GdkPixbuf, PipeWire development files,
`gdbus-codegen`, and a C compiler. Python fixtures require PyGObject. All are
already build dependencies or local test tools; the runner installs nothing.

```sh
python3 tests/portal-screenshot/run.py --keep /var/tmp/portal-142-check
CC=clang python3 tests/portal-screenshot/run.py --sanitize --integration
python3 tests/portal-screenshot/frontend.py /var/tmp/portal-142-check/backend
dbus-run-session -- python3 tests/portal-screenshot/pipewire.py /var/tmp/portal-142-check/capture
```

The frontend check uses the installed `xdg-desktop-portal` and permission store
on a private bus with temporary XDG directories. Access consent and image
production are substitutes; screenshot handling and request export/close are
production code. It checks the public `Response`
signal and verifies that `Close` emits no response.

The PipeWire check requires `gst-launch-1.0`, `videotestsrc`, `pipewiresink`,
`pw-dump` and `pw-link`. It starts a private PipeWire daemon and a synthetic
red source, then links only that source to the real capture adapter. The
Mutter D-Bus service is a stand-in. It checks PNG dimensions, RGB pixels,
mode 0600 and session teardown, without a desktop or device monitor.

Build the complete backend in a new directory, without changing the shared
upstream tree or installing anything:

```sh
python3 tests/portal-screenshot/build.py /var/tmp/portal-142-build \
  --prefix /usr/local/lib/gnoblin
```

The script archives the pinned upstream revisions, copies the overlays,
applies every portal patch, builds the backend and runs its Meson tests.
It refuses an existing output directory and does not fetch missing sources.

## Source facts and choice of capture route

These facts were checked against the pinned source and primary protocol
files before implementation:

- [Mutter ScreenCast XML at the pinned commit](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/data/dbus-interfaces/org.gnome.Mutter.ScreenCast.xml)
  defines session creation, start, stop and closure, area recording, and the
  PipeWire node announcement. `RecordArea` takes stage coordinates; stream pixel
  size can differ with monitor scale. This is a private API for the integrated
  libmutter system, with no cross-version compatibility promise. Gnoblin pins
  both components and already uses this interface for screencasting.
- [Mutter's session manager](https://gitlab.gnome.org/GNOME/mutter/-/blob/138a14fbeef09d49ebf5be8a0cb83b042dd5c841/src/backends/meta-dbus-session-manager.c)
  rejects creation while inhibited and closes existing sessions on inhibition.
  Sessions belong to the creating peer; closing the capture's private bus
  connection also handles cancellation before a creation reply arrives.
- Gnoblin's [lock capture overlay](../../patches/mutter/71-session-lock-capture/0001-session-lock-embargo-capture-and-remote-input.patch)
  rejects area capture while locked and covers active area streams during lock.
  The new backend does not modify or bypass that boundary.
- [Upstream wlr-screencopy XML](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-screencopy-unstable-v1.xml)
  marks the protocol deprecated and recommends `ext-image-copy-capture-v1`.
  Gnoblin's existing screencopy implementation checks session lock at frame
  creation and copy, but does not use Mutter's remote-access inhibition
  controller. Neither a `grim` subprocess nor a new screencopy client would
  preserve that controller's inhibition without additional compositor work.
- [The backend Screenshot interface](../../src/portal/interfaces/org.freedesktop.impl.portal.Screenshot.xml)
  defines `interactive` as a customization hint. The new native dialog offers
  whole-desktop capture with optional pointer inclusion. It does not promise
  window/region selection. `PickColor` still lacks a native selection UI and
  returns response 2 promptly, without capture.
- [The portal frontend](https://github.com/flatpak/xdg-desktop-portal/blob/1.20.3/src/screenshot.c)
  handles permission-store access and document registration. Its
  [public Request contract](https://github.com/flatpak/xdg-desktop-portal/blob/1.20.3/data/org.freedesktop.portal.Request.xml)
  requires response 0/1/2 for success/cancel/other failure, and no `Response`
  signal after `Close`. The backend returns its method result; it does not
  emit the public signal itself.

No new public capture protocol or GNOME Shell service is added. Failure to
connect to the selected compositor or PipeWire produces a failed request;
there is no alternate capture path that can bypass those checks.

## Main-only native VM proof

Main owns all guest/host GUI operations and delivery. Do not run the old
`tests/qemu-e2e/run-portal-screenshot.sh` as part of this handoff: it restarts
GDM and accepts the old missing-PNG result as a known gap.

Use an already provisioned disposable Gnoblin VM. Main must select and start
the newly built backend through its normal VM delivery procedure; this
fixture does not install, replace or restart a service.

Before testing,
record the backend executable path and SHA256, the compositor executable
and loaded Mutter library, the portal frontend version, and the active
monitor layout. A compiled binary alone does not prove which one is running.

Run from the guest repository with the guest user's session bus and Wayland
environment. Keep a new artifact directory for every invocation:

```sh
python3 tests/portal-screenshot/request.py /var/tmp/shot-allow
python3 tests/portal-screenshot/request.py /var/tmp/shot-interactive --interactive
python3 tests/portal-screenshot/request.py /var/tmp/shot-cancel --interactive --expect cancel
python3 tests/portal-screenshot/request.py /var/tmp/shot-close \
  --interactive --expect closed --close-after-ms 500
```

For the first request, choose Allow in the frontend dialog if it appears.
In the second, toggle **Include pointer** and take the screenshot. In the
third, choose Cancel. The fourth closes its request programmatically; its
dialog must disappear and no public response should arrive.

The probe verifies one response, a decodable PNG on success, no URI on failure,
and that the request is unexported afterwards. It writes `result.json` and,
on success, a copy named `screenshot.png`.

Inspect the actual PNG in the VM:
it must show the desktop, contain every output, have the expected scale and
orientation, and exclude the consent dialog. Check pointer inclusion in both
states.

Repeat after a monitor layout change, with a rotated display and with
mixed/fractional scales; the resulting PNG need not have logical dimensions.

After one successful request has been approved in the consent dialog, lock the guest through its real locker. From the guest's existing remote
terminal with the same session bus, run:

```sh
python3 tests/portal-screenshot/request.py /var/tmp/shot-locked --expect failure
```

A new request while locked must return no URI. Unlock normally and rerun the
success probe.

For an in-flight lock race, collect lock transition and capture
ordering: an area stream may fail or provide a black cover frame, but must not
capture unlocked desktop pixels after the compositor enters its cover state.
The local source checks do not establish that timing on a native seat.

For actual inhibition, use a debug-symbol build in the disposable VM. Gnoblin
does not expose a user command for toggling Mutter's inhibition counter. Main
can attach GDB to the compositor worker, break at
`meta_dbus_session_manager_create_session`, then start one screenshot probe.
At that breakpoint, save and inhibit the actual manager:

```gdb
set $shot_manager = session_manager
call (void) meta_dbus_session_manager_inhibit($shot_manager)
disable breakpoints
continue
```

The probe must fail with no URI. Interrupt GDB, restore the counter, and detach:

```gdb
call (void) meta_dbus_session_manager_uninhibit($shot_manager)
detach
quit
```

Then rerun the success probe. Inhibition closes other sessions managed by that
object, so use an otherwise idle disposable guest. If symbols or native seat
access are unavailable, record inhibition or lock-race proof as outstanding;
do not count the D-Bus stand-in as native evidence.

For every case, compare `pw-dump` and the compositor's ScreenCast object tree
before and after. Once a request finishes or closes, its new stream/session
must be gone, and a subsequent request must still succeed.

Also test a
sandboxed screenshot client through the public portal: its returned document
URI must be readable inside that sandbox. The host-app probe cannot prove
sandbox document export.

## Frontend consent candidate

`consent-frontend-1.20.4.patch` is a candidate for the separate upstream
`xdg-desktop-portal` frontend. Apply it to a disposable copy of that version's
source and build it normally with Meson. It retains the negotiated backend
version for consent decisions across backend restarts. Gnoblin's build does
not apply this patch, and the runner does not build or install the frontend.
A backend with a different protocol version needs frontend restart to
renegotiate.

Use `--backend-version 1` only with a backend that actually advertises version
`1`. That compatibility control verifies the legacy backend-owned consent route
in both states; it is not a way to change the backend's advertised version.
Native Wayland presentation, real compositor policy and sandbox-readable PNGs
remain main's separate VM checks.

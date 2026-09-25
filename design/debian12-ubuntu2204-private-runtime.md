# Debian 12 and Ubuntu 22.04 private-runtime decision

Status: design investigation, 2026-09-25. This is internal release
engineering material. It does not mark either target supported.

## Decision

Do not add Debian 12 or Ubuntu 22.04 to the package build matrix with the
current DEB runtime. Do not lower Mutter or GNOME dependency floors.

A Gnoblin-private runtime can support these releases in principle, but only as
a deliberate compatibility-runtime project. The present bundle is not that
project: it supplies a private GLib, Wayland, libinput, PipeWire client,
GJS/mozjs, GNOME Desktop, and a handful of compositor libraries. It does not
supply the GTK, GIRepository, and GCR chain that GNOME Shell 51 requires.

Proceed only when the work is funded as a separate compatibility milestone
with its own source lockfile, clean-image builds, and graphical session tests.
Until then, retain the CI capability probes and `unsupported` target state.

## Measured starting point

The `scripts/probe-deb-target.py` clean-image gate records the following
stock-repository state:

| Requirement     | Debian 12   | Ubuntu 22.04 | Current consequence                                         |
| --------------- | ----------- | ------------ | ----------------------------------------------------------- |
| GTK 4           | 4.8.3       | 4.6.9        | Both are below Mutter's GTK 4.14 floor.                     |
| GIRepository 2  | unavailable | unavailable  | GNOME Shell's `girepository-2.0` dependency cannot resolve. |
| GCR 4           | unavailable | unavailable  | GNOME Shell's `gcr-4` dependency cannot resolve.            |
| libei/libeis    | unavailable | unavailable  | Mutter's remote-input interfaces cannot resolve.            |
| libdisplay-info | unavailable | unavailable  | Mutter's display metadata dependency cannot resolve.        |
| Glycin 2        | unavailable | unavailable  | Mutter's image loader dependency cannot resolve.            |
| Hyprcursor      | unavailable | unavailable  | Gnoblin's cursor support cannot resolve.                    |

The requirements are not guesses from package names. The checked-out Mutter
Meson files require GTK `>= 4.14`, Glycin `>= 2.0.beta.2`, libei/libeis
`>= 1.3.901`, libdisplay-info `>= 0.2`, GLib `>= 2.81.1`, and current Wayland
interfaces. The GNOME Shell Meson files require `gcr-4`, `girepository-2.0`,
GLib/Gio `>= 2.86`, and GJS `>= 1.87.1`. Repeat this inspection in the
release-clean source worktree when source pins change; an arbitrary developer
submodule checkout is not release evidence.

Debian 12's Rust 1.63 is also below the modern toolchain used by the existing
private Glycin and Hyprcursor build path. A pinned build-only toolchain solves
that compiler problem, but it does not solve the runtime ABI issues above.

### First source-closure attempt

On 2026-09-25, a clean Debian 12 container installed only compiler, graphics,
introspection, crypto, and archive bootstrap packages. The private dependency
builder downloaded and configured GLib 2.90.0, then stopped before compilation:

```
Program g-ir-scanner found: NO found 1.74.0 but need: '>= 1.80.0'
```

This is the first concrete source-closure blocker. Debian 12's
`gobject-introspection` package supplies scanner 1.74.0, while GLib 2.90.0
requires scanner 1.80.0 or newer when introspection is enabled. The target
cannot build the final private GLib directly from the current flat manifest.

The next implementation must use an explicit bootstrap graph:

1. Build the pinned GLib source once with introspection disabled in the private
   prefix.
2. Build a pinned GObject Introspection 1.80+ scanner against that bootstrap
   GLib, as a build-only tool.
3. Rebuild the pinned GLib with introspection enabled. This produces the
   private `girepository-2.0.pc` interface and typelibs required by Shell.
4. Build private GDK-Pixbuf, Pango, Graphene, GTK 4.14+, and GCR 4 before GJS,
   GNOME Desktop, Mutter, and Shell.

The isolated `packaging/deb/compat-bootstrap.json` graph now completes this
sequence in a disposable Debian 12 image: patchelf, GLib 2.90 with
introspection disabled, GObject Introspection 1.80.1, then GLib 2.90 with
introspection enabled. Both GNOME source checksums were verified against the
upstream checksum files. The build-only scanner lives in the sibling
`deps.build-tools` prefix; the runtime prefix contains `girepository-2.0.pc`
and private libraries with an RPATH to its own `lib64`, and does not contain
`g-ir-scanner`. The final GLib build passed its private-interface check.

The disposable image needed `python-is-python3`, `python3-dev`, and `flex`
for the scanner bootstrap. This is only a source-closure result on Debian 12:
the production DEB manifest and workflow are unchanged, and neither GTK/GCR,
Mutter/Shell, package transactions, Ubuntu 22.04, nor graphical sessions have
been validated. The builder still supports declared dependency ordering and
selected subgraphs, and its archive extraction supports Python 3.10/3.11 while
retaining staging-path and link-containment checks.

The next clean-image pass installed the normal host build prerequisite
`shared-mime-info` 2.2-1. GDK-Pixbuf 2.44.8 then resolved the private GLib
interfaces and stopped at its required `glycin-2` dependency. Glycin 2.0.0 is
therefore part of the experimental closure, built without the optional GTK4
binding so that it can precede GDK-Pixbuf. Its upstream `Cargo.toml` declares
`rust-version = "1.85"`; Debian 12 provides Rust 1.63.0. This is the current
source-closure incompatibility. Resolving it requires a pinned, build-only
Rust 1.85+ toolchain. Do not substitute the host compiler, lower Glycin, or
promote Debian 12 support until that toolchain and the rest of the graph build
cleanly.

## Boundary: what can be private

The executable compositor and Shell should load one coherent Gnoblin library
closure from `/usr/lib/gnoblin/deps`. These libraries are viable private
runtime candidates when every transitive ABI dependency is built from the same
closure and the loader path is limited to Gnoblin Shell/Mutter:

| Layer                            | Private candidates                                                                                                           | Reason and constraint                                                                                                                                                            |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GNOME language and introspection | GLib/GObject/Gio, GObject Introspection and GIRepository, mozjs, GJS, typelibs                                               | GJS must use the same GLib and GIRepository ABI as Shell. Typelib lookup must not allow old host GLib/Gio typelibs to override private ones.                                     |
| Toolkit and Shell UI             | GTK 4, Graphene, Pango and any newer text/rendering dependencies, libadwaita if Shell needs it, GCR 4                        | These are in-process Shell UI dependencies. They may live privately, but GCR's Secret Service and PKCS#11 interactions need a real-session test.                                 |
| Compositor libraries             | GNOME Desktop, Glycin/loaders, libei/libeis, libdisplay-info, Hyprutils/Hyprlang/Hyprcursor, Wayland and protocols, libinput | These are linked by private Mutter/Shell. Build only libraries and data under the private prefix; never install global udev rules, system services, or a second display manager. |
| PipeWire client                  | the private `libpipewire-0.3` already used by the bundle                                                                     | A client library may be private, but it must be tested against each host daemon and WirePlumber version for ScreenCast and RemoteDesktop. It must not launch a private daemon.   |

The first two rows turn this from the current modest bundle into a GNOME
platform closure. GTK itself pulls further libraries that must be discovered
from a clean build rather than assumed from a newer distribution: the likely
closure includes compatible GDK Pixbuf, Pango, Graphene, HarfBuzz, Fribidi,
libepoxy, and image codecs. GCR brings its own crypto, smart-card, and keyring
dependencies. The private lockfile must record the actual source graph,
versions, hashes, Meson options, and runtime RPATHs after a reproducible build
proves the exact set.

## Boundary: what must remain host-owned

These components define the desktop session or hardware policy and must not be
shadowed by a private copy:

| Host service                                    | Why it stays host-owned                                                         | Required compatibility evidence                                                                    |
| ----------------------------------------------- | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| GDM, PAM, systemd-logind, systemd user manager  | Own authentication, seat assignment, session lifecycle, and service activation. | Login through GDM, lock/unlock, logout, then stock GNOME login.                                    |
| `gnome-session` and GNOME Settings Daemon       | Own the target distribution's session and settings policy.                      | Start the Gnoblin session through the host manager without private loader variables leaking to it. |
| D-Bus and `xdg-desktop-portal` broker/backends  | Own app-facing desktop integration and backend selection.                       | File chooser, settings, ScreenCast, and RemoteDesktop against the host broker.                     |
| PipeWire daemon and WirePlumber                 | Own audio/video graph and permission/session policy.                            | RustDesk connect, disconnect, reconnect, and locked-session visibility through the host graph.     |
| Mesa, DRM, kernel drivers, udev, libseat/logind | Own hardware discovery, DRM access, and device policy.                          | Real hardware graphics, monitor hotplug, input, suspend/resume, and a second seat where supported. |

The existing launchers already point in the right direction: `gnoblin-env.sh`
scopes `LD_LIBRARY_PATH` and `GI_TYPELIB_PATH` to the private Shell/Mutter
processes, while `gnoblin-session` clears them before it launches the host
`gnome-session`. A compatibility runtime must preserve and extend that
property. Do not set a global loader configuration, alter `/usr/lib`, replace
host typelibs, or export private schemas to stock GNOME.

## Required build and package changes

This is the minimum concrete work to make a clean implementation possible.
None of it is implemented by this design note.

1. Create a separate Debian-family compatibility manifest. It must be a
   dependency graph rather than the current flat list, because GTK/GI/GCR must
   build before GJS, GNOME Desktop, Mutter, and Shell. Pin every source and
   checksum. Add a pinned build-only Rust/C++ toolchain for targets whose
   archive toolchain cannot build the graph.
2. Extend `build-private-deps.py` to consume that graph, isolate bootstrap
   tools from runtime files, generate private pkg-config and typelib search
   paths, and reject an ELF or typelib resolving to an incompatible host
   version. Preserve the existing atomic prefix installation and RPATH check.
3. Build GNOME Desktop, Mutter, Shell, and Gnoblin against the private graph.
   Emit a dependency report from `lddtree`/`readelf`, `pkg-config`, and
   typelib inspection. The report must identify every host library still
   allowed at runtime.
4. Keep the single namespaced `gnoblin` DEB layout. Add explicit package
   metadata only for host-owned services, never for private GNOME libraries.
   `dpkg-shlibdeps` must resolve host libraries without generating dependencies
   on the private SONAMEs or stock `gnome-shell`/`mutter` packages.
5. Add target-specific build jobs only after a local container build succeeds.
   Add clean stock-GNOME install, co-install, removal, and graphical-session
   gates. The existing headless DEB test is useful, but it cannot prove GDM,
   a hardware seat, portal capture, or RustDesk reconnect behaviour.

The private build must keep udev rules, PipeWire units, systemd units, portal
files, and GSettings schemas inside Gnoblin's namespace. A build recipe that
installs any of those into a host path is a design failure, even if the package
transaction succeeds.

## Closure milestones and decision gates

| Milestone              | Deliverable                                                                               | Stop condition                                                                                                          |
| ---------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| 1. Source closure      | Locked source DAG and license inventory; no unverified downloads.                         | A dependency needs a host GNOME service or replaces a host session component.                                           |
| 2. Clean build         | Debian 12 and Ubuntu 22.04 containers build the complete private graph and GNOME runtime. | Any private binary reaches an older host GLib/GI/GTK through RPATH, typelib, or `dlopen`.                               |
| 3. Package transaction | One target-specific DEB installs beside stock GNOME, then removes cleanly.                | Package owns a stock GNOME file, Provides/Replaces/Conflicts with a stock GNOME package, or leaves files after removal. |
| 4. Graphical session   | Both session choices work on each target.                                                 | GDM/logind, portal, input, display, suspend/resume, or stock GNOME regression occurs.                                   |
| 5. Remote desktop      | RustDesk uses the host portal/PipeWire stack before and after locking.                    | Capture, input permission, reconnect, or the lock screen fails on the real target.                                      |

The work is substantial because it creates and maintains a second, private
GNOME platform for each older ABI family. It is not equivalent to adding two
containers to CI. A realistic planning unit is the five milestones above,
with a stop/go review after the first clean private closure; no time estimate
is credible until that closure has been built and its size, licenses, and host
interfaces are known.

## Recommendation

Conditional go for a dedicated compatibility-runtime project; no-go for
shipping Debian 12 or Ubuntu 22.04 in the current `0.1.7` package path.

The project can retain the requested support policy without pretending these
releases work today: leave both targets as `unsupported`, retain the
capability-probe artifacts, and open an implementation item for the source
closure milestone. Promote either target only after all six package target
gates pass, including graphical-session verification. This preserves GNOME
coexistence and avoids the fragile alternative of partially upgrading host
GNOME libraries.

## References

- `scripts/probe-deb-target.py`: reproducible clean-image package evidence.
- `build-dependencies.json` and `packaging/deb/build-dependencies.json`:
  current private runtime boundary.
- `src/tools/gnoblin-env.sh` and `src/tools/gnoblin-session`: private loader
  scoping and host-session hand-off.
- [xdg-desktop-portal design considerations](https://flatpak.github.io/xdg-desktop-portal/docs/design-considerations.html)
- [xdg-desktop-portal backend configuration](https://flatpak.github.io/xdg-desktop-portal/docs/configuration-file.html)
- [PipeWire session manager model](https://docs.pipewire.org/page_session_manager.html)

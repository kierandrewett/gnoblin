# Gnoblin session and compositor architecture

The standalone `./build.sh` path builds the Gnoblin supervisor and Lua runtime,
patched Mutter, and the portal backend from pinned GNOME sources. It does not
build GNOME Shell. Separate Shell compatibility recipes and patches remain in
the repository and still use GJS; they are outside the standalone build and
remain migration work. GNOME Session and Settings Daemon are optional services
for compatibility login paths. Removing a package requirement does not remove
code that uses its interfaces.

## Ownership target

Gnoblin must own its desktop behavior in Gnoblin source. Mutter should remain
the upstream compositor and window manager, with only a small integration
surface where Wayland clients cannot provide the required capability. A new
Gnoblin behavior is not a reason by itself to add another Mutter patch.

The intended process split is:

- **`gnoblin`:** the session supervisor and Lua runtime. It owns config, policy,
  session lifecycle, Gnoblin's control interface, and its child processes. It
  does not import Mutter or GNOME Shell internals.
- **`gnoblin-mutter`:** upstream Mutter with a small Gnoblin patch set. It runs
  as a child of `gnoblin` and owns Wayland clients, windows, rendering, input,
  and compositor-side protocol implementations. Patches fill compositor gaps
  that plugins and client processes cannot cover; generic fixes should go
  upstream.
- **Shell projects:** build their own desktop interface as separate projects.
  Bingux is the example: it uses layer-shell surfaces and Gnoblin's config and
  control interfaces. Gnoblin does not ship a replacement shell UI or require
  GNOME Shell feature parity.
- **GNOME Shell compatibility:** remains an optional adapter for users who
  choose a GNOME session. GJS is not part of the standalone Gnoblin runtime.

Lua describes Gnoblin policy and behavior; it does not replace compositor
mechanics. Keep Lua in the `gnoblin` process. It sends validated operations to
`gnoblin-mutter` over a narrow local interface and exposes the Gnoblin control
contract to shell clients. Use standard Wayland protocols for shell clients
wherever possible. Keep compositor-specific operations behind a small
versioned interface instead of exposing Mutter's private C types.
Configuration reloads should apply runtime-safe changes without restarting
Mutter. Protocol registration and other startup-only compositor settings may
still require a compositor restart.

When considering a Mutter change, first check whether Gnoblin can own it in
its runtime, shell, or another client module. If a compositor hook is required,
keep it as a narrow capability with a versioned interface; put policy,
configuration, and user-facing behavior in Gnoblin. Upstreamable generic fixes
should be proposed upstream rather than maintained only as Gnoblin patches.

## Current seams

| Part                      | Current dependency                                         | Reason it remains                                                                                                                                           |
| ------------------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Compositor                | Mutter 51                                                  | Current patches add Gnoblin protocols, input, rendering, configuration, and native control. Target: `gnoblin-mutter`, with a small patch set over upstream. |
| Session                   | `gnoblin`, logind, systemd user targets                    | Target: `gnoblin` supervises Mutter, its Lua runtime, and session tools.                                                                                    |
| Shell host                | Separate shell clients; optional GNOME Shell compatibility | The standalone build omits GNOME Shell. Compatibility packaging and its GJS adapter remain separate and are not yet retired.                                |
| Optional desktop services | `gnome-settings-daemon`                                    | The fuller source-login mode requests hardware, accessibility, and other services.                                                                          |
| Portals                   | `xdg-desktop-portal` plus Gnoblin's backend                | The generic frontend routes requests to the selected backend.                                                                                               |
| Settings                  | `gsettings-desktop-schemas`                                | Mutter and Shell read shared desktop setting definitions.                                                                                                   |

The package recipes select Gnoblin's portal backend for a Gnoblin session.
The Gnoblin session package requires `gnoblin-portal` and the generic portal
frontend. The GTK portal is no longer a base dependency. GNOME's backend can coexist for
an existing GNOME login. The source build can prepare and check the portal
without preparing or checking Mutter or Shell. The Arch release job builds
from the source tarball and checks package installation beside stock Shell and
Mutter when its distribution libraries meet the pinned requirements.
An Arch-style `DESTDIR` build from the r44 release tarball completed with the
private `/usr/lib/gnoblin` prefix. Its assembled package tree has a compiled
private schema cache and no build-prefix paths; GNOME Shell found Mutter's
staged pkg-config files. This was staged on Fedora 45, not built or installed
with Arch's `makepkg` and `pacman`. Arch's stable desktop-schema package is
still below the pinned GNOME 51 minimum, so that target needs a matching
distribution package before a native Arch package build can pass.

A fresh Fedora 45 solve with the r42 source tarball's Mutter, Shell, session,
portal and meta RPMs, with weak dependencies disabled, selected 207 packages
and 661 MiB of installed files. It still selected GTK3. Fedora's
`gnome-desktop4` package required `gnome-desktop3`, which linked GTK3.
The Shell's direct requirement for that package is now removed. The Gnoblin portal
now previews images directly through Glycin, with
a GdkPixbuf fallback, and selects the first frame of an XML slideshow with a
small GMarkup reader. Its executable no
longer links `libgnome-bg-4` or `libgnome-desktop-4`. Mutter now queries the
same udev hardware database directly for PnP
vendor names and builds without `libgnome-desktop`. This r42 solve is historical;
the newer clean install is recorded below. The solve is package metadata evidence, not a login
or app compatibility check. Installing those five local RPMs on the clean
Fedora 45 base completed and left stock GNOME Shell, Mutter, GNOME Session,
Settings Daemon, GDM, and `xdg-desktop-portal-gtk` absent. That transaction
confirms package installation and script execution, but no graphical login
or application behavior.

Shell now creates launched-app scopes with its own native systemd call and
selects input-source names and language codes through libxkbregistry. Its
native slideshow parser reads GNOME wallpaper XML once and retains timed
transitions and monitor-size variants. The Gnoblin session no longer imports
`GnomeBG` or `GnomeDesktop`; GNOME-only date and unlock UI still uses
`GnomeDesktop.WallClock` when that mode is selected. Wallpaper backgrounds
share one `/etc/localtime` watcher. Meson and the base RPM and Arch recipes no
longer require `gnome-desktop-4`. The new Shell library and typelib compiled on
Fedora 45, and the native parser returned a schedule through GJS. This is not
yet a graphical slideshow proof.

Fedora 45 RPM builds of Shell, Mutter, portal, session, and the matching
`gnoblin` metapackage completed. A fresh Fedora 45 container installed these
five local RPMs with weak dependencies disabled. The transaction selected 204
packages and 637 MiB. After installation, `rpm -q` confirmed all five Gnoblin
packages and the absence of `gnome-desktop4`, `gnome-desktop3`, GTK3, stock
GNOME Shell, Mutter, GNOME Session, Settings Daemon, and GDM. The installed
`gnoblin --version` command reported the source remote, commit, and component
versions. This proves package installation, not a graphical login or wallpaper
transition.

Gnoblin now handles FileChooser, Email, Settings, and Inhibit without the GTK
portal. A standalone login maps portal idle requests to its owned
`org.freedesktop.ScreenSaver` service and suspend requests to logind. It returns
an error when a lock cannot be acquired and releases both locks on Request.Close
or frontend disconnect. Logout and user-switch inhibition are reported as
unsupported in the lean login; GNOME Session login forwards all flags to
GNOME SessionManager only when the active desktop identifies as GNOME. A
manager process left on a shared user bus does not change the standalone
session's inhibitor route. Monitor sessions report screen saver activity and a
running session, but standalone session-end query and ending states are not yet
reported. That lifecycle integration and real-seat native and Flatpak probes
remain work before full portal Inhibit parity.
Mutter's `idle-inhibit-unstable-v1` handler also calls
`org.freedesktop.ScreenSaver.Inhibit` and `UnInhibit`. The standalone session
therefore needs one owned screen saver/inhibitor service shared by Wayland
clients and the portal backend. It must connect idle prevention to the actual
screen lock and idle policy; a D-Bus cookie without that connection would repeat
the GTK fallback's false-success behavior. The portal's logout and switch
inhibitors need separate session-lifecycle handling.

GNOME Control Center is not built or shipped, so its unused source submodule
and release pin are removed; users can install their distribution's Settings
app separately. The disposable CI source-build images no longer request stock
GNOME Shell, Mutter, or GNOME Session packages; the source build uses patched
Mutter and Shell.
The main Fedora verification job now builds the release source tarball after
extracting it without Git metadata. This exercises the tarball path before
package adapters and catches files omitted from the archive.

The RPM component and session packages do not require `gnome-settings-daemon`.
Mutter's optional settings-daemon headers are no longer mandatory to compile.
The CI source-build dependency installer now omits the settings daemon too;
the optional GNOME Session login needs it for the services listed below.
It also omits Just: the source tarball's build, nested preview, and session
registration paths are all available through `./build.sh`.
The Fedora Shell RPM also drops inherited requirements for Tecla, the
dual-GPU launch service, and user-directory setup; the last is provided by
the optional GNOME app integration package.
It no longer recommends Bolt by default: the Gnoblin session has no GNOME
Thunderbolt menu, and users can install Bolt when their devices need it.
The direct session also skips GNOME's power menu and end-session dialog, so
the Shell RPM no longer requires UPower. GNOME and the optional GNOME Session
path still load UPower for those interfaces.

The remaining Shell requirements cannot be removed by editing the spec alone.
`js/misc/dependencies.js` still eagerly imports AccountsService, which the
polkit agent and other Shell modules use. GDM, Geoclue, and GWeather now load
only for the Shell UI that needs them in Gnoblin mode or another session.
The GnomeQR typelib is now optional: Shell loads it only for QR rendering and
web login keeps the URL and code visible when it is unavailable.
The Gnoblin date menu is empty, so its weather module now skips the Geoclue
and GWeather typelibs in Gnoblin mode. The Fedora Shell RPM no longer names
those libraries as direct requirements. GNOME's normal date menu still loads
them; the session's remaining services may depend on the same libraries.
Gnoblin also has no network menu in its Shell panel. Shell now skips the NMA4
typelib in Gnoblin mode, while retaining the NM typelib for its network secret
agent. The source tarball carries this change in the Shell patch series; RPM
metadata follows the source behavior.
The Gnoblin session also skips GNOME's login and unlock dialogs and loads
GDM's typelib only if it is present for system actions. Without it, the
switch-user action is unavailable, while logout still works. The source
tarball carries this Shell patch, and the Fedora Shell RPM no longer directly
requires `gdm-libs`. Shell loads session mode after its main module is ready so
GNOME's login and unlock dialogs can still import that module when needed.
Shell already probes for GNOME Bluetooth at runtime and Gnoblin never creates
the upstream Bluetooth panel. Its explicit Fedora build requirement and
generated Shell typelib requirement are removed from the package metadata.
Gnoblin's session mode also omits Shell's automatic removable-media mount and
autorun components. Those components call GNOME SessionManager to check active
session state and automount inhibitors. File-manager-initiated mounts remain
available through the user's chosen file manager and its services.
The Shell build now disables the matching HotplugSniffer executable and D-Bus
service. A fresh source-tarball build on Fedora 45 installed neither file.
The Shell MessageTray no longer subscribes to GNOME SessionManager presence in
Gnoblin mode; that subscription only controlled native notification banners,
which Gnoblin does not show.
The source build registers a standalone login entry by default with
`./build.sh --register-session`. It launches the compositor
without `gnome-session` or `gnome-settings-daemon`. In that branch, Shell routes
logout, power, reboot, and suspend actions to logind. Once the compositor
reports ready, Shell copies its display address to D-Bus activation and the
systemd user manager, then the session target starts `graphical-session.target`
and XDG autostart. The wrapper stops that target when the compositor exits.
The wrapper clears stale display addresses left by another desktop before
launch, then removes them from the user manager at logout.
Standalone Shell skips GNOME's end-session dialog and its startup proxies;
logout terminates the logind session directly.
Both the wrapper and Shell use `XDG_CURRENT_DESKTOP=Gnoblin` so GNOME-only
autostart entries do not become part of the minimal session. Source and package
login entries use the direct launcher. This path does not start the selected
settings-daemon services and has not been verified at a real login screen.
The GNOME extension tool is disabled in every build route. Its only
`gnome-autoar` dependency is therefore removed from source-build provisioning
and both RPM build recipes. The Shell RPMs also drop the extension tool's
`bash-completion` requirement; Fedora's Shell RPM drops `rst2man` because its
manpages are disabled. Mutter still builds manpages and needs `rst2man` in the
combined source build.
The source tarball exposes Mutter's existing XWayland build switch through
`./build.sh --without-xwayland`. It avoids X11 build dependencies for a
Wayland-only install; the default retains X11 application compatibility.
The build entry point now defaults to the extracted tree's `./install` even
inside an active Gnoblin session, where `GNOBLIN_PREFIX` may point to the
installed system runtime. A custom source prefix uses `--prefix DIR`.
The normal build omits Mutter's development viewer. `./build.sh --preview`
builds it on demand; package recipes disable it. The portal backend still
requires libadwaita, so this does not remove that library from the full build.

Mutter's package disables installation of its optional Python `gdctl` and
`gnome-service-client` tools while upstream's default remains available.
The installed `gnoblinctl` command is native and uses GLib, GIO, and JSON-GLib;
using GJS for that command would increase reliance on the JS runtime. A clean
Fedora 45 solve with an experimental GJS CLI and the optional Mutter tools
removed still selected 207 packages and 661 MiB: `at-spi2-core`, `gstreamer1`,
and `libwacom` each require Python in that distribution. That experimental
CLI was reverted. No graphical session was run for the experimental package.

Gnoblin's private Shell build also disables the captive-network portal helper,
which its panel-free session does not expose; the network secret agent and
camera monitor remain enabled. The same panel-free session has no date menu,
so its Shell build also omits the calendar server and direct Evolution Data
Server build requirement. These changes reduce the install and build graph;
they do not make the compositor independent of GNOME Shell or Mutter.
Shell's native code no longer calls libxml2. The source patch stack removes its
remaining Meson links, and the source-build dependency installer and Fedora
Shell recipe no longer request its development package. The camera monitor
remains: its PipeWire state feeds `cameraInUse` in the compositor bridge.

## Replacement order

1. **Verify the owned login lifecycle.** Build from the release tarball on a
   minimal host, then verify display-manager registration, the D-Bus and
   systemd user environment, compositor failure shutdown, and switching back
   to GNOME on a real seat. The direct launcher exists, but this gate remains
   open. GNOME Settings Daemon services need separate wiring if retained.
2. **Account for desktop services.** Record which requested
   `gnome-settings-daemon` services the session actually needs. Replace or
   make each one optional only with an equivalent user-visible behavior for
   input, accessibility, hardware controls, and XSettings.
3. **Finish native API coverage and retire the GJS adapter.** The standalone
   `gnoblin` binary now owns the Lua worker, configuration reload, operation
   completions, and the native control socket. Native-control validates
   requests and dispatches compositor-owned operations through Mutter's
   versioned API; `gnoblinctl` is a native client. Keep compositor policy in
   Lua and use standard Wayland interfaces where they cover the operation.
   The standalone build omits Shell, but separate compatibility code still
   contains a GJS bridge. Compare every remaining compatibility operation with
   the native API, preserve its documented socket response, and retire the
   bridge only when no supported path depends on it. Track remaining cutovers
   in `gnoblin-0zx`.
4. **Verify session supervision and recovery.** `gnoblin` starts Mutter as its
   compositor child, supervises the Lua worker, and owns readiness, environment
   handoff, failure reporting, and cleanup. Shell projects such as Bingux
   remain separately installed clients, not Gnoblin session internals. The
   worker and compositor have separate restart paths; restarting the worker
   can preserve windows while Mutter remains alive, but restarting Mutter
   disconnects Wayland clients and loses their windows. Verify these boundaries
   at a real login before claiming recovery behavior. The runtime owns Lua
   config loading and reload; each setting still needs a clear live-versus-
   restart requirement because protocol registration and other startup-only
   compositor settings cannot become live merely by moving Lua out of Mutter.
   The built `mutter` executable ran for eight seconds as a headless Wayland
   compositor with a virtual monitor and its default plugin, without starting
   GNOME Shell. Upstream describes plain standalone Mutter as a debugging
   mode; that run does not establish a supported user-session host. See
   [Mutter's standalone guidance](https://mutter.gnome.org/). The process does
   not load Gnoblin's Lua runtime document or
   provide the existing control socket, session services, or lock UI. It is a
   compositor feasibility check, not a login path.
   Mutter's `-- COMMAND` hook starts a child after its monitor is ready and
   passes the child `WAYLAND_DISPLAY`. A Fedora 45 headless run launched a
   client command with `WAYLAND_DISPLAY=wayland-0` and the session's
   `XDG_CURRENT_DESKTOP`; the compositor exited when that command ended. This
   gives a native session launcher a ready point for environment handoff and
   an external shell. It does not provide the session lifecycle or lock UI.
   The native `mutter` entry point can load the same Lua runtime document as
   the Shell session with `--gnoblin-config PATH`. It applies the compositor
   sections it supports and logs warnings for recognized Shell-only sections
   and event handlers that it skips; unknown sections still stop startup. The
   native shortcut path also skips Shell keybinding groups, named Shell
   actions, and command shortcuts that request Shell input capture. This
   avoids a second config file while making the unavailable behavior visible.
   A rebuilt binary on Fedora 45 loaded the repository's full
   `src/data/init.lua.example`, logged which Shell-only settings it skipped,
   and answered `gnoblinctl ping` without Shell. Unknown sections remain fatal.
   It still lacks the full control service, session lifecycle, and lock UI, so
   the login path remains on Shell.
   The native host now owns a limited `compositor-v1.sock` endpoint. Its first
   methods are ping, monitor and window listing, numeric workspace management,
   and native window actions. Requests pass through the same Lua
   method validator and Mutter dispatcher as the Shell
   bridge. A Fedora 45 headless run returned `pong` and a 1280x720 monitor to
   `gnoblinctl` without GJS. A headless GTK4 window appeared in native
   `gnoblinctl window list` output with its ID, title, focus, workspace, monitor,
   and geometry. The endpoint accepts multiple requests per connection and
   sends a `hello` greeting with no optional features. Its native `windows`
   subscription published the initial empty set, an opened GTK4 window, and
   the empty set after that window closed in a Fedora 45 headless run. The
   native CLI also listed four Mutter workspaces, created and renamed a fifth,
   moved a GTK4 window there, refused to remove it while occupied, and removed
   it after the window closed. This was the numeric-only stage.
   The next Fedora 45 headless run applied two configured workspace IDs and
   names, switched by ID, created and renamed a temporary ID, rejected a
   duplicate ID and removal of a configured workspace, and returned the
   focused GTK4 window's workspace ID. Native workspace count no longer writes
   GNOME's saved workspace count; a probe kept the same count and name settings
   before and after native create/remove operations. The endpoint does not
   implement other subscriptions, shortcut bindings, or Shell services.
   Mutter now consumes the committed Lua document for native window-management
   and compositor preferences. A Fedora 45 startup probe observed the requested
   sloppy focus, disabled click raising, visible bell, and disabled audible bell
   values in Mutter. The probe was removed from the final compiled source.
   The native path now validates the Lua autostart entries before launching
   any command. In a Fedora 45 headless run, it launched a shell command with
   `WAYLAND_DISPLAY=wayland-0` and answered `gnoblinctl ping` without GJS.
   Launch failures are logged and commands are not restarted after exit.
   Native Mutter also registers Lua command shortcuts with press and release
   triggers. A headless Fedora 45 session claimed a `<Super>Return` binding;
   an injected accelerator signal launched its command without GJS. This
   proves the signal-to-command path, not a physical key press on a real seat.
   Built-in Shell actions and popup input capture still require Shell.
   Native Mutter now validates and applies Lua input settings directly through
   its in-memory overlay: mouse, touchpad, keyboard, tablets, styluses, and
   orientation lock. A Fedora 45 headless run accepted all groups and showed
   converted double, uint32, string-array, enum, and boolean values before
   applying them. An out-of-range pointer speed stopped startup with its field
   name. Physical devices and live config reload remain unverified.
   Native Mutter applies top-level Lua `keybindings` and named `shortcuts`
   actions for the `wm`, `mutter`, and `wayland` groups in memory. Fedora 45
   headless probes read `wm.close=<Super>q` back after startup from both
   configuration forms. A `shell` group and `gnome:shell` action fail with an
   explicit Shell-required error. The full login still requires Shell while
   its UI, capture-input shortcuts, and services are being replaced.
   The full login still requires Shell while those capabilities are moved.
5. **Narrow the remaining forks.** Keep the portal frontend protocol and
   backend selection standard. Move Gnoblin's portal implementation out of the
   GNOME backend fork only after its dialogs, capture, permissions, and GNOME
   coexistence behavior have replacement implementations.

Each cut needs a source build, package-resolution check, GNOME coexistence
check, and a graphical login on a real seat. A smaller dependency list alone
does not establish a usable session.

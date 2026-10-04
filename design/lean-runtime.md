# Gnoblin session and compositor architecture

The default standalone `./build.sh` path builds the Gnoblin supervisor and Lua
runtime and patched Mutter. `./build.sh --with-portal` also builds the optional
GTK-based Gnoblin portal backend from pinned GNOME sources. Neither path builds
or runs GNOME Shell or GJS. GNOME Shell compatibility recipes, patches,
and adapters left in the checkout are migration residue, not a supported
Gnoblin session path; retire them as their behavior is covered by Gnoblin's
native Lua API or independent shell clients. GNOME Session and Settings Daemon
compatibility paths are outside the standalone session as well.

## Ownership target

Gnoblin must own its desktop behavior in Gnoblin source. Mutter should remain
the upstream compositor and window manager, with only a small integration
surface where Wayland clients cannot provide the required capability. A new
Gnoblin behavior is not a reason by itself to add another Mutter patch.

The target process split keeps the compositor alive when Lua or its supervisor
must restart:

- **Session guardian:** the durable `gnoblin` login process. It owns session
  activation and teardown, Mutter's process lifetime, the private compositor
  channel, session targets, and initial autostart. The guardian does not
  interpret Lua configuration. It starts the restartable supervisor and keeps
  Mutter's channel endpoint so a replacement supervisor can resume the
  existing compositor. The guardian can be an internal mode of the `gnoblin`
  executable rather than a separate user-facing program.
- **Gnoblin supervisor:** a restartable internal mode of `gnoblin`. It owns Lua
  configuration, policy, runtime API dispatch, and the Lua worker. It receives
  the private compositor endpoint from the guardian and restores the accepted
  configuration and operation watermark after a restart. Its death must not
  terminate Mutter or its Wayland clients.
- **`gnoblin-mutter`:** upstream Mutter with a small Gnoblin patch set. It runs
  under the guardian and owns Wayland clients, windows, rendering, input, and
  compositor-side protocol implementations. Patches fill compositor gaps that
  plugins and client processes cannot cover; generic fixes should go upstream.
- **Shell projects:** build their own desktop interface as separate projects.
  Bingux is the example: it uses layer-shell surfaces and Gnoblin's config and
  control interfaces. Gnoblin does not ship a replacement shell UI or require
  GNOME Shell feature parity.
- **GNOME Shell and GJS:** are not part of the standalone session or its
  supported integration surface. Shell projects use Gnoblin's Lua-backed
  control API from independent Wayland clients.

Lua describes Gnoblin policy and behavior; it does not replace compositor
mechanics. Keep Lua in the restartable `gnoblin` supervisor. It sends validated
operations to `gnoblin-mutter` over the guardian-retained private channel and
exposes the Gnoblin control contract to shell clients. Use standard Wayland
protocols for shell clients wherever possible. Keep compositor-specific
operations behind a small versioned interface instead of exposing Mutter's
private C types. The guardian must not expose this channel through a
same-user-discoverable socket; it passes the inherited endpoint only to the
active supervisor.
Configuration reloads should apply runtime-safe changes without restarting
Mutter. Protocol registration and other startup-only compositor settings may
still require a compositor restart.

When considering a Mutter change, first check whether Gnoblin can own it in
its runtime, shell, or another client module. If a compositor hook is required,
keep it as a narrow capability with a versioned interface; put policy,
configuration, and user-facing behavior in Gnoblin. Upstreamable generic fixes
should be proposed upstream rather than maintained only as Gnoblin patches.

## Current seams

| Part       | Current dependency                          | Reason it remains                                                                                                                                                                                                                                            |
| ---------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Compositor | Mutter 51                                   | Current patches add Gnoblin protocols, input, rendering, configuration, and native control. Target: `gnoblin-mutter`, with a small patch set over upstream.                                                                                                  |
| Session    | `gnoblin`, logind, systemd user targets     | A durable `gnoblin` guardian owns Mutter and session lifecycle; its restartable supervisor owns Lua policy and runtime API dispatch. Real-seat lifecycle verification remains open.                                                                          |
| Shell host | Separate Wayland clients                    | Shell projects own presentation and use Gnoblin's native Lua-backed control API. GNOME Shell and GJS are outside the supported session.                                                                                                                      |
| Portals    | `xdg-desktop-portal` plus Gnoblin's backend | The generic frontend routes requests to the selected backend.                                                                                                                                                                                                |
| Settings   | `gsettings-desktop-schemas >= 49.1`         | Shared schemas provide Mutter types and defaults. Lua can override window, pointer, keyboard, tablet, touchpad, Xwayland, and privacy-screen preferences; omitted input options retain GSettings values for compatibility. GeoClue keeps per-field fallback. |

For each Gnoblin-owned Mutter preference, expose a domain-specific Lua setting
and validate its type and accepted values in Gnoblin. At startup and reload,
Mutter's central preference adapter maps the Lua snapshot into its existing
in-memory preference slots and queues their normal change notifications. The
Xwayland adapter covers grabs, grab access rules, disabled extensions,
byte-swapped clients, and scaling; settings that affect Xwayland startup take
effect on the next Xwayland start. In a Gnoblin session, Mutter ignores the
matching GSettings change so the desktop value cannot override Lua policy.
Keep these mappings in the central adapter; do not add separate config lookups
to consumers.

Keep the legacy schema and key as the compatibility boundary, but keep
user-facing configuration in Gnoblin's domain-specific snake_case paths. Add
new Gnoblin paths only when the setting belongs to the compositor or session;
leave desktop-wide and application-owned preferences with their existing
owners. Make write behavior explicit for every mapped key. Gnoblin config
reloads update adapter values and notify affected consumers without writing
the user's Lua files or silently redirecting writes to GSettings.

Prefer this Mutter-owned adapter over replacing the process-wide default
GSettings backend: a backend swap would also affect unrelated GSettings users
inside Mutter, while the backend extension API has weaker stability guarantees
than public GIO APIs. Non-Gnoblin sessions continue to use GSettings. For
input settings exposed in Lua, an explicit Lua value overrides the current
system preference. Omitted fields and `"inherit"` retain the current GSettings
value for compatibility. This keeps existing pointer and keyboard behavior as
shell clients move to Gnoblin's settings API; replacing that fallback with
Gnoblin-owned defaults is a separate compatibility change. Other unmigrated
preferences continue to use their existing schema values.
The `location.enabled` and `location.max_accuracy` fields are an explicit
compatibility seam: either field can override its matching GNOME setting, and
omitting it or setting it to `inherit` keeps the existing system value. These
global controls do not replace per-application authorization through Lua and
the GeoClue agent.

`monitors.privacy_screen` uses a session-scoped override. `true` and `false`
control supported monitor privacy screens without writing the system key;
omitting the field or setting it to `inherit` follows the system preference.
While the override is active, system preference changes do not override the
Lua value. Removing the override restores the current system value.

This adapter does not remove the schema package by itself: GSettings still
needs key types, enum definitions, and defaults, and other Mutter or portal
consumers may continue reading schemas. The configured input-source list is an
exception: it comes only from Gnoblin's `input_sources.sources`; omitting it
leaves Mutter's current keymap in place and does not import GNOME's saved
source list. Revisit the schema package and its version requirement only after
other consumers and build-time schema checks have been accounted for.

Window interaction policy such as `window_management.auto_maximize`,
`window_management.mouse_button_modifier`,
`window_management.resize_with_right_button`, and
`window_management.check_alive_timeout` belongs in Lua. Mutter applies these
values through its preference adapter and ignores the matching desktop
settings while Gnoblin is running. The drag modifier defaults to `<Super>`;
the resize-button swap defaults to `false`; the client liveness timeout defaults
to 5000 ms and accepts `0` to disable the check.

`compositor.locate_pointer_key` configures the key that triggers
`compositor.locate_pointer`. Mutter owns the key event and visual effect; Lua
owns the configured key name and whether the effect is enabled.

The session package does not require `gnoblin-portal`; users can install and
select another XDG portal backend. The GTK-based Gnoblin backend is a separate
optional package and requires the generic portal frontend. GNOME's backend can
coexist for an existing GNOME login. The default source build does not prepare
or check the portal component. The Arch release job builds from the source
tarball and checks package installation beside stock Shell and Mutter when its
distribution libraries meet the pinned requirements.
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

At an earlier Shell-based packaging checkpoint, the private Shell carried a
native systemd call for launched-app scopes, selected input-source names and
language codes through libxkbregistry, and parsed GNOME wallpaper XML for timed
transitions and monitor-size variants. Those details describe the retired
Shell integration, not the standalone Gnoblin session. The standalone session
does not build or run Shell; wallpaper presentation and launched-app scope
policy belong to shell clients or their launchers. The old Shell implementation
also stopped importing `GnomeBG` and `GnomeDesktop` in Gnoblin mode, while
GNOME-only date and unlock UI retained `GnomeDesktop.WallClock`. At that
checkpoint the Shell library and typelib compiled on Fedora 45 and its parser
returned a schedule through GJS; this did not prove a graphical slideshow.

At that earlier Fedora 45 packaging checkpoint, RPM builds of the private
Shell, Mutter, portal, session, and matching `gnoblin` metapackage completed.
A fresh Fedora 45 container installed those five local RPMs with weak
dependencies disabled. The transaction selected 204
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
running session. When the standalone session supervisor begins teardown, it
reports the portal's session-end query state, waits for active monitor
responses for up to one second, then reports that the session is ending. An
isolated D-Bus integration check covers acknowledged and timed-out responses
and rejects calls from senders that do not own the session-supervisor name.
Real-seat native and Flatpak probes remain before full portal Inhibit parity.
Mutter's `idle-inhibit-unstable-v1` handler also calls
`org.freedesktop.ScreenSaver.Inhibit` and `UnInhibit`. The standalone session
therefore needs one owned screen saver/inhibitor service shared by Wayland
clients and the portal backend. It must connect idle prevention to the actual
screen lock and idle policy; a D-Bus cookie without that connection would repeat
the GTK fallback's false-success behavior. The portal's logout and switch
inhibitors need separate session-lifecycle handling.

GNOME Control Center is not built or shipped, so its unused source submodule
and release pin are removed; users can install their distribution's Settings
app separately. Earlier CI source-build images stopped installing stock GNOME
Shell, Mutter, and GNOME Session packages while the project still built its
private Shell. That packaging stage is historical: the current source build
builds patched Mutter and Gnoblin, and does not build or package GNOME Shell.
The main Fedora verification job now builds the release source tarball after
extracting it without Git metadata. This exercises the tarball path before
package adapters and catches files omitted from the archive.

The following package notes record an earlier private-Shell packaging phase;
they do not describe current Gnoblin components or a supported Shell RPM.
The RPM component and session packages do not require
`gnome-settings-daemon`. Mutter's optional settings-daemon headers are not
mandatory to compile. The CI source-build dependency installer omits the
settings daemon too. The tarball's build, nested preview, and session
registration paths are available through `./build.sh` without Just.
During the Shell RPM phase, inherited requirements for Tecla, the dual-GPU
launch service, user-directory setup, Bolt, and UPower were removed or made
optional for the panel-free Gnoblin mode. The standalone session has no Shell
panel or power menu; those old RPM adjustments are retained here as packaging
history only.

At that checkpoint, remaining Shell requirements could not be removed by
editing the spec alone. `js/misc/dependencies.js` eagerly imported
AccountsService, which the polkit agent and other Shell modules used. GDM,
Geoclue, and GWeather were loaded only for Shell UI that needed them. The
GnomeQR typelib was made optional, and web login kept the URL and code visible
when it was unavailable. The empty Gnoblin date menu caused the weather module
to skip Geoclue and GWeather typelibs in Gnoblin mode; the Fedora Shell RPM
then stopped naming those libraries as direct requirements. The Shell network
menu was also absent in Gnoblin mode, so Shell skipped the NMA4 typelib while
retaining NM for its network secret agent. The source tarball carried these
changes as Shell patches and RPM metadata followed them.

That Shell mode skipped GNOME's login and unlock dialogs and loaded GDM's
typelib only when available for system actions. Without it, switch-user was
unavailable while logout remained available. Its Shell patch and Fedora RPM
metadata removed the direct `gdm-libs` requirement. Session mode loaded after
Shell's main module so GNOME's login and unlock dialogs could still import it
in other modes. Shell's runtime probe made GNOME Bluetooth optional, and the
Fedora metadata dropped the explicit build and generated-typelib requirements.
That mode also omitted Shell's automatic removable-media mount and autorun
components, which called GNOME SessionManager for active-session state and
automount inhibitors. File-manager-initiated mounts remained available through
the selected file manager. The Shell build disabled HotplugSniffer and its
D-Bus service; a Fedora 45 source-tarball build installed neither file. Shell's
MessageTray stopped subscribing to GNOME SessionManager presence in Gnoblin
mode because the subscription only controlled native notification banners.
An earlier standalone-login implementation used `./build.sh --register-session`
to launch the compositor without `gnome-session` or
`gnome-settings-daemon`, but still relied on a Gnoblin-mode Shell for session
actions and D-Bus/systemd environment handoff. That Shell-based path is
retired. The current direct launcher is the supported direction; its display
manager registration, real-seat environment handoff, failure cleanup, and
switch-back-to-GNOME behavior remain unverified at a real login screen, as
recorded in the replacement gates below.
The GNOME extension tool was disabled in every Shell build route. Its only
`gnome-autoar` dependency is therefore removed from source-build provisioning
and both RPM build recipes. The Shell RPMs also dropped the extension tool's
`bash-completion` requirement; Fedora's Shell RPM dropped `rst2man` because its
manpages were disabled. Mutter still builds manpages and needs `rst2man` in the
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

The retired private Shell build disabled its captive-network portal helper and
calendar server and removed direct Evolution Data Server and libxml2 build
requirements. Those Shell build changes reduced the old install graph; they did
not remove Mutter from the compositor architecture. The standalone session
monitors camera and microphone activity in native code. With remote-desktop
support enabled and PipeWire connected, native-control publishes activity
through `gnoblin.privacy.state()`
and the `camera-monitor` and `microphone-monitor` capabilities. Camera activity
remains active for 500 ms after the last camera node stops. The native socket
contract test checks the snapshot and capability wiring. The isolated
`tests/test-privacy-pipewire.py` fixture also drives a synthetic microphone and
camera-role stream through a private PipeWire graph and verifies the live Lua
privacy state transitions without changing the host graph.

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
3. **Keep the standalone session native.** The supported build, login entry,
   and control API use `gnoblin` and Lua; they do not run GNOME Shell or GJS.
   Native-control validates requests and dispatches compositor-owned
   operations through Mutter's versioned API; `gnoblinctl` is a native client.
   Keep compositor policy in Lua and use standard Wayland interfaces where
   they cover the operation. Preserve old-prefix cleanup while existing
   installations upgrade. Put any newly found behavior gap in the Lua/native
   API rather than restoring the retired Shell runtime path.
   The tracked GNOME Shell corner and border geometry helpers and their
   JavaScript-only unit fixtures have been removed. Lua owns the window-rule
   policy, Mutter owns clipping and effects, and the standalone devkit keeps
   native border, shadow, and CSD reconstruction checks. The remaining
   historical Shell patch series is not built or packaged by this checkout;
   its audited migration and retirement status is tracked in
   [the Shell patch retirement audit](gnome-shell-retirement.md). The old
   `52-live-shell-config` and
   `55-window-rules` resource patches were audited and removed in commits
   `4051113a` and `57672e7e`. The old private Shell D-Bus config test was
   removed with the first patch. The supported standalone runtime owns config
   and window rules in Lua; the retained lifecycle test now exercises those
   behaviors in a fresh nested compositor session.
4. **Verify session supervision and recovery.** The durable `gnoblin` guardian
   owns Mutter, readiness, session activation and teardown, the retained
   private compositor channel, and one-shot initial autostart. Its restartable
   supervisor owns Lua configuration and runtime dispatch, and can recover
   after either the Lua worker or the supervisor exits while Mutter remains
   alive. The devkit verifies worker and supervisor recovery, including the
   same Mutter PID and no repeated autostart after supervisor recovery. A
   Mutter restart still disconnects Wayland clients and loses their windows.
   Verify login, logout, and compositor-failure cleanup through logind on a
   real seat before claiming the complete session lifecycle. The runtime owns
   Lua config loading and reload; each setting still needs a clear
   live-versus-restart requirement because protocol registration and other
   startup-only compositor settings cannot become live merely by moving Lua
   out of Mutter. Shell projects such as Bingux remain separately installed
   clients, not Gnoblin session internals.
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
   At that checkpoint, built-in Shell actions and popup input capture still
   required Shell.
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
   explicit Shell-required error. Native-control API 1.22 later added held and
   modal `gnoblin.shortcuts.bind` sessions for Lua and shell clients. Modal
   sessions deliver captured key press and release events while their held
   modifier is down; bare-Super bindings can capture type-ahead input when
   Mutter's early modifier hook is available. These APIs let an external shell
   own shortcut behavior and UI without GNOME Shell. Named GNOME Shell actions
   remain unsupported, and static
   `gnoblin.configure.shortcuts.*.capture_input` remains unsupported. Real-seat
   shortcut behavior and a complete standalone login still need verification.
   On Fedora 43, a clean permanent worktree completed the full source build
   with GCC 15.3.1. Host versions of GNOME schemas (49.1), xdg-desktop-portal
   (1.20.4), and GTK (4.20.4) were below the pinned GNOME 51 build floors, so
   matching schemas, portal, and GTK sources were built in a private prefix;
   Lua and PipeWire came from an existing private dependency prefix, and
   fuse3 development headers were staged from Fedora's RPM without installing
   packages on the host. A fresh devkit session answered `gnoblinctl ping`,
   listed workspaces, and moved a live Foot window to workspace 2 through
   `gnoblinctl window workspace`; the window snapshot reported
   `workspace_id: "@session-2"`. The devkit now supplies a 1280×720 virtual
   output. This verifies the native build and nested control path, not a real
   login on a seat; real-seat login lifecycle verification remains open. On
   October 1, 2026, a clean worktree at `6415a974`
   rebuilt the complete session on Fedora 43 with GCC 15.3.1 and the existing
   local GNOME 51 and Lua development prefixes. A fresh devkit session listed
   configured `us` and `gb` XKB sources. It selected `gb` with
   `gnoblinctl input select` and confirmed it active. Fedora's installed schema
   version remained 49.1, so this does not establish an unassisted stock-host
   build. The same session verified both CLI version forms; they reported the
   Git remote and full SHA alongside the Gnoblin, GNOME, Mutter, portal, Lua,
   native API, and build-ID versions.
   On October 2, a clean permanent worktree at commit `61dd3c45` completed
   `./build.sh --jobs 8` on Fedora 43 using a private dependency prefix with
   GNOME schemas 51.0, GTK 4.22.5, PipeWire 1.6.0, and xdg-desktop-portal
   1.21.1. The fresh `tests/test-gnoblin-devkit.sh` session passed Lua config
   reload, native CLI calls, workspace switching, Lua state snapshots, and
   Lua-worker recovery while retaining the same compositor. The installed
   `gnoblinctl` CLI smoke test passed, and its version output reported the
   build's Git remote, full SHA, Gnoblin, GNOME, Mutter, Lua, and API versions.
   This verifies the source build and nested development session. Fedora 43's
   configured repositories still provide older versions, so the build needs
   those newer development packages supplied through a prefix. A real-seat
   login is still open. On October 2, commit `cf4c29ff` built the full session
   in a second fresh worktree with the same private dependency prefix. The
   updated `gnoblinctl lua` CLI smoke test passed against the installed client, and
   `tests/test-gnoblin-devkit.sh` passed. A separate nested devkit session
   opened a Foot window and verified Lua window lookup, immutable properties,
   minimize, and restore through the live compositor socket. This validates
   the CLI Window-record path but does not close the remaining API parity or
   real-seat login work. On October 3, 2026, commit `8456879c` built the
   default session on Fedora 43 with host GLib 2.86.5, schemas 49.1, and Lua
   5.4.0; the optional portal component was not built. The same worktree then
   built Mutter's devkit into its writable private prefix, and
   `GNOBLIN_PREFIX=<worktree>/install tests/test-gnoblin-devkit.sh` passed Lua
   config and native-control checks plus worker and supervisor recovery while
   retaining Mutter and running login autostart once. This verifies the fresh
   nested session, not a real-seat login or physical input. The standalone
   login lifecycle and remaining API parity work are still open.
   Later on October 3, commit `6eba868d` built the full session on Fedora 43
   with `./build.sh --jobs 4` (log:
   `build/logs/build-20261003-005839-485272.log`). The 51-case
   `tests/native-socket-text-snap-api.test.py` contract suite passed, and a
   fresh `tests/test-gnoblin-devkit.sh` run passed Lua/native API and worker
   recovery checks. This verifies compilation and the nested session path; it
   does not verify focus-context delivery from real key input.
5. **Narrow the remaining forks.** Keep the portal frontend protocol and
   backend selection standard. Move Gnoblin's portal implementation out of the
   GNOME backend fork only after its dialogs, capture, permissions, and GNOME
   coexistence behavior have replacement implementations.

Each cut needs a source build, package-resolution check, GNOME coexistence
check, and a graphical login on a real seat. A smaller dependency list alone
does not establish a usable session.

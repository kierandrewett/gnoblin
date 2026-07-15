# Verifying gnoblin on real hardware

`just verify` builds the current source and runs deterministic checks plus every
isolated GNOME Shell integration recipe headlessly. `just verify-release` also
runs Mutter's native/Wayland/focus suites on the host and builds both RPMs.
Neither gate can prove a GDM login, visible bring-your-own chrome, interactive
portal consent, or an installed system-package transaction. This checklist
covers those boundaries.

**Fastest path to eyeball it without logging out:** `just gnome-devkit` opens a *nested*
gnoblin session (a window in your current Wayland session) + a terminal wired to it; run
your chrome (`qs -p ~/dev/kobel-shell`) from that terminal. Everything below can be poked
from there too. The full login-session checks (§1) still matter for GDM/session wiring.

## 0. Build & install

```sh
just init          # fetch pinned source checkouts and Meson subprojects
just dev           # patched mutter + patched gnome-shell + session data -> ./install
```

## 1. Log in to a real gnoblin session

```sh
just dev-session              # installs gnoblin.desktop + gnome-session + session mode
just dev-session-register     # links the systemd --user units, prints the (root) command
```

`dev-session-register` links the gnoblin-specific `org.gnoblin.Shell.target` /
`@wayland.service` systemd --user units (does not touch the shared
`org.gnome.Shell@wayland.service`, so this can't affect a system GNOME
session) and prints the `sudo install` command that copies `gnoblin.desktop`
into `/usr/share/wayland-sessions/` — the one step it can't do for you,
since login managers only read session `.desktop` files from a fixed system
directory. Run that, then pick **Gnoblin** at GDM. Full detail:
[Installation § install the session for real](installation.md#install-the-session-for-real).

Expect: a bare session — **no top bar, no dash, no overview** (stripped by the session
mode), your windows managed by mutter, and **nothing drawing chrome until you run a
layer-shell client**. That's by design: chrome is bring-your-own.

Sanity from a terminal in the session:

```sh
gnoblinctl ping          # -> pong
gnoblinctl version       # -> 49.6-gnoblin
gnoblinctl features      # osd, screenshot, ...
```

## 2. Bring-your-own chrome (Quickshell / waybar / …)

gnoblin advertises `zwlr_layer_shell_v1` v5, so any layer-shell client draws the bar.

```sh
qs -p ~/path/to/shell.qml        # Quickshell
# or: waybar
```

- If Quickshell warns *"built against Qt X but system has Qt Y … must be rebuilt"*,
  rebuild the quickshell package first — a stale build crashes before it maps a surface.
- A client's surface should appear at the anchored edge. Drive gnoblin from QML/JS via
  the `org.gnoblin.Shell` D-Bus interface (see the control protocol in the README).

## 3. Feature toggles (let your chrome own subsystems)

```sh
gnoblinctl disable osd            # your bar's OSD owns all volume/brightness popups
gnoblinctl disable osd-volume     # ...or just the volume OSD (per-type: osd-microphone,
                                  #    osd-brightness, osd-keyboard-brightness)
gnoblinctl disable screenshot     # your own screenshot UI owns it
gnoblinctl disable notifications  # gnome releases org.freedesktop.Notifications; start
                                  #    your external notification daemon to own it
gnoblinctl enable osd             # hand any of them back to gnome-shell
```

Change the volume with the media keys: with `osd` (or `osd-volume`) disabled, gnome-shell
shows no popup. With `notifications` disabled, run e.g. a quickshell notification service
and confirm it claims `org.freedesktop.Notifications` (`gdbus call ... NameHasOwner`).

## 4. Wayland soft-reload (windows survive)

Edit an extension or a `~/.config/gnoblin/scripts/*.js`, then:

```sh
gnoblinctl reload      # or Alt+F2, r
```

Your windows and your chrome stay up (mutter is never torn down); the JS layer reloads.

## 5. Extensions + scripting

```sh
# Sideload: drop an extension in ~/.local/share/gnome-shell/extensions/<uuid>/ and enable it
gnoblinctl extensions
gnoblinctl reload-ext <uuid>      # hot-reload its code after an edit

# Mutter-hooking extensions (e.g. Rounded Window Corners Reborn, Blur-my-Shell) work;
# top-bar/dock/overview extensions do not (that chrome is gone).

gnoblinctl scripts
gnoblinctl reload-scripts
```

## 6. Mutter's own test suite

```sh
just test-mutter
```

Validates that gnoblin's mutter patches (layer-shell, protocol overlays, WM/crash fixes)
don't regress mutter. The native/Wayland backend tests boot a compositor that monitors an
ICC profile directory, so they need a real environment with a working local file monitor
(inotify) and a seat — in a restricted sandbox they all bail with *"Unable to find default
local file monitor type"* (exit 251), which is environmental, not a regression. The unit
tests (no backend) pass anywhere.

## 7. Persistent Screen Cast and Remote Desktop grants

```sh
sudo dnf install xdg-desktop-portal-devel
just dev-portal
```

Run the patched backend so it owns
`org.freedesktop.impl.portal.desktop.gnome`, then connect the application:

```sh
./install/libexec/xdg-desktop-portal-gnome -r
```

On the first connection, expect the normal Screen Cast source picker or Remote
Desktop consent dialog. The remember checkbox is shown only when Gnoblin can
derive a trustworthy requester identity and can restore the selected source
exactly. Select it and approve the request.

On a later matching request, expect no dialog: the backend restores only the
approved monitor selection, input-device mask, and clipboard state. It prompts
again if the requester asks for broader capabilities or an approved monitor is
no longer available. Window selections and other source shapes that cannot be
matched safely remain one-shot approvals.

Grants live under `$XDG_DATA_HOME/gnoblin/portal-grants/<kind>/`, which defaults
to `~/.local/share/gnoblin/portal-grants/<kind>/`. Sandboxed apps use a verified
`app-id:<id>` identity. Unsandboxed callers use
`host-exe:<canonical-executable-path>`. Filenames are opaque SHA-256 digests.

```sh
gnoblinctl portal-grants
gnoblinctl revoke-grant <kind> <id>
```

The gnoblin Settings panel in the next section shows the same typed list and
provides a Revoke button for each record.

## 8. gnoblin Settings (forked gnome-control-center)

```sh
sudo dnf install accountsservice-devel colord-gtk4-devel cups-devel gsound-devel ibus-devel \
  libgtop2-devel libnma-gtk4-devel malcontent-devel ModemManager-glib-devel libpwquality-devel \
  libsmbclient-devel libudisks2-devel
just dev-settings                 # builds the fork + hides the multitasking panel
./install/bin/gnome-control-center gnoblin
```

> g-c-c compiles `.blp` UI files with `blueprint-compiler` (≥ 0.17). `dev-settings`
> handles it automatically — it uses the system package if present, else links the
> meson-wrap's source package next to its launcher (the wrap's launcher otherwise can't
> import itself). Installing `sudo dnf install blueprint-compiler` is the cleaner path
> but not required. **Build-verified**: the `gnoblin` panel compiles + links + is listed
> by `gnome-control-center --list`.

Expect: GNOME Settings with a **gnoblin** panel, switch rows for every
`gnoblinctl features` toggle, typed Screen Cast and Remote Desktop grant rows
with capability summaries and Revoke buttons, and a **Reload gnoblin** button.
The **Multitasking** panel is gone. With `./install/bin` ahead on `PATH`, "open
Settings" or `gnome-control-center` launches this fork.

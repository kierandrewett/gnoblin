# Verifying gnoblin on real hardware

Everything in gnoblin's automated suite runs headless (`just gnome-verify`,
`gnome-dbus-verify`, `gnome-hot-reload-verify`, `gnome-scripting-verify`,
`gnome-notifications-verify`, `gnome-protocol-gating-verify`, `gnome-devkit-verify`,
`test-mutter`, `test`). A few things can only be confirmed on a real GPU / a real
login session / with root. This is that checklist.

**Fastest path to eyeball it without logging out:** `just gnome-devkit` opens a *nested*
gnoblin session (a window in your current Wayland session) + a terminal wired to it; run
your chrome (`qs -p ~/dev/kobel-shell`) from that terminal. Everything below can be poked
from there too. The full login-session checks (§1) still matter for GDM/session wiring.

## 0. Build & install

```sh
just init          # fetch pinned submodules
just dev           # patched mutter + patched gnome-shell + session data -> ./install
```

## 1. Log in to a real gnoblin session

`just dev-session` installs `gnoblin.desktop`, the `gnoblin` gnome-session, and the
`gnoblin` GNOME Shell session mode into the prefix. To offer it at the login manager,
the prefix's share dir must be on `XDG_DATA_DIRS` for GDM (or install to a system
prefix). Then pick **Gnoblin** at GDM.

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

## 7. Per-app persistent screencast grants (macOS-style, rustdesk)

```sh
sudo dnf install xdg-desktop-portal-devel      # the one build dep for the portal
just dev-portal                                # build the patched backend into ./install
```

Run the patched backend so it owns `org.freedesktop.impl.portal.desktop.gnome`
(replacing the system one for the test), then connect rustdesk:

```sh
./install/libexec/xdg-desktop-portal-gnome -r
```

Expect, **first** connection: the normal ScreenCast source-picker / RemoteDesktop consent
dialog, now with an **"Always allow this app"** checkbox. Tick it and approve.

Expect, **every subsequent** connection from that app: **no dialog** — screen + input are
granted straight away (all monitors). Apps you never ticked still prompt each time. This is
the macOS "Screen Recording" model: grant once per app, never re-asked.

The grant is a file per app under `~/.config/gnoblin/portal-grants/` (keyed on the app-id,
or the app's executable name for unsandboxed apps like rustdesk). Manage it:

```sh
gnoblinctl screen-grants          # list apps with a persistent grant
gnoblinctl revoke-grant <id>      # revoke one (or rm ~/.config/gnoblin/portal-grants/<id>)
```

(The gnoblin Settings panel — §8 — shows the same list with a Revoke button per app.)

## 8. gnoblin Settings (forked gnome-control-center)

```sh
sudo dnf install accountsservice-devel colord-gtk4-devel cups-devel gsound-devel ibus-devel \
  libgtop2-devel libnma-gtk4-devel malcontent-devel ModemManager-glib-devel libpwquality-devel \
  libsmbclient-devel libudisks2-devel
just dev-settings                 # builds the fork + hides the multitasking panel
./install/bin/gnome-control-center gnoblin
```

Expect: GNOME Settings with a **gnoblin** panel — switch rows for every `gnoblinctl features`
toggle (flip one, confirm the subsystem changes live), the screencast per-app grants with a
Revoke button each, and a **Reload gnoblin** button. The **Multitasking** panel is gone. With
`./install/bin` ahead on `PATH`, "open Settings" / `gnome-control-center` launches this fork.

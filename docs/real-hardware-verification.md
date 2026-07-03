# Verifying gnoblin on real hardware

Everything in gnoblin's automated suite runs headless (`just gnome-verify`,
`gnome-dbus-verify`, `gnome-hot-reload-verify`, `gnome-scripting-verify`,
`test-mutter`, `test`). A few things can only be confirmed on a real GPU / a real
login session / with root. This is that checklist.

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

## 7. Unattended screensharing (rustdesk)

```sh
sudo dnf install xdg-desktop-portal-devel      # the one build dep for the portal
just dev-portal                                # build the patched backend into ./install
```

Enable the OPT-IN auto-grant (off by default — the stock dialog is unchanged unless you do this):

```sh
mkdir -p ~/.config/gnoblin && touch ~/.config/gnoblin/portal-autogrant
# or: export GNOBLIN_PORTAL_AUTOGRANT=1
```

Run the patched backend so it owns `org.freedesktop.impl.portal.desktop.gnome`
(replacing the system one for the test), then connect rustdesk:

```sh
GNOBLIN_PORTAL_AUTOGRANT=1 ./install/libexec/xdg-desktop-portal-gnome -r
```

Expect: **no ScreenCast source-picker / RemoteDesktop consent dialog** — screen + input
are granted (all monitors). Remove the flag/file → the dialog returns.

Note: `persist_mode` + restore-token (unattended *after* one manual grant) already works
with stock GNOME; the patch above is for bypassing even the first dialog.

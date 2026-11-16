# Troubleshooting

Start with the symptom. Keep the exact error and `gnoblinctl version` output
when reporting a problem.

## Gnoblin is missing from the login screen

Check that the `gnoblin` package is installed. On Fedora, run:

```sh
rpm -q gnoblin
```

For source builds, run `make` and then `make install`.
It asks for sudo to copy the build to its prefix and add the login entry. See [session registration](install-source.md#login-session).

## The source build returns to the login screen

Log into another session or a text console and read the startup records:

```sh
cat "${XDG_STATE_HOME:-$HOME/.local/state}/gnoblin/session-last.log"
cat "${XDG_STATE_HOME:-$HOME/.local/state}/gnoblin/compositor-last.log"
```

The session record shows startup stages and process exit statuses. The
compositor record contains its error output. Each file is limited to 64 KiB;
the previous attempt is kept as `session-previous.log` and
`compositor-previous.log` in the same folder.

Also check the session journal:

```sh
journalctl -b --no-pager -g gnoblin
```

If it reports a missing Gnoblin or Mutter library, run `make` and then
`make install` to install the build. The `PREFIX` for `make` and `make install`
must be the same.

Do not copy an executable out of `build/` by hand. Its library paths point at the
build directory and can fail under the login manager. Run `make install` instead.
It checks that the installed executable loads its libraries without
`LD_LIBRARY_PATH` set. Run it again after a rebuild or if the login entry changed.

The installed session does not need the source directory. If an earlier install
left links in your home directory that point into it, run `make install` again to
remove them.

GDM logs “Session never registered” when a session ends before it registers.
Gnoblin's login entry sets `X-GDM-SessionRegisters=false`, so GDM's session
wrapper registers the session itself about 10 seconds after it starts.

Check which case applies:

- The session ended within 10 seconds of starting. The warning is expected, and
  reinstalling does not prevent it.
- The session ran longer than 10 seconds. Check that the entry says
  `X-GDM-SessionRegisters=false`, and run `make install` again if it does not.
  If the message still appears, include the output of `journalctl -b -u gdm` in
  your bug report.

If no configuration exists, the session runs the stock TypeScript config. `gnoblinctl config init` writes a starter `~/.config/gnoblin/shell.ts`
and never replaces an existing file.

Gnoblin starts with its embedded configuration before applying the user
configuration. If loading `shell.ts` or the native settings fail, it tries the last
accepted settings, then keeps embedded defaults if those also fail.

The top-left ImGui recovery panel appears above application and shell layers
and shows the error. Choose **Open config** to open its folder, fix the config
in an editor, then choose **Reload** or run
`gnoblinctl config reload` from a terminal. Gnoblin keeps failed files unchanged. See
[the TypeScript shell](/typescript-shell#with-no-shell-ts).

The panel requires a running compositor. A missing library or a native
compositor crash prevents it from displaying; inspect the login logs above
for those failures.

The panel also reports config-owned autostart commands that cannot start or
exit with an error. It names the failed component and its exit status or signal.

## No bar, dock or launcher

Gnoblin does not include a desktop shell.
[Install one](bring-your-own-shell.md) if you have not already.

Gnoblin reports failed config-owned autostart commands in its recovery panel.
Choose **Open config**, **Terminal**, or **Reload** to investigate. **Terminal**
is disabled when no supported terminal is installed. The panel hides while
the session is locked. Choose **Dismiss** to hide the current notice; a new
error opens it again.

If the panel does not appear, switch to another session or a text console and
inspect Gnoblin's session logs for config-owned shell clients:

```sh
journalctl -b --no-pager | rg -i 'gnoblin'
```

## A network needs a browser sign-in

Open the network's sign-in page in a browser. The NetworkManager secret agent handles Wi-Fi and VPN credential prompts.

## A config edit does nothing

```sh
gnoblinctl config reload
```

Edit `~/.config/gnoblin/shell.ts` and read the reload error. Then check:

- Does this setting restart a child service, and does that affect its clients?
  See [reload behavior](/guides/files_and_load_order#reload).
- Did an `import` in `shell.ts` name a file that is not there?
- Did the file bind a trigger twice? A second binding of one trigger at one priority is refused.

## My component's shortcuts disappeared

A `shell.ts` replaces the default entirely, so the keys, workspaces and titlebars the default made are gone unless your
file makes them. Run `gnoblinctl config init` to write the default as code, then keep the lines you want.

## A shortcut fails

Run its command in a terminal. Check that the executable is on PATH.
Command arguments do not expand `~`, `$HOME` or pipes.

Check for another shortcut in your Gnoblin config that uses the same binding.
Use `gnoblin.configure.keybindings` to change a built-in action. Set
`enable = false` on a named `gnoblin.configure.shortcuts` entry to disable it.
See [shortcut conflicts](/guides/shortcuts#avoid-conflicts).

## Shell integration errors

| Symptom                                                | Check                                                                                                                                                                                |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `gnoblinctl window list` cannot connect                | Run `gnoblinctl status --json` and check `windowControlError` and the active socket path. See [connection details](gnoblinctl.md#connection-problems).                               |
| A bridge request says a window is unavailable          | Get a fresh ID from `gnoblinctl window list --json` or a `windows` snapshot. IDs expire when windows close.                                                                          |
| A Wayland client cannot bind a Gnoblin interface       | Inspect the registry inside the Gnoblin session, then check its [protocol gate](wayland-protocols.md). Changes apply after config reload; existing client bindings remain connected. |
| A layer appears on the host desktop during devkit work | Launch it from the devkit environment and check `WAYLAND_DISPLAY`. The [devkit guide](devkit.md) explains the nested display.                                                        |

## Removing a setting does not reset it

Built-in bindings and native feature booleans persist in GSettings.
Set the desired value explicitly. Removing an autostart entry does not stop
its running process.

## Blur is invisible

The application or panel must draw a translucent background.
Rule opacity fades text too; it does not replace client background transparency.
See [blur](/guides/window_effects#background-blur-from-applications).

## Gnoblin is slow or stutters after login

Check the scheduling policy of the compositor:

```sh
chrt -p "$(pgrep -x gnoblin | head -n 1)"
```

The policy should be `SCHED_OTHER`. A process tuner such as Ananicy can start Gnoblin as background work. The policy
is then `SCHED_IDLE`, and the session misses its frame and input deadlines.

### Ananicy-cpp rules {#ananicy-cpp-rules}

Install the `gnoblin-ananicy-rules` package from [Gnoblin's package repository](package-repositories.md). It gives
Gnoblin normal scheduling:

```sh
sudo dnf install gnoblin-ananicy-rules        # Fedora
sudo zypper install gnoblin-ananicy-rules     # openSUSE
sudo apt install gnoblin-ananicy-rules        # Debian
sudo pacman -S gnoblin-ananicy-rules          # Arch Linux
```

On another system, copy `99-gnoblin.types` and `99-gnoblin.rules` from the `data/ananicy/` folder of the source
to `/etc/ananicy.d/`. Then run `sudo systemctl restart ananicy-cpp`.

Log out and back in, then check the policy again. To go back to the daemon's defaults, remove the two files or the
package.

## The session ended and returned to the login screen

If the compositor stops on its own, every Wayland app stops with it, because
apps cannot survive their compositor. GDM then shows the login screen. Gnoblin
does not restart the compositor in place.

Log in again. If the compositor ended on a signal other than an orderly stop
request, the recovery panel appears with the signal and the time the session
ran, for example `The compositor was stopped by signal 11 (Segmentation fault)
after 53 seconds`. The panel shows once. A clean logout never shows it.

The logs from the session that ended stay available until the next login after
this one:

```sh
cat ~/.local/state/gnoblin/session-previous.log
tail -n 40 ~/.local/state/gnoblin/compositor-previous.log
coredumpctl list --no-pager | rg gnoblin
```

`session-previous.log` records the exit as `compositor pid=... killed
signal=11`. Keep these files when you report the problem.

## Portal requests fail or apps start slowly

Look for a portal backend that cannot start:

```sh
journalctl --user -b --no-pager | rg 'Failed at step EXEC'
```

A line such as `xdg-desktop-portal-gnome.service: Failed at step EXEC spawning
.../install/libexec/xdg-desktop-portal-gnome: No such file or directory` means
a user override points the service at a file that does not exist. Apps then
wait for the failed backend whenever they read settings or open a dialog.

Show the unit with its overrides and note the path in `ExecStart`:

```sh
systemctl --user cat xdg-desktop-portal-gnome.service
```

A drop-in from a local build is the usual cause. Move it away and reload:

```sh
mv ~/.config/systemd/user/xdg-desktop-portal-gnome.service.d ~/xdg-desktop-portal-gnome.service.d.bak
systemctl --user daemon-reload
systemctl --user reset-failed xdg-desktop-portal-gnome.service
```

Restart the session so apps reconnect to the portal.

## X11 apps closed but Wayland apps kept running

If Xwayland stops on its own, every X11 app closes. The compositor and the Wayland
apps keep running. Start the X11 app again. Xwayland starts again when the first X11
app connects.

Xwayland does not run at login. `pgrep -x Xwayland` prints nothing until you start an
X11 app. This is expected.

If the same X11 app closes each time, run it from a terminal and read its output. If
Xwayland itself keeps stopping, report it with the compositor log:

```sh
tail -n 40 ~/.local/state/gnoblin/compositor-last.log
```

## X11 apps look too large on a fractional-scale display

With a fractional display scale such as 1.25, Mutter rounds the X11 scale up to
the next whole number. X11 apps then:

- report `Xft.dpi: 192` instead of 96
- draw larger than Wayland apps at the same font size
- show small titlebar buttons

Check the value an X11 app sees:

```sh
xrdb -query | rg Xft.dpi
```

The X11 scale is the `xwayland` section of the compositor's configuration, and `shell.ts` cannot set it: no object of the
shell API carries it. See [Settings with no object](/js-settings).

## A window opens behind the focused window

A window from an app that is not the focused app can open behind the focused
window, and the compositor sets an attention flag on it. The flag does not
produce a notification by itself.

The focus policy that decides this is not something `shell.ts` can set: no object of the shell API carries it (see
[Settings with no object](/js-settings)). Check the windows with `gnoblinctl window list --json`; a hidden window has
`"focused": false`. To bring one forward from a script, call `client.focus()` and `client.raise()` on it.

### `wl-copy` does not return

When a new window does not take the keyboard focus, `wl-copy` from `wl-clipboard` can block until you stop it. It opens a
small window and waits for keyboard focus before it sets the clipboard. Click the window to give it the focus.

## An app cannot take a screenshot

Portal screenshots capture the entire desktop. Allow the request when asked.
Interactive requests show a confirmation dialog with an **Include pointer**
option, off by default. Cancel returns no image. Region selection, window
selection and the portal's `PickColor` method are not available.

Capture fails while the session is locked or capture is inhibited. Unlock the
session and try again. Check that PipeWire and the Gnoblin portal backend are
running:

```sh
systemctl --user status pipewire xdg-desktop-portal-gnoblin
```

A log that mentions `org.gnome.Shell.Screenshot` means another portal backend is
still running. Log out and back in to start the Gnoblin backend.

For a separate screenshot tool, use your shell's capture action or run it on the
privileged socket. Plain `grim` on the ordinary socket fails with no screencopy
global, on purpose:

```sh
WAYLAND_DISPLAY=$GNOBLIN_PRIVILEGED_DISPLAY grim ~/screenshot.png
```

`GNOBLIN_PRIVILEGED_DISPLAY` is set for the programs the session starts. A shell
that the session did not start does not have it set, so run the command from a
shell the session started, such as a terminal opened from the launcher.

The variable is a convenience, not a lock. The socket is owner-only, so any
program you run as your user can reach it by its path, with or without the
variable.

## The screen stays locked after the lock screen closes

If the lock client crashes or is killed, Gnoblin keeps the session locked. This
is deliberate: the lock state becomes `failsafe`, and the screen and input stay
locked until a lock client takes over and unlocks.

Check the state from another login, such as a text console or an SSH session:

```sh
export XDG_RUNTIME_DIR=/run/user/$(id -u)
gnoblinctl status
```

`lock_state` reads `failsafe` after a lock client failure.

Start your lock client again. It takes over the lock, and you can unlock it as
usual. For example, with `swaylock`:

```sh
WAYLAND_DISPLAY=wayland-0 swaylock
```

Use the display name from the session, shown by `ls $XDG_RUNTIME_DIR | rg wayland`.
After you unlock, `lock_state` reads `unlocked`.

## Apps reach gnome-keyring instead

Gnoblin cannot stop every route by which another program takes `org.freedesktop.secrets`.
Check these first, in order:

1. The services log names the owner. Look for `org.freedesktop.secrets is already owned by`
   or `held by another program`: another keyring took the name before Gnoblin's started.
   Stop that program, or set `keyring.enabled = false` and use it on purpose.
2. An autostart entry that starts `gnome-keyring-daemon` is refused at load while the keyring
   is on, with a message naming the entry. A shell string that hides the daemon in one word
   is not caught.
3. Not caught by Gnoblin: PAM `pam_gnome_keyring`, the D-Bus activation file that gnome-keyring
   installs, and gnome-keyring's systemd user units. Remove or mask them yourself. The full
   list, and what was tested, is in `docs/config/configure/keyring.md`.

```sh
journalctl -b --no-pager | rg 'held by another program|already owned by|gnome-keyring'
```

## Read the logs

```sh
journalctl -b --since '10 minutes ago' --no-pager | rg 'gnoblin-config|gnoblin-shader'
```

Restore a saved working config and reload to undo file changes. This does not
undo launched processes or persistent GSettings values.

# Troubleshooting

Start with the symptom. Keep the exact error and `gnoblinctl version` output
when reporting a problem.

## Gnoblin is missing from the login screen

For Fedora, check the session package:

```sh
rpm -q gnoblin
```

For source builds, run `./build.sh --register-session` after building.
It asks for sudo to add the login entry. See [session registration](install-source.md#login-session).

## The source build returns to the login screen

Log into another session or a text console and read the latest startup records:

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

If it reports a missing Gnoblin or Mutter library, run `./build.sh` to install
the current build. Meson installs the executable with paths to its private
libraries. Copying an executable directly from `build/` can leave build-directory
library paths that fail under the login manager.

Session registration checks that the installed executable can load its
libraries without a developer's `LD_LIBRARY_PATH`. Register again only if the
build prefix or login entry changed.

Keep the extracted source directory after registration; the login entry runs
its private binaries from that directory.

Gnoblin does not register with GDM through GNOME Session. Its login entry must
use `X-GDM-SessionRegisters=false`. If GDM reports “Session never registered,”
update the entry with `./build.sh --register-session`.

If no configuration exists, Gnoblin loads its built-in Lua configuration. Run
`gnoblinctl init` to create an editable configuration tree from the copy
embedded in the binary. The command leaves existing files unchanged.

Gnoblin starts with its embedded configuration before applying the user
configuration. If Lua loading or native settings fail, it tries the last
accepted settings, then keeps embedded defaults if those also fail.

The top-left ImGui recovery panel appears above application and shell layers
and shows the error. Choose **Open config** to open its folder, fix the config
in an editor, then choose **Reload** or run
`gnoblinctl reload` from a terminal. Gnoblin keeps failed files unchanged. See
[configuration recovery](/config/load#recover-from-a-configuration-error).

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
journalctl -b --no-pager | rg -i 'gnoblin|bingux'
```

Bingux's search and status helpers use user services. Check those separately:

```sh
systemctl --user status bingux-searchd.service bingux-statusd.service
journalctl --user -b -u bingux-searchd.service -u bingux-statusd.service
```

A Qt/Quickshell version mismatch requires rebuilding or installing a matching
Quickshell package. Restarting repeatedly will not fix that mismatch.

## A network needs a browser sign-in

Open the network's sign-in page in a browser. Gnoblin does not install GNOME
Shell's captive-network helper. Its NetworkManager secret agent still handles
Wi-Fi and VPN credential prompts.

## Calendar events are not in the shell

Gnoblin has no GNOME Shell date menu or calendar event server. Open a
calendar application to view events.

## A config edit does nothing

```sh
gnoblinctl config path
gnoblinctl config reload
```

Edit the printed path and read the reload error. Then check:

- Does this setting restart a child service, and does that affect its clients?
  See [reload behavior](/guides/files_and_load_order#reload-and-persistence).
- Did an unmatched include glob load nothing?
- Did a later list replace your rules or shortcuts?

## My component's shortcuts or rules disappeared

Nonempty lists in `gnoblin.configure` replace earlier lists.
[Append to the existing list](/guides/files_and_load_order#override-or-append)
or edit its entries after the component loads.

## A window rule does not match

Every matcher must match. Text matchers use case-sensitive Lua patterns, not
JavaScript or PCRE regex. Check anchors, escaping (`%.` matches a literal
dot), and the raw app ID. See [window rules](/guides/window_rules).

## A shortcut fails

Run its command in a terminal. Check that the executable is on PATH.
Command arguments do not expand `~`, `$HOME` or pipes.

Check for another shortcut in your Gnoblin config that uses the same binding.
Use `gnoblin.configure.keybindings` to change a built-in action. Set
`enable = false` on a named `gnoblin.configure.shortcuts` entry to disable it.
See [shortcut conflicts](/guides/shortcuts#avoid-conflicts).

## Shell integration errors

| Symptom                                                | Check                                                                                                                                                  |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `gnoblinctl window list` cannot connect                | Run `gnoblinctl status --json` and check `windowControlError` and the active socket path. See [connection details](gnoblinctl.md#connection-problems). |
| A bridge request says a window is unavailable          | Get a fresh ID from `gnoblinctl window list --json` or a `windows` snapshot. IDs expire when windows close.                                            |
| A Wayland client cannot bind a Gnoblin interface       | Inspect the registry inside the Gnoblin session, then check its [protocol gate](wayland-protocols.md). Changes apply after config reload; existing client bindings remain connected. |
| A layer appears on the host desktop during devkit work | Launch it from the devkit environment and check `WAYLAND_DISPLAY`. The [devkit guide](devkit.md) explains the nested display.                          |

## Removing a setting does not reset it

Built-in bindings and native feature booleans persist in GSettings.
Set the desired value explicitly. Removing an autostart entry does not stop
its running process.

## Blur is invisible

The application or panel must draw a translucent background.
Rule opacity fades text too; it does not replace client background transparency.
See [blur](/guides/window_effects#blur-and-opacity).

## There are two titlebars, or none

Check your [frame mode](/guides/window_frames#choose-a-mode).
A renderer name alone does not enable SSD, and an unset protocol preference
does not prove that the client draws a titlebar.

## Gnoblin is slow or stutters after login

Check the scheduling class of the compositor:

```sh
chrt -p "$(pgrep -x gnoblin | head -n 1)"
```

The expected policy is `SCHED_OTHER`. If a display manager or a process
tuner such as Ananicy starts Gnoblin as background work, the policy is
`SCHED_IDLE` and the session misses its frame and input deadlines.

At startup Gnoblin resets an idle or batch policy, a positive nice value and
the idle I/O class. When a normal reset is not allowed, it asks RealtimeKit
for a higher priority. Both the session guardian and the compositor log the
result to the system journal:

```sh
journalctl -b --no-pager | rg 'scheduling policy'
```

A line such as `gnoblin-guardian: scheduling policy 5->0, nice 16->-11`
shows that Gnoblin corrected the inherited priority. If the policy is still
`SCHED_IDLE`, check that `rtkit-daemon` is running.

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

A drop-in under `~/.config/systemd/user/xdg-desktop-portal-gnome.service.d/`
left over from an old local build is the usual cause. Move it away and reload:

```sh
mv ~/.config/systemd/user/xdg-desktop-portal-gnome.service.d ~/xdg-desktop-portal-gnome.service.d.bak
systemctl --user daemon-reload
systemctl --user reset-failed xdg-desktop-portal-gnome.service
```

Restart the session so apps reconnect to the portal.

## X11 apps look too large on a fractional-scale display

With a fractional display scale such as 1.25, Mutter rounds the X11 scale up to
the next whole number. Stock GNOME Shell does the same. X11 apps then:

- report `Xft.dpi: 192` instead of 96
- draw larger than Wayland apps at the same font size
- show small titlebar buttons

Check the value an X11 app sees:

```sh
xrdb -query | rg Xft.dpi
```

To draw X11 apps at their normal size, set the X11 scale to 1:

```lua
gnoblin.configure {
    xwayland = {scaling_factor = 1},
}
```

Reload the config. Xwayland does not restart, and X11 apps then report
`Xft.dpi: 96`.

Use `0` to return to the automatic value. The accepted values are listed in the
[xwayland reference](/config/configure/xwayland).

## A window opens behind the focused window

A window from an app that is not the focused app can open behind the focused
window.

The default `focus_new_windows = "strict"` keeps the focused window in front
and sets an attention flag on the new window. GNOME Shell turns that flag into a
notification. Gnoblin has no shell, so nothing shows it.

The starter config raises modal dialogs, such as a portal prompt, with a
`gnoblin.window.created` handler in `config/10-windows.lua`. If you replaced
that file, add the handler back:

```lua
gnoblin.on("gnoblin.window.created", function(event)
    if event.window.modal then
        event.window:set_above(true)
    end
end)
```

Other windows from background apps stay behind the focused window. Check them
with `gnoblinctl window list --json`; a hidden window has `"focused": false`.

Password dialogs from the keyring or GPG are the common case. They open on top
but without focus, so typing does nothing until you click the dialog. Either
set `focus_new_windows = "smart"` (below), or show these prompts in your shell
with the [prompt broker](/config/configure/prompts).

To let any app take focus when it opens a window, set `smart`. A background app
can then interrupt your typing:

```lua
gnoblin.configure {
    window_management = {focus_new_windows = "smart"},
}
```

See [focus options](/config/configure/window_management#focus-and-raising).

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

## Read the logs

```sh
journalctl -b --since '10 minutes ago' --no-pager | rg 'gnoblin-config|gnoblin-shader'
```

Restore a saved working config and reload to undo file changes. This does not
undo launched processes or persistent GSettings values.

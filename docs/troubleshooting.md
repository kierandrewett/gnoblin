# Troubleshooting

Start with the symptom. Keep the exact error and `gnoblinctl version` output
when reporting a problem.

## Gnoblin is missing from the login screen

For Fedora, check the session package:

```sh
rpm -q gnoblin-session
```

For source builds, complete [session registration](install-source.md#3-add-a-login-session),
including the printed root commands. Building alone does not add a login entry.

## No bar, dock or launcher

Gnoblin does not include a desktop shell.
[Install one](bring-your-own-shell.md) if you have not already.

Right-click the desktop to open a terminal. With no visible layer surface,
the recovery panel appears after eight seconds. It cannot detect a frozen
shell that still has a visible surface.

For Bingux:

```sh
systemctl --user status bingux.service
journalctl --user -b -u bingux.service
```

A Qt/Quickshell version mismatch requires rebuilding or installing a matching
Quickshell package. Restarting repeatedly will not fix that mismatch.

## A config edit does nothing

```sh
gnoblinctl config path
gnoblinctl config reload
```

Edit the printed path and read the reload error. Then check:

- Does this setting need a [new session](/guides/files_and_load_order#reload-and-persistence)?
- Is a function missing? See [configuration compatibility](#gnoblin-or-configure-is-nil).
- Did an unmatched include glob load nothing?
- Did a later list replace your rules or shortcuts?

## `gnoblin` or `configure` is nil

An error such as `attempt to index a nil value (global 'gnoblin')` or
`attempt to call a nil value (field 'configure')` can mean your running
compositor predates the Lua functions used in these guides.

If you have just updated Gnoblin, log out and back in. Reloading the config
does not load an updated compositor. Otherwise, update through your
[installation method](installation.md). If your package does not include these
functions, keep using the [existing config syntax](/guides/files_and_load_order#existing-configs)
or [build from source](install-source.md).

On a build that supports these functions, check that your config has not
assigned another value to `gnoblin` or `gnoblin.configure`.

## My component's shortcuts or rules disappeared

Nonempty lists in `gnoblin.configure` replace earlier lists.
[Append to the existing list](/guides/files_and_load_order#override-or-append)
or edit its entries after the component loads.

## A window rule does not match

Every matcher must match. Text matchers use JavaScript regex, not Lua patterns.
Check anchors, escaping and the raw app ID. See [window rules](/guides/window_rules).

## A shortcut fails

Run its command in a terminal. Check that the executable is on PATH.
Command arguments do not expand `~`, `$HOME` or pipes.

Check for an existing binding in your component config or GNOME keybindings.
See [shortcut conflicts](/guides/shortcuts#avoid-conflicts).

## Shell integration errors

| Symptom                                                | Check                                                                                                                                                                                                                                    |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblinctl window list` cannot connect                | Run `gnoblinctl status --json`. Check `windowControlError` and the active socket path. The bridge is built into current source builds; it does not appear in `script list`. See [connection details](gnoblinctl.md#connection-problems). |
| A bridge request says a window is unavailable          | Get a fresh ID from `gnoblinctl window list --json` or a `windows` snapshot. IDs expire when windows close.                                                                                                                              |
| A Wayland client cannot bind a Gnoblin interface       | Inspect the registry inside the Gnoblin session, then check its [protocol gate](wayland-protocols.md). Gates take effect at login, not config reload.                                                                                    |
| A layer appears on the host desktop during devkit work | Launch it from the devkit environment and check `WAYLAND_DISPLAY`. The [devkit guide](devkit.md) explains the nested display.                                                                                                            |

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

## Read the logs

```sh
journalctl -b --since '10 minutes ago' --no-pager | rg 'gnoblin-config|gnoblin-shader'
```

Restore a saved working config and reload to undo file changes. This does not
undo launched processes or persistent GSettings values.

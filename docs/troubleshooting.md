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

- Does this setting need a [new session](configuration-loading.md#reload-and-persistence)?
- Does your installed version support it?
- Did an unmatched include glob load nothing?
- Did a later list replace your rules or shortcuts?

## My component's shortcuts or rules disappeared

Nonempty lists in `gnoblin.configure` replace earlier lists.
[Append to the existing list](configuration-loading.md#override-or-append)
or edit its entries after the component loads.

## A window rule does not match

Every matcher must match. Text matchers use JavaScript regex, not Lua patterns.
Check anchors, escaping and the raw app ID. See [window rules](window-rules.md).

## A shortcut fails

Run its command in a terminal. Check that the executable is on PATH.
Command arguments do not expand `~`, `$HOME` or pipes.

Check for an existing binding in your component config or GNOME keybindings.
See [shortcut conflicts](shortcuts.md#avoid-conflicts).

## Removing a setting does not reset it

Built-in bindings and native feature booleans persist in GSettings.
Set the desired value explicitly. Removing an autostart entry does not stop
its running process.

## Blur is invisible

The application or panel must draw a translucent background.
Rule opacity fades text too; it does not replace client background transparency.
See [blur](window-effects.md#blur-and-opacity).

## There are two titlebars, or none

Check your [frame mode](window-frames.md#choose-a-mode).
A renderer name alone does not enable SSD, and an unset protocol preference
does not prove that the client draws a titlebar.

## Read the logs

```sh
journalctl -b --since '10 minutes ago' --no-pager | rg 'gnoblin-config|gnoblin-shader'
```

Restore a saved working config and reload to undo file changes. This does not
undo launched processes or persistent GSettings values.

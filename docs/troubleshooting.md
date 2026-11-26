# Troubleshooting

Start with the symptom. Keep the exact error and `gnoblinctl version` output
when reporting a problem.

## Gnoblin is missing from the login screen

Check that the `gnoblin` package is installed. On Fedora, run:

```sh
rpm -q gnoblin
```

For a source build, install it with `just dist` or `cargo xtask dist install`. The install writes the login entry,
`share/wayland-sessions/gnoblin.desktop`, under its prefix. See [build from source](install-source.md).

## The session returns to the login screen

Log into another session or a text console and read the session's records:

```sh
gnoblinctl log --level WARN --since 10m
journalctl -b --no-pager -g gnoblin
```

`gnoblinctl log` reads `gnoblin.jsonl` in `$XDG_STATE_HOME/gnoblin/log` (see [logs](gnoblinctl.md#logs)); the journal
holds what the login manager and the user services printed.

If the journal reports a missing Gnoblin library, such as `libgnoblin-core.so.1`, the install is incomplete or the prefix
is not where the session file points. Install again with `cargo xtask dist install --prefix PREFIX`, using the same
`PREFIX` the session file names.

Run `cargo xtask dist-check --prefix PREFIX` to see which file or library is wrong. Do not copy an executable out of `target/` by hand: it finds its libraries beside the build tree and can fail under the
login manager.

GDM logs “Session never registered” when a session ends before it registers.
Gnoblin's login entry sets `X-GDM-SessionRegisters=false`, so GDM's session
wrapper registers the session itself about 10 seconds after it starts.

Check which case applies:

- The session ended within 10 seconds of starting. The warning is expected, and
  reinstalling does not prevent it.
- The session ran longer than 10 seconds. Check that the entry says
  `X-GDM-SessionRegisters=false`, and install again if it does not.
  If the message still appears, include the output of `journalctl -b -u gdm` in
  your bug report.

If no configuration exists, the session runs the stock configuration. `gnoblinctl config init` writes a starter, `shell.ts`
by default, or `shell.py` or a Rust project with `--language python` or `--language rust`, and never replaces an existing file.

Gnoblin starts with its embedded configuration before applying the user
configuration. If loading the shell file or the native settings fail, it tries the last
accepted settings, then keeps embedded defaults if those also fail. A failed load leaves your files unchanged: read the
error with `gnoblinctl log --level WARN`, fix the file, and run `gnoblinctl config reload`. See
[the TypeScript shell](/typescript-shell#with-no-shell-ts).

## No bar, dock or launcher

Gnoblin does not include a desktop shell.
[Install one](bring-your-own-shell.md) if you have not already.

If the shell does not start, read why:

```sh
gnoblinctl log --level WARN --since 10m
journalctl -b --no-pager | rg -i 'gnoblin'
```

## A network needs a browser sign-in

Open the network's sign-in page in a browser. The NetworkManager secret agent handles Wi-Fi and VPN credential prompts.

## A config edit does nothing

```sh
gnoblinctl config reload
```

Edit your shell file in `~/.config/gnoblin/` and read the reload error. Then check:

- Does this setting restart a child service, and does that affect its clients?
  See [reload behavior](/guides/files_and_load_order#reload).
- Did an `import` in the shell file name a file that is not there?
- Did the file bind a trigger twice? A second binding of one trigger at one priority is refused.

## My component's shortcuts disappeared

A shell file replaces the default entirely, so the keys, workspaces and titlebars the default made are gone unless your
file makes them. Run `gnoblinctl config init` to write the default as code, then keep the lines you want.

## A shortcut fails

Run its command in a terminal. Check that the executable is on PATH.
Command arguments do not expand `~`, `$HOME` or pipes.

Check for another binding in your shell file on the same trigger: the shell refuses a second binding of one trigger, and
the load reports it. See [shortcut conflicts](/guides/shortcuts#avoid-conflicts).

## Shell integration errors

| Symptom                                          | Check                                                                                                                                                                                |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `gnoblinctl window list` cannot connect          | Run `gnoblinctl status --json` and check `windowControlError` and the active socket path. See [connection details](gnoblinctl.md#connection-problems).                               |
| A bridge request says a window is unavailable    | Get a fresh ID from `gnoblinctl window list --json` or a `windows` snapshot. IDs expire when windows close.                                                                          |
| A Wayland client cannot bind a Gnoblin interface | Inspect the registry inside the Gnoblin session, then check its [protocol gate](wayland-protocols.md). Changes apply after config reload; existing client bindings remain connected. |

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

Gnoblin ships no `ananicy-cpp` rules. Give the `gnoblin` process and the programs it starts a rule of your own that does
not use `SCHED_IDLE`, in `/etc/ananicy.d/`, then run `sudo systemctl restart ananicy-cpp`. Log out and back in, then
check the policy again. To go back to the daemon's defaults, remove the rule.

## The session ended and returned to the login screen

If the compositor stops on its own, every Wayland app stops with it, because
apps cannot survive their compositor. GDM then shows the login screen. Gnoblin
does not restart the compositor in place.

Log in again, then read what the compositor logged before it stopped, and whether it left a core dump:

```sh
gnoblinctl log --level WARN --limit 100
coredumpctl list --no-pager | rg gnoblin
```

Keep that output when you report the problem.

## X11 apps closed but Wayland apps kept running

If Xwayland stops on its own, every X11 app closes. The compositor and the Wayland
apps keep running. Start the X11 app again. Xwayland starts again when the first X11
app connects.

Xwayland does not run at login. `pgrep -x Xwayland` prints nothing until you start an
X11 app. This is expected.

If the same X11 app closes each time, run it from a terminal and read its output. If
Xwayland itself keeps stopping, report it with the compositor log:

```sh
gnoblinctl log --limit 40
```

## An X11 app opens at 1 by 1 pixel or never shows

Some X11 toolkits, `xterm` among them, map a window at a placeholder size, then ask for the size they want and wait to be
told they got it. The compositor answers every such request:

- A floating window is granted the size it asks for, and the shell keeps the position it gave the window.
- A maximised, fullscreen or tiled window keeps the size the shell wrote, and the app is told that size.

If an app still stays tiny, the window manager refused its request and logged the reason. Find the line with the window
id:

```sh
gnoblinctl log --level WARN --limit 100
```

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

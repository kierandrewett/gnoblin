# Login managers

Gnoblin can run as the login screen. A login manager starts `gnoblin --greeter`
as its compositor, shows a greeter client on it, and then starts a normal
Gnoblin session when someone signs in. This page covers the greetd setup.

The greeter client is a separate program. Gnoblin does not ship one.

## greetd

Point greetd at Gnoblin in `/etc/greetd/config.toml`:

```toml
[terminal]
vt = 1

[default_session]
command = "gnoblin --greeter"
user = "greeter"
```

Use the greeter user your distribution creates for greetd. The `vt` value
must match the terminal greetd manages.

greetd starts the greeter through PAM with `XDG_SEAT=seat0`,
`XDG_SESSION_CLASS=greeter`, `XDG_VTNR`, `USER`, `HOME`, and `GREETD_SOCK`. It
does not set `XDG_SESSION_TYPE`. Gnoblin sets it to `wayland` itself when it
runs in greeter mode.

## Command line

```text
gnoblin --greeter [--config PATH] [--xwayland]
```

- `--greeter` starts a greeter-class session. It cannot be combined with
  devkit mode.
- `--config PATH` loads a Lua configuration from `PATH`. Login managers should
  pass their own file here.
- `--xwayland` starts Xwayland. Greeter mode leaves it off by default.

## Configuration

Greeter mode never reads `~/.config/gnoblin` or user drop-in files. It picks
the first file that exists:

1. The file given with `--config`.
2. `/etc/gnoblin/greeter.lua`.
3. `greeter.lua` in the installed Gnoblin share directory,
   `${prefix}/share/gnoblin/greeter.lua`.

The shipped `greeter.lua` starts no autostart entries. Put your greeter client
in the file your login manager passes with `--config`.

## What the greeter client receives

Autostart entries in the greeter configuration run with the guardian's
environment, so they receive:

- `WAYLAND_DISPLAY`, the compositor's socket.
- `GREETD_SOCK`, the socket greetd gave the greeter. The client uses it to
  talk to greetd.
- `GNOBLIN_SESSION_CLASS=greeter`.

The compositor itself also runs with `GNOBLIN_SESSION_CLASS=greeter`. In Lua,
`gnoblin.session.class()` returns `"greeter"`. User sessions return `"user"`.

## What greeter mode disables

Greeter mode is a login screen, so it does not run the user session services:

- It does not register a polkit authentication agent.
- It does not start keyring or GPG prompters.
- It does not handle logind `Lock` requests. The ScreenSaver `Lock` and
  `SetActive(true)` methods return an error, so the greeter never locks.
- It does not publish portal routes from the user's configuration.
- It does not start `gnoblin-session.target` and does not touch
  `graphical-session.target`.

It still exports `WAYLAND_DISPLAY` and the other display variables to the
systemd user manager and D-Bus activation, so greeter clients can start
services.

Diagnostics are written under the state directory of the greeter's home. If
that directory cannot be written, Gnoblin logs a warning and keeps running.

## Launch a user session

Gnoblin's login entry is `/usr/share/wayland-sessions/gnoblin.desktop`. A
greeter client should list that entry and start it when someone signs in.

When it starts a user session, the greeter must set `XDG_SESSION_TYPE=wayland`
in the environment. Gnoblin's user-session checks reject a session without it.
The user session must also come from a fresh logind login, not from a running
graphical session.

After the user logs out, the compositor exits and greetd restarts the greeter.

# Gnoblin

Gnoblin is a drop-in GNOME session for people who want GNOME's compositor,
window management and desktop services without GNOME's desktop chrome. It is
built from Mutter and GNOME Shell, with a session mode that removes the panel,
Activities overview, dash and notification banners.

You bring the chrome. Run Bingux, Waybar or another layer-shell client for the
bar, dock, launcher and notifications. Gnoblin provides the Wayland protocols
and the small `org.gnoblin.Shell` D-Bus API that a shell needs to integrate with
the session.

## What stays familiar

- GDM and the normal GNOME session remain available.
- Applications, workspaces, keyboard input, portals, polkit, keyring,
  automount and NetworkManager integration keep using GNOME's services.
- Mutter manages windows and workspaces. A layer-shell client owns the visible
  desktop controls.
- The `gnoblin` session is separate from the stock `user` session, so you can
  switch back at the login screen.

## Choose an install path

- **Fedora:** use the RPMs described in [distribution](docs/distribution.md).
  The [Gnoblin COPR project](https://copr.fedorainfracloud.org/coprs/kierandrewett/gnoblin/)
  is being prepared; do not treat the repository as ready until its builds and
  a clean login test pass.
- **NixOS:** use the flake and module in [Installation](docs/installation.md#nixos).
- **Development or another distribution:** build a private prefix with the
  [source instructions](docs/installation.md#get-the-source). This does not
  change system files until you explicitly register the session.

## Try it without changing your login

From a checkout with the normal Mutter and GNOME Shell build dependencies:

```sh
just init
just dev
just gnome-devkit
```

This opens a nested Gnoblin session and a terminal connected to it. Start your
layer shell from that terminal, for example:

```sh
qs -p /path/to/your/shell.qml
# or start another layer-shell client such as waybar
```

Close the terminal to end the nested session. For a real login, use the
[installation guide](docs/installation.md) and then the
[real-hardware checklist](docs/real-hardware-verification.md).

## Configure the session

The compositor reads `$GNOBLIN_CONFIG`, or
`$XDG_CONFIG_HOME/gnoblin/gnoblin.toml`, for protocol and window behaviour.
The `gnoblinctl` command controls live shell features:

```sh
gnoblinctl ping
gnoblinctl features
gnoblinctl disable osd
```

Use `disable` when your layer shell owns a surface such as notifications, OSD
or screenshots. Read the [configuration reference](docs/configuration.md) for
the complete list and migration details.

## Build and test

```sh
just test
just verify
just verify-release
```

`just test` is the quick deterministic gate. `just verify` builds the private
prefix and runs the isolated headless session checks. `just verify-release`
adds the real-host Mutter suite and RPM builds. See [Testing](docs/testing.md)
for the boundaries of each command.

## Documentation

- [Installation](docs/installation.md)
- [Bring your own shell](docs/bring-your-own-shell.md)
- [Distribution and COPR](docs/distribution.md)
- [Devkit](docs/devkit.md)
- [Configuration and `gnoblinctl`](docs/configuration.md)
- [Testing](docs/testing.md)
- [Real-hardware verification](docs/real-hardware-verification.md)
- [Source map](src/README.md)

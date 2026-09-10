# Installation

Choose your install method:

- [Fedora — RPM packages](#fedora)
- [NixOS — flake module](#nixos)
- [Arch, Debian and Ubuntu](#arch-debian-and-ubuntu)
- [Build from source — local test install](#build-from-source)

You'll also need a [desktop shell](bring-your-own-shell.md), such as Bingux.

## Fedora

The RPMs replace Fedora's Mutter and GNOME Shell packages. Both login sessions
use the patched builds. Use a Fedora installation with GNOME 49.

See [package availability](distribution.md#repository-status) for COPR status.
To build the RPMs yourself:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
sudo dnf install git just meson ninja-build rpmdevtools rpm-build
sudo dnf builddep packaging/rpm/mutter.spec packaging/rpm/gnome-shell.spec
just init
just rpm-all
```

Install the built packages:

```sh
just install-session dry     # preview the transaction
just install-session         # review and confirm installation
```

Log out, select **Gnoblin** at the login screen and start your chosen shell.
No manual session registration is needed.

### Go back to GNOME

Select **GNOME** at login. To restore Fedora's packages too, use DNF's
transaction history to undo the Gnoblin installation; review the proposed
changes before confirming. See [packaging details](../packaging/rpm/README.md).

## NixOS

Use Nixpkgs `nixos-25.11` with the GNOME 49 stack. Add Gnoblin to your flake:

```nix
inputs = {
  nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  gnoblin = {
    url = "github:kierandrewett/gnoblin";
    inputs.nixpkgs.follows = "nixpkgs";
  };
};
```

In your NixOS configuration, with `inputs` passed through `specialArgs`:

```nix
{ inputs, ... }: {
  imports = [ inputs.gnoblin.nixosModules.default ];
  programs.gnoblin.enable = true;
  services.displayManager.gdm.enable = true;
}
```

Keep your existing display manager if you already use another one. Rebuild
with `sudo nixos-rebuild switch`, then select **Gnoblin** at login.
Install your desktop shell separately.

To remove Gnoblin, remove the module import and enable option, then rebuild.
The module does not replace the system GNOME Shell package.

## Arch, Debian and Ubuntu

Packages aren't available yet. You can [build from source](#build-from-source),
but dependency lists for these distributions are not maintained. You'll need
the build dependencies for Mutter 49.5 and GNOME Shell 49.6, plus Git, Just,
Meson and Ninja.

## Build from source

This installs into `./install` for local testing. It does not replace system
packages. On Fedora, use the dependency commands in the [Fedora section](#fedora).

### Get the source

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
just init
```

### Build and try it

```sh
GNOBLIN_PREFIX="$PWD/install" just dev
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit
```

The devkit opens a nested session and a terminal. Start your shell from that
terminal. Close it to end the test. See [Devkit](devkit.md) for options.

### Install the session for real

After testing, register the local build:

```sh
GNOBLIN_PREFIX="$PWD/install" just dev-session-register
```

Run the `sudo install` command it prints, then select **Gnoblin** at login.
Keep the checkout and `install` directory in place while using this session.

To remove this local registration, log into another session and run:

```sh
rm ~/.config/systemd/user/org.gnoblin.Shell.target
rm ~/.config/systemd/user/org.gnoblin.Shell@wayland.service
systemctl --user daemon-reload
sudo rm /usr/share/wayland-sessions/gnoblin.desktop
```

You can then delete the checkout's `build` and `install` directories.

<a id="unattended-screensharing-xdg-desktop-portal-gnome"></a>

## Optional components

These require a source build and are not included in `just dev`:

- **Gnoblin Settings:** feature toggles and a reload button in GNOME Settings.
- **Persistent screen-sharing permissions:** remember approved Screen Cast
  and Remote Desktop access.

[Build optional components](source-development.md#optional-components)

[Development build options](source-development.md) · [Testing](testing.md) · [First-login checks](real-hardware-verification.md)

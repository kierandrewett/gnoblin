# Installation

**Gnoblin installs alongside GNOME.** It has its own runtime and login entry.
Installing or removing it does not replace your GNOME packages or session.

Choose your install method:

- [Fedora — RPM packages](#fedora)
- [NixOS — flake module](#nixos)
- [Arch, Debian and Ubuntu](#arch-debian-and-ubuntu)
- [Build from source — local test install](#build-from-source)

You'll also need a [desktop shell](bring-your-own-shell.md), such as Bingux.

## Fedora

The packages are named `gnoblin-mutter`, `gnoblin-shell` and `gnoblin-session`.
Their runtime lives in `/usr/lib/gnoblin`. Your existing GNOME packages remain
installed and selectable.

### Install from COPR — the official path

The installer uses the signed Gnoblin COPR for the Fedora release it is running
on. From a checkout, the complete system install is:

```sh
just install-session dry     # validate the host's COPR packages
just install-session          # enable COPR and install/update Gnoblin
```

No checkout, `just init`, source build, or session registration is required
for the official Fedora installation:

```sh
sudo dnf install dnf-plugins-core
sudo dnf copr enable kierandrewett/gnoblin
sudo dnf install --refresh gnoblin-mutter gnoblin-shell gnoblin-session
```

`gnoblin-session` pulls in the matching `gnoblin-shell` and `gnoblin-mutter`
packages. No source build, local RPM directory, or manual file copying is part
of the supported system-install path.

The repository also enables the Hyprcursor dependency repository used by the
Gnoblin Mutter build. Install your desktop shell separately, then log out and
select **Gnoblin** at the login screen.

### Build the RPMs yourself

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
sudo dnf install git just meson ninja-build rpmdevtools rpm-build
sudo dnf builddep packaging/rpm/mutter.spec
just init
just rpm mutter
sudo dnf install ~/rpmbuild/RPMS/*/gnoblin-mutter-51.0-*.rpm \
  ~/rpmbuild/RPMS/*/gnoblin-mutter-devel-51.0-*.rpm
sudo dnf builddep packaging/rpm/gnome-shell.spec
just rpm gnome-shell
```

Install the resulting shell and session RPMs with `sudo dnf install` before
logging out. Building RPMs alone does not install them. No manual session
registration is needed for packaged installations.

### Go back to GNOME

Select **GNOME** at login. To remove Gnoblin:

```sh
sudo dnf remove gnoblin-session gnoblin-shell gnoblin-mutter
```

No GNOME reinstall or downgrade is needed. Older experimental builds that
replaced Fedora packages need separate migration; this installer does not
automatically remove or downgrade them.

## NixOS

Use current Nixpkgs unstable with the GNOME 51 stack. Add Gnoblin to your flake:

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
The module exposes only Gnoblin's entry points; its GNOME Shell and Mutter
builds remain in private Nix store paths.

## Arch, Debian and Ubuntu

Gnoblin binary packages aren't available for these distributions yet. Use
the [source instructions](#build-from-source). Dependency installation is
automated for Arch/CachyOS. Other distributions can use `./build.sh --no-deps`
after installing compatible development dependencies.

## Build from source

This installs into `./install` for local testing. It does not replace system
packages. This is the source-development route; Fedora users who want to
install Gnoblin should use [COPR](#fedora).

### Get the source

Install Git using your distribution's package manager first.

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
```

### Build everything

```sh
./build.sh
```

This installs dependencies, initializes the sources, and builds Mutter, GNOME
Shell, Gnoblin Settings, the portal backend, and session files into `./install`.
It supports Fedora 43 and Arch/CachyOS. Arch dependency installation performs a
full package upgrade; Fedora enables the Gnoblin COPR for build dependencies.
Use `./build.sh --yes` for unattended package installation.

On other Linux distributions, install the development dependencies for Mutter,
GNOME Shell, GNOME Control Center and xdg-desktop-portal-gnome 51.0, then run
`./build.sh --no-deps`. That option skips package management;
it runs the same initialization and complete source build.

You need a C/C++ toolchain, Git, Bash, Just, Meson, Ninja, pkg-config, Python,
GLib development tools (including `glib-mkenums`), GObject Introspection,
Blueprint Compiler, SassC and Docutils (`rst2man`). Library requirements include
GLib 2.86+, GJS 1.85.90+, GTK 4, the Glycin 2 API, GNOME desktop libraries,
Evolution Data Server, Wayland protocols, PipeWire, libei, libdisplay-info,
Lua 5.4+ and Hyprcursor. Meson reports additional dependencies and required
versions for each component. Install headers and tools as well as runtime
libraries; package names vary by distribution. Older releases may require
newer dependencies. Only Fedora and Arch are currently tested in CI.

To try the resulting build:

```sh
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit
```

The devkit opens a nested session and a terminal. Start your shell from that
terminal. Close it to end the test. See [Devkit](devkit.md) for options.

### Install the session for real

After `./build.sh` succeeds and you have tested it, register the local
build. Registration does not build Gnoblin or create a missing runtime:

```sh
GNOBLIN_PREFIX="$PWD/install" just dev-session-register
```

Run the `sudo install` commands it prints, then select **Gnoblin** at login.
Keep the checkout and `install` directory in place while using this session.

To remove this local registration, log into another session and run:

```sh
rm ~/.config/systemd/user/org.gnoblin.Shell.target
rm ~/.config/systemd/user/org.gnoblin.Shell@wayland.service
rm ~/.config/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf
systemctl --user daemon-reload
sudo rm /usr/share/wayland-sessions/gnoblin.desktop
sudo rm /usr/share/gnome-session/sessions/gnoblin.session
```

You can then delete the checkout's `build` and `install` directories.

### Updating a source checkout

For an existing source checkout, run `git pull --ff-only`, then `./build.sh`.
If Git reports local changes, preserve them before updating; do not use forced resets to repair an installation.

The GitHub Actions installation workflow tests Fedora 43 package installation
and source compilation on Fedora 43 and Arch Linux. Graphical devkit use, GDM login, and session removal
still require a graphical host; a successful build alone does not verify them.

<a id="unattended-screensharing-xdg-desktop-portal-gnome"></a>

## Optional components

These are included in `./build.sh`:

- **Gnoblin Settings:** feature toggles and a reload button in GNOME Settings.
- **Persistent screen-sharing permissions:** remember approved Screen Cast
  and Remote Desktop access.

[Use these components](source-development.md#optional-components)

[Development build options](source-development.md) · [Testing](testing.md) · [First-login checks](real-hardware-verification.md)

# Source development

For installation, see [Installation](installation.md). This page covers
build options and optional components.

## Build options

The default prefix is `./install`, with libraries in `lib64`. Override both
when needed:

```sh
GNOBLIN_PREFIX=/tmp/gnoblin GNOBLIN_LIBDIR=lib just dev
```

`GNOBLIN_LIBDIR` is relative to the prefix. Pass the same prefix when running
other build or devkit commands. GNOME Shell gets a clean rebuild each time;
see `dev-gnome-shell` in the Justfile for the reason.

```sh
just verify-installed-headless   # check the existing local build
nix flake check                  # check the Nix flake
nix build .#gnoblin              # build the Nix package
```


## Optional components

Neither of these is part of `just dev` — build them explicitly once you need
them.

### Unattended screensharing (`xdg-desktop-portal-gnome`)

The optional portal backend can remember exact, portal-scoped Screen Cast and
Remote Desktop permissions for a verified requester. It needs one extra build
dependency:

```sh
sudo dnf install xdg-desktop-portal-devel
just dev-portal
```

Then run the patched backend so it owns the impl portal:

```sh
./install/libexec/xdg-desktop-portal-gnome -r
```

The [real-hardware verification guide](real-hardware-verification.md#7-persistent-screen-cast-and-remote-desktop-grants)
shows the first approval, exact-capability restore, storage, and revocation flow.

### Gnoblin Settings (forked `gnome-control-center`)

A `gnoblin` panel in GNOME Settings driving `org.gnoblin.Shell` (feature
toggles, Screen Cast and Remote Desktop grants, and a reload button):

```sh
sudo dnf install accountsservice-devel colord-gtk4-devel cups-devel gsound-devel \
  ibus-devel libgtop2-devel libnma-gtk4-devel malcontent-devel \
  ModemManager-glib-devel libpwquality-devel libsmbclient-devel libudisks2-devel
just dev-settings
./install/bin/gnome-control-center gnoblin
```

`dev-settings` also hides the Multitasking panel (no top bar/overview/dash
under gnoblin, so it doesn't apply) and handles the `blueprint-compiler`
build-side quirk automatically. Details in
[Real-hardware verification §8](real-hardware-verification.md#8-gnoblin-settings-forked-gnome-control-center).

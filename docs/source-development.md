# Source development

For installation, see [Installation](installation.md). This page covers
component rebuilds and build options. Run `./build.sh` first to install dependencies
and build everything.

## Build options

The default prefix is `./install`, with libraries in `lib64`. Override both
when needed:

```sh
GNOBLIN_PREFIX=/tmp/gnoblin GNOBLIN_LIBDIR=lib just build-local
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

Both components are built by `./build.sh`. The commands below rebuild them
individually during development.

### Unattended screensharing (`xdg-desktop-portal-gnome`)

The optional portal backend can remember exact, portal-scoped Screen Cast and
Remote Desktop permissions for a verified requester. To rebuild it:

```sh
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
just dev-settings
./install/bin/gnome-control-center gnoblin
```

`dev-settings` also hides the Multitasking panel (no top bar/overview/dash
under gnoblin, so it doesn't apply) and handles the `blueprint-compiler`
build-side quirk automatically. Details in
[Real-hardware verification §8](real-hardware-verification.md#8-gnoblin-settings-forked-gnome-control-center).

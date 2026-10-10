# Arch Linux packaging

The `gnoblin` package builds the standalone session and Mutter under
`/usr/lib/gnoblin`. The GTK-based portal backend is available separately as
`xdg-desktop-portal-gnoblin`; you can install a different portal backend instead. The
desktop schemas come from Arch and must meet the pinned source's minimum.

It never replaces, provides, or conflicts with Arch's `mutter` or
`gnome-shell` packages. The files outside that prefix are Gnoblin's login
entry, `gnoblinctl`, session systemd user units, and the portal route for
Gnoblin sessions. The optional portal package owns the backend's activation
files.

## Release source and integrity

Each release publishes these paired assets:

- `gnoblin-<version>-gnome-<gnome-version>-source.tar.xz`
- `gnoblin-<version>.PKGBUILD`
- `xdg-desktop-portal-gnoblin-<portal-version>.PKGBUILD`
- `xdg-desktop-portal-gnome-<gnome-version>.tar.xz`

The source archive includes the tracked Gnoblin tree, the materialised,
patch-applied source archives for Mutter and the portal backend, and the pinned
schema source used to generate keybinding metadata. It is also the general
source-build tarball: users can extract it and run `./build.sh` without Git or
submodules.

The core release PKGBUILD contains the source archive SHA-256. Download the
source archive and core recipe from the same release, place `PKGBUILD` beside
the archive, then run:

```bash
makepkg -si
```

To install Gnoblin's optional portal backend, download its PKGBUILD and the
matching portal source archive. Build and install them with `makepkg -si` in
that directory. The package needs GTK4 4.22 or newer and
`xdg-desktop-portal` 1.21.1 or newer. Otherwise, install and select another
portal backend.

The repository copy of `PKGBUILD` uses `SKIP` only as a generated development
template. Do not use it to install a release: use the checked release asset.

## Current status

The recipe is designed for a clean Arch build environment without stock GNOME
installed. It has no dependencies on unpublished `gnoblin-*` packages. The
distribution must provide the development-library and desktop-schema versions
required by the pinned GNOME sources; an older desktop-schema major version
blocks the package build.

Publishing an Arch repository or AUR package requires separate evidence that a
stock GNOME installation, a Gnoblin login, and Gnoblin removal all work on the
same target. Until those checks run for a release, this is a source packaging
path rather than a supported binary distribution channel.

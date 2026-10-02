# Arch Linux packaging

Gnoblin is one `gnoblin` package on Arch. It builds the standalone session,
Mutter, and portal backend together under `/usr/lib/gnoblin`. The desktop
schemas come from Arch and must meet the pinned source's minimum.

It never replaces, provides, or conflicts with Arch's `mutter` or
`gnome-shell` packages. The files outside that prefix are Gnoblin's login
entry, `gnoblinctl`, session systemd user units, and portal activation and
configuration files.

## Release source and integrity

Each release publishes these paired assets:

- `gnoblin-<version>-gnome-<gnome-version>-source.tar.xz`
- `gnoblin-<version>-gnome-<gnome-version>.PKGBUILD`

The source archive includes the tracked Gnoblin tree and the two materialised,
patch-applied component source archives for Mutter and the portal backend. It
is also the general source-build tarball: users can extract it and run
`./build.sh` without Git or submodules.

The release PKGBUILD contains the source archive SHA-256. Download the two
assets from the same release, place `PKGBUILD` beside the archive, then run:

```bash
makepkg -si
```

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

# Arch Linux packaging

Gnoblin is one `gnoblin` package on Arch. It builds the patched GNOME Shell,
Mutter, schemas, and private dependencies together under `/usr/lib/gnoblin`.
It never replaces, provides, or conflicts with Arch's `mutter` or
`gnome-shell` packages. The only files outside that prefix are Gnoblin's login
entry, `gnoblinctl`, and its `org.gnoblin.*` systemd user units.

## Release source and integrity

Each release publishes these paired assets:

- `gnoblin-<version>-gnome-<gnome-version>-arch-source.tar.xz`
- `gnoblin-<version>-gnome-<gnome-version>.PKGBUILD`

The source archive includes the tracked Gnoblin tree and the three
materialised, patch-applied component source archives for GSettings desktop
schemas, Mutter, and GNOME Shell. It does not depend on Git submodules being
present on the machine running `makepkg`.

The release PKGBUILD contains the source archive SHA-256. Download the two
assets from the same release, place `PKGBUILD` beside the archive, then run:

```bash
makepkg -si
```

The repository copy of `PKGBUILD` uses `SKIP` only as a generated development
template. Do not use it to install a release: use the checked release asset.

## Current status

The recipe is designed for a clean Arch build environment with stock GNOME
installed. It has no dependencies on unpublished `gnoblin-*` packages.
Publishing an Arch repository or AUR package requires separate evidence that a
stock GNOME installation, a Gnoblin login, and Gnoblin removal all work on the
same target. Until those checks run for a release, this is a source packaging
path rather than a supported binary distribution channel.

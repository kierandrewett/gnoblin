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

## Build dependency installation errors

`makepkg -si` installs the recipe's build dependencies with pacman before it
starts compiling. If pacman reports a conflicting file such as
`/usr/share/inkscape/palettes/elementary.gpl`, that is an existing file on the
machine which the package database does not allow the `inkscape` package to
install over. Gnoblin has not started building yet.

Check which package, if any, owns the path:

```bash
pacman -Qo /usr/share/inkscape/palettes/elementary.gpl
```

If it is owned by another package, inspect that package before changing
anything. If pacman says no package owns it, preserve or remove the stale file
only after confirming it is safe to do so, then install the missing dependency
and rerun `makepkg -si`. Do not use pacman's `--overwrite '*'`; that can replace
unrelated system files.

## Current status

The recipe is designed for a clean Arch build environment with stock GNOME
installed. It has no dependencies on unpublished `gnoblin-*` packages.
Publishing an Arch repository or AUR package requires separate evidence that a
stock GNOME installation, a Gnoblin login, and Gnoblin removal all work on the
same target. Until those checks run for a release, this is a source packaging
path rather than a supported binary distribution channel.

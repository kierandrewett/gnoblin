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

## Pacman repository

The release workflow (`.github/workflows/arch-package.yml`) builds both
packages, then runs `repo-add` to create `gnoblin.db`, a regular file that
lists each package with its SHA-256. The release publishes `gnoblin.db` beside
the `.pkg.tar.zst` files, so
`Server = https://github.com/kierandrewett/gnoblin/releases/latest/download`
works as a pacman server. Ananicy releases use `--latest=false`, so they do not
take over `latest`. The runtime job installs both packages through a `file://`
server that uses this database. The packages are not signed with a pacman key,
so users set `SigLevel = Optional TrustAll`.

`gnoblin.db` is part of the release asset set. `scripts/publish-github-release.sh`
refuses to publish a release without it. Releases up to 0.1.12 predate it.

An AUR package is not published. The released `gnoblin-<version>.PKGBUILD`
already names the release tag's source archive and its real SHA-256, so it
works as an AUR recipe. Publishing it needs an AUR account and SSH key.

## Current status

The recipe is designed for a clean Arch build environment without stock GNOME
installed. It has no dependencies on unpublished `gnoblin-*` packages. The
distribution must provide the development-library and desktop-schema versions
required by the pinned GNOME sources; an older desktop-schema major version
blocks the package build.

`packaging/targets.json` lists Arch as a candidate. The release workflow proves
package build, installation, coexistence with stock GNOME and removal. The
graphical-session gate stays open: Bingux reports that it boots this package in
QEMU, but that is not a Gnoblin CI run.

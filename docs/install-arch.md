# Arch Linux

Each [Gnoblin release](https://github.com/kierandrewett/gnoblin/releases)
publishes prebuilt Arch packages for x86_64. Releases after 0.1.12 also publish
a pacman repository database.

The release workflow builds the packages from the release's source bundle in a
clean Arch container. Before it publishes, it installs them without stock GNOME,
checks them beside stock GNOME, and removes them. See
[platform support](platform-support.md) before using the package as a login
session.

## Install from the release repository

Add the repository to `/etc/pacman.conf`:

```ini
[gnoblin]
SigLevel = Optional TrustAll
Server = https://github.com/kierandrewett/gnoblin/releases/latest/download
```

`SigLevel = Optional TrustAll` is required because the packages are not signed
with a pacman key. pacman checks each package against the SHA-256 in the
repository database, and each release attests its assets with GitHub build
provenance.

To pin a release, replace `latest/download` with `download/gnoblin-vVERSION`,
using the release tag you want. Release 0.1.12 has no database. For it,
download the package from the release and run `pacman -U`.

Then install:

```sh
sudo pacman -Syu gnoblin
```

Install `xdg-desktop-portal-gnoblin` as well if you want Gnoblin's GTK portal
backend. See the note on portals below.

## Build from a release PKGBUILD

Use this path to rebuild the package yourself, or when you cannot add the
repository. Download the matching source tarball and Arch PKGBUILD assets from
the release into `~/Downloads`. The PKGBUILD names the release source archive
and its SHA-256. Rename the PKGBUILD asset to `PKGBUILD`.

Check its `depends` list against the versions available in your configured
pacman repositories. The desktop schemas must meet the package's minimum
version; `makepkg` stops before compilation if they do not. When the required
versions are available, build and install as your normal user:

```sh
cd ~/Downloads
mv gnoblin-[0-9]*.PKGBUILD PKGBUILD
makepkg -si
```

The package installs Gnoblin under `/usr/lib/gnoblin` and registers its login
session. [Install a desktop shell](bring-your-own-shell.md), log out, and
select **Gnoblin** at the login screen. Keep another session available while
testing this package path.

The core package installs the default portal route for Gnoblin sessions but
does not include a backend. The release also provides the optional
`xdg-desktop-portal-gnoblin` package, its PKGBUILD and the matching source
archive. Install it with pacman from the repository, or build it with
`makepkg -si`. It needs GTK4 4.22 or newer and `xdg-desktop-portal` 1.21.1 or
newer.

Install the portal package if you want Gnoblin's GTK backend. The default route uses it when installed and otherwise selects
another installed backend. You can route interfaces to another backend in the
Lua config.

For GNOME applications on a minimal Arch system, install the
[shared desktop services](gnome-apps.md) first. Install the applications you
want through pacman.

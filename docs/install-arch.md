# Arch Linux

Gnoblin has no pacman repository yet. Use the source tarball and PKGBUILD
from a [Gnoblin release](https://github.com/kierandrewett/gnoblin/releases)
that targets your installed development libraries. See
[platform support](platform-support.md) before using the package as a login
session.

Download the matching source tarball and Arch PKGBUILD assets from the release
into `~/Downloads`. Rename the PKGBUILD asset to `PKGBUILD`.

Check its `depends` list against the versions available in your configured
pacman repositories. The desktop schemas must meet the package's minimum
version; `makepkg` stops before compilation if they do not. When the required
versions are available, build and install as your normal user:

```sh
cd ~/Downloads
mv gnoblin-*-gnome-*.PKGBUILD PKGBUILD
makepkg -si
```

The PKGBUILD installs Gnoblin under `/usr/lib/gnoblin` and registers its login
session. [Install a desktop shell](bring-your-own-shell.md), log out, and
select **Gnoblin** at the login screen. Keep another session available while
testing this package path.

For GNOME applications on a minimal Arch system, install the separate
[`gnoblin-gnome-integration` package](gnome-apps.md). Install the applications
you want through pacman.

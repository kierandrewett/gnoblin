# Package repositories

Gnoblin has package repositories for Fedora, openSUSE Tumbleweed, Debian testing, Arch Linux and Alpine edge. Add one
and your package manager installs and updates Gnoblin. The steps are in the guide for your system:
[Fedora](install-fedora.md), [openSUSE](install-opensuse.md), [Debian](install-debian.md),
[Arch Linux](install-arch.md) and [Alpine](install-alpine.md).

## Channels

Each repository has two channels.

| Channel   | What it carries                                 |
| --------- | ----------------------------------------------- |
| `stable`  | Released versions, such as 0.1.13               |
| `nightly` | The newest build of the main branch, once a day |

## Signing key

Every package and repository is signed. The key is at `https://repo.gnoblin.org/gpg.key`. Its fingerprint is:

```text
91BA A379 0941 B9B3 BBA6  3BC4 F1B8 D899 DF3D 786E
```

Check that the fingerprint your package manager shows matches this one before you accept the key.

## Switch channels

To move from `stable` to `nightly`, add the nightly repository and update. To move back, remove the nightly
repository and install the release again.

- **Fedora and openSUSE:** you follow the newest version of every repository you have enabled. With both channels
  enabled, you follow the nightly build.
- **Arch Linux:** pacman uses the first repository in `/etc/pacman.conf` that has the package. Put
  `[gnoblin-nightly]` above `[gnoblin]` to follow the nightly build, or below it to stay on releases.
- **Alpine:** you follow the newest version of every repository in `/etc/apk/repositories`.

To go back to the release on Fedora:

```sh
sudo rm /etc/yum.repos.d/gnoblin-nightly.repo
sudo dnf --refresh distro-sync gnoblin
```

On Arch Linux, delete the `[gnoblin-nightly]` section, then run `sudo pacman -Sy` and `sudo pacman -S gnoblin`.
On Alpine, delete the nightly line, run `apk update`, then install the release version, for example
`apk add gnoblin=0.1.13-r1`.

## Remove a repository

Removing a repository does not remove Gnoblin. Remove the packages as the guide for your system describes.

### Fedora

```sh
sudo rm /etc/yum.repos.d/gnoblin-stable.repo /etc/yum.repos.d/gnoblin-nightly.repo
```

### openSUSE Tumbleweed

```sh
sudo zypper removerepo gnoblin-stable
```

### Debian

```sh
sudo rm /etc/apt/sources.list.d/gnoblin-stable.sources /etc/apt/sources.list.d/gnoblin-nightly.sources /etc/apt/keyrings/gnoblin.asc
sudo apt update
```

### Arch Linux

Delete the `[gnoblin]` and `[gnoblin-nightly]` sections from `/etc/pacman.conf`, then run:

```sh
sudo pacman-key --delete 91BAA3790941B9B3BBA63BC4F1B8D899DF3D786E
sudo pacman -Sy
```

### Alpine

Delete the Gnoblin lines from `/etc/apk/repositories`, then remove the key:

```sh
rm /etc/apk/keys/gnoblin.rsa.pub
apk update
```

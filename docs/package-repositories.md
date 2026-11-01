# Package repositories

Gnoblin publishes signed package repositories. Add one and your package manager installs and updates Gnoblin.
The steps are in the install guide for your system. Start at [Install Gnoblin](installation.md).

## Channels

Each repository has two channels.

| Channel   | What it carries           |
| --------- | ------------------------- |
| `stable`  | Released versions         |
| `nightly` | Builds of the main branch |

## Signing key

Every package and repository is signed. The key is at `https://repo.gnoblin.org/gpg.key`. Your package manager
shows the key and asks you to accept it the first time you use the repository. Check that the fingerprint it
shows matches this one:

```text
91BA A379 0941 B9B3 BBA6  3BC4 F1B8 D899 DF3D 786E
```

If the fingerprints differ, do not accept the key.

## Switch channels

To move from `stable` to `nightly`, add the nightly repository and update. To move back, remove the nightly
repository and install the release again.

- **Fedora and openSUSE:** you get the highest version from any repository you have enabled. With both channels
  enabled, that is the nightly build.
- **Arch Linux:** pacman uses the first repository in `/etc/pacman.conf` that has the package. Put
  `[gnoblin-nightly]` above `[gnoblin]` to follow the nightly build, or below it to stay on releases.
- **Alpine:** you get the highest version from any repository in `/etc/apk/repositories`.

To go back to the release on Fedora:

```sh
sudo rm /etc/yum.repos.d/gnoblin-nightly.repo
sudo dnf --refresh distro-sync gnoblin
```

On Arch Linux, delete the `[gnoblin-nightly]` section, then run `sudo pacman -Sy` and `sudo pacman -S gnoblin`.
On Alpine, delete the nightly line, run `apk update`, then install the release version, for example
`apk add gnoblin=VERSION`. Run `apk policy gnoblin` to see the versions.

## Remove a repository

Removing a repository does not remove Gnoblin. Remove the packages as the guide for your system describes.

### Fedora

```sh
sudo rm /etc/yum.repos.d/gnoblin-stable.repo /etc/yum.repos.d/gnoblin-nightly.repo
```

### openSUSE

```sh
sudo zypper removerepo gnoblin-stable
```

### Debian

```sh
sudo rm /etc/apt/sources.list.d/gnoblin-stable.sources /etc/apt/sources.list.d/gnoblin-nightly.sources /etc/apt/keyrings/gnoblin.asc
sudo apt update
```

### Arch Linux

Delete the `[gnoblin]` and `[gnoblin-nightly]` sections from `/etc/pacman.conf`, then find the Gnoblin key with `pacman-key --list-keys` and run this with its fingerprint:

```sh
sudo pacman-key --delete FINGERPRINT
sudo pacman -Sy
```

### Alpine

Delete the Gnoblin lines from `/etc/apk/repositories`, then remove the key:

```sh
rm /etc/apk/keys/gnoblin.rsa.pub
apk update
```

# Package repositories

Gnoblin publishes signed package repositories for Fedora, openSUSE Tumbleweed
and Arch Linux. Add one repository and your package manager installs and updates
Gnoblin like any other package.

Each repository has two channels. Add the one you want.

| Channel   | What it carries                                 | Repository name   |
| --------- | ----------------------------------------------- | ----------------- |
| `stable`  | Released versions, such as 0.1.13               | `gnoblin-stable`  |
| `nightly` | The newest build of the main branch, once a day | `gnoblin-nightly` |

A nightly build is numbered between two releases. `0.1.13~nightly.20261031.142.gabc1234`
is newer than release 0.1.12 and older than release 0.1.13, so installing a
release always replaces the nightly build of the same version.

The packages and the repository metadata are signed. The signing key is
published at `https://repo.gnoblin.org/gpg.key`. Its fingerprint is:

```text
REPLACE-WITH-KEY-FINGERPRINT
```

Compare the fingerprint your package manager shows with this one before you
accept the key.

Debian and Ubuntu have no repository because there is no Gnoblin APT package.
Build from [source](install-source.md) there.

## Fedora

Add the repository file of the channel you want, then install:

```sh
sudo curl -fsSL -o /etc/yum.repos.d/gnoblin-stable.repo \
  https://repo.gnoblin.org/gnoblin/stable/fedora/gnoblin-stable.repo
sudo dnf --setopt=install_weak_deps=False install gnoblin
```

For the nightly channel, use `nightly` in both places of the first command.
`dnf` shows the key and asks you to accept it the first time it reads the
repository. The repository file turns on `gpgcheck` (package signatures) and
`repo_gpgcheck` (metadata signatures).

Update with `sudo dnf upgrade --refresh gnoblin`.

The packages are built for Fedora 45. Other releases have no directory in the
repository.

## openSUSE Tumbleweed

The stable channel has Tumbleweed packages. The nightly channel does not.

```sh
sudo zypper addrepo --refresh \
  https://repo.gnoblin.org/gnoblin/stable/opensuse/tumbleweed/gnoblin-stable.repo
sudo zypper refresh
sudo zypper install gnoblin
```

`zypper` shows the key and asks you to trust it. Compare the fingerprint first.

## Arch Linux

Trust the key, add the repository to `/etc/pacman.conf`, then install:

```sh
curl -fsSL -o gnoblin.key https://repo.gnoblin.org/gpg.key
sudo pacman-key --add gnoblin.key
sudo pacman-key --lsign-key REPLACE-WITH-KEY-FINGERPRINT
printf '\n' | sudo tee -a /etc/pacman.conf > /dev/null
curl -fsSL https://repo.gnoblin.org/gnoblin/stable/arch/gnoblin.conf | sudo tee -a /etc/pacman.conf
sudo pacman -Syu gnoblin
```

The `gnoblin.conf` file holds this section, which requires a valid signature on
every package and on the database:

```ini
[gnoblin]
SigLevel = Required DatabaseRequired
Server = https://repo.gnoblin.org/gnoblin/stable/arch/$arch
```

For the nightly channel, download `arch/gnoblin-nightly.conf` from the nightly
directory instead. Its section is named `[gnoblin-nightly]` because pacman
names the database file after the section.

## Switch channels

You can add both channels, but choose which one wins:

- Fedora and openSUSE use the newest version of any enabled repository. With
  both channels enabled you follow the nightly build.
- pacman uses the first repository in `/etc/pacman.conf` that has the package,
  not the newest. List `[gnoblin-nightly]` above `[gnoblin]` to follow the
  nightly build, or below it to stay on releases.

To move from stable to nightly, add the nightly repository and update. To move
back, remove the nightly repository and downgrade to the release:

```sh
sudo rm /etc/yum.repos.d/gnoblin-nightly.repo
sudo dnf --refresh distro-sync gnoblin
```

On Arch, delete the `[gnoblin-nightly]` section, run `sudo pacman -Sy` and then
`sudo pacman -S gnoblin`. pacman downgrades the package to the release of the
repository you kept, but only when you ask for the package by name.

## Remove a repository

Remove the repository, then the key. Removing a repository does not remove
Gnoblin. Remove the packages as described on your system's
[install page](installation.md).

### Fedora

```sh
sudo rm /etc/yum.repos.d/gnoblin-stable.repo /etc/yum.repos.d/gnoblin-nightly.repo
rpm -qa 'gpg-pubkey*' --queryformat '%{NAME}  %{SUMMARY}\n'
```

The second command lists the imported keys. Remove the one whose fingerprint
matches the one above with `sudo rpm -e NAME`, using the name from the list.

### openSUSE Tumbleweed

```sh
sudo zypper removerepo gnoblin-stable
rpm -qa 'gpg-pubkey*' --queryformat '%{NAME}  %{SUMMARY}\n'
```

Remove the key as on Fedora.

### Arch Linux

Delete the `[gnoblin]` and `[gnoblin-nightly]` sections from `/etc/pacman.conf`,
then run:

```sh
sudo pacman-key --delete REPLACE-WITH-KEY-FINGERPRINT
sudo pacman -Sy
```

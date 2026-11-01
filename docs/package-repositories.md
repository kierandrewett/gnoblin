# Package repositories

Gnoblin publishes signed package repositories for Fedora, openSUSE Tumbleweed,
Debian testing, Arch Linux and Alpine edge, for amd64 and arm64. Add one repository and your package manager installs and updates
Gnoblin like any other package.

Each repository has two channels. Add the one you want.

| Channel   | What it carries                                 | Repository name   |
| --------- | ----------------------------------------------- | ----------------- |
| `stable`  | Released versions, such as 0.1.13               | `gnoblin-stable`  |
| `nightly` | The newest build of the main branch, once a day | `gnoblin-nightly` |

A nightly build is numbered between two releases. `0.1.13~nightly.20261031.142.gabc1234`
is newer than release 0.1.12 and older than release 0.1.13, so installing a
release always replaces the nightly build of the same version. On Alpine the same
build is `0.1.13_pre20261031142`.

The packages and the repository metadata are signed. The signing key of every
system except Alpine is published at `https://repo.gnoblin.org/gpg.key`. Its
fingerprint is:

```text
91BA A379 0941 B9B3 BBA6  3BC4 F1B8 D899 DF3D 786E
```

Compare the fingerprint your package manager shows with this one before you
accept the key.

Debian 13 and Ubuntu are not in the repositories, because their libraries are too old for the
packages. Build from [source](install-source.md) there.

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

Both channels have Tumbleweed packages.

```sh
sudo zypper addrepo --refresh \
  https://repo.gnoblin.org/gnoblin/stable/opensuse/tumbleweed/gnoblin-stable.repo
sudo zypper refresh
sudo zypper install gnoblin
```

`zypper` shows the key and asks you to trust it. Compare the fingerprint first.

## Debian testing

Install the key and the sources file of the channel you want, then install:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/gnoblin.asc https://repo.gnoblin.org/gpg.key
sudo curl -fsSL -o /etc/apt/sources.list.d/gnoblin-stable.sources \
  https://repo.gnoblin.org/gnoblin/stable/deb/gnoblin-stable-forky.sources
sudo apt update
sudo apt install gnoblin
```

For the nightly channel, use `nightly` in the file names and the address. Check the key
fingerprint with `gpg --show-keys /etc/apt/keyrings/gnoblin.asc` before you run `apt update`.
See [Debian](install-debian.md).

## Arch Linux

Trust the key, add the repository to `/etc/pacman.conf`, then install:

```sh
curl -fsSL -o gnoblin.key https://repo.gnoblin.org/gpg.key
sudo pacman-key --add gnoblin.key
sudo pacman-key --lsign-key 91BAA3790941B9B3BBA63BC4F1B8D899DF3D786E
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

## Alpine edge

Alpine does not use GnuPG. `apk` checks the repository index against an RSA
public key that you put in `/etc/apk/keys`, so the key file must have the name
`gnoblin.rsa.pub`. `apk` has no prompt that shows a fingerprint, so download
the key over HTTPS only. Run these commands as root, or with `doas`:

```sh
wget -O /etc/apk/keys/gnoblin.rsa.pub https://repo.gnoblin.org/gnoblin.rsa.pub
echo https://repo.gnoblin.org/gnoblin/stable/alpine/edge >> /etc/apk/repositories
apk update
apk add gnoblin
```

For the nightly channel, use `nightly` in the address. `apk update` prints
`Gnoblin stable` or `Gnoblin nightly` for the repository it read. Update with
`apk upgrade`.

The packages are built for Alpine edge and for `x86_64` and `aarch64`. A release
branch of Alpine has no directory in the repository, because its libraries are
older than Gnoblin needs.

## Switch channels

You can add both channels, but choose which one wins:

- Fedora and openSUSE use the newest version of any enabled repository. With
  both channels enabled you follow the nightly build.
- `apk` installs the newest version of any repository in `/etc/apk/repositories`. With both
  channels listed you follow the nightly build.
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

On Alpine, delete the nightly line from `/etc/apk/repositories`, run `apk update`
and ask for the release by version, for example `apk add gnoblin=0.1.13-r1`.
`apk upgrade` never downgrades.

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

### Debian

```sh
sudo rm /etc/apt/sources.list.d/gnoblin-stable.sources /etc/apt/sources.list.d/gnoblin-nightly.sources /etc/apt/keyrings/gnoblin.asc
sudo apt update
```

### Alpine

Delete the Gnoblin lines from `/etc/apk/repositories`, then remove the key:

```sh
rm /etc/apk/keys/gnoblin.rsa.pub
apk update
```

### Arch Linux

Delete the `[gnoblin]` and `[gnoblin-nightly]` sections from `/etc/pacman.conf`,
then run:

```sh
sudo pacman-key --delete 91BAA3790941B9B3BBA63BC4F1B8D899DF3D786E
sudo pacman -Sy
```

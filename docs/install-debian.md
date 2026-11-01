# Debian

Install Gnoblin on Debian from the Gnoblin package repository. See [platform support](platform-support.md) for the supported releases and architectures. Where there is no package, use
[Build from source](install-source.md).

## Install

Add the Gnoblin repository, then install Gnoblin:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/gnoblin.asc https://repo.gnoblin.org/gpg.key
sudo curl -fsSL -o /etc/apt/sources.list.d/gnoblin-stable.sources \
  "https://repo.gnoblin.org/gnoblin/stable/deb/gnoblin-stable-$(. /etc/os-release && echo "$VERSION_CODENAME").sources"
sudo apt update
sudo apt install gnoblin
```

Before you run `apt update`, check the key fingerprint with `gpg --show-keys /etc/apt/keyrings/gnoblin.asc`.
It must match the one in [package repositories](package-repositories.md#signing-key).

To follow the main branch instead of releases, use `nightly` in place of `stable` in the file name and the address.

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

```sh
sudo apt update
sudo apt install --only-upgrade gnoblin
```

Log out and back in to use the new version.

## Remove

Log in to another session first, then run:

```sh
sudo apt purge gnoblin
```

Your shell and your configuration stay. Remove the shell separately if you do not want it.

See [troubleshooting](troubleshooting.md) if the session does not start.

## More information

- [Debian Wiki: using third-party repositories](https://wiki.debian.org/DebianRepository/UseThirdParty)

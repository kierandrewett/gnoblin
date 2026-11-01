# Debian

Gnoblin is built for Debian testing (`forky`), on amd64 and arm64. Debian 13 and Ubuntu are not supported.
Use [Build from source](install-source.md) there.

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

To follow the newest build instead of releases, use `nightly` in place of `stable` in the file name and the address.

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

Your shell and your configuration stay. Remove the shell separately if you no longer want it.

## If installation fails

- **404 when you download the sources file:** your release is not Debian testing. Gnoblin has no packages for it.
- **Unmet dependencies:** run `sudo apt update` and install again. If it still fails, report the full error.

See [troubleshooting](troubleshooting.md) if the session does not start.

# Fedora

Gnoblin is built for Fedora 45, on amd64 and arm64.

## Install

Add the Gnoblin repository, then install Gnoblin:

```sh
sudo dnf config-manager addrepo --from-repofile=https://repo.gnoblin.org/gnoblin/stable/fedora/gnoblin-stable.repo
sudo dnf install gnoblin
```

`dnf` shows the signing key the first time. Check that its fingerprint matches the one in
[package repositories](package-repositories.md#signing-key), then accept it.

To follow the newest build instead of releases, use `nightly` in place of `stable` and `gnoblin-nightly.repo`
in the first command.

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

```sh
sudo dnf upgrade --refresh gnoblin
```

Log out and back in to use the new version.

## Remove

Log in to GNOME or another session first, then run:

```sh
sudo dnf remove gnoblin
```

Your shell and your configuration stay. Remove the shell separately if you no longer want it.

## If installation fails

- **Package not found:** Gnoblin has packages for Fedora 45 only. Check your release with `cat /etc/fedora-release`.
- **Missing dependency:** run `sudo dnf upgrade --refresh`, then install again. If it still fails, report the full error.

See [troubleshooting](troubleshooting.md) if the session does not start.

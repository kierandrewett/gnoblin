# Fedora

Install Gnoblin on Fedora from the Gnoblin package repository. See [platform support](platform-support.md) for the supported releases and architectures.

## Install

Add the Gnoblin repository, then install Gnoblin:

```sh
sudo dnf config-manager addrepo --from-repofile=https://repo.gnoblin.org/gnoblin/stable/fedora/gnoblin-stable.repo
sudo dnf install gnoblin
```

`dnf` shows the signing key the first time. Check that its fingerprint matches the one in [package repositories](package-repositories.md#signing-key), then accept it.

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

See [troubleshooting](troubleshooting.md) if the session does not start.

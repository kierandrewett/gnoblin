# openSUSE

Install Gnoblin on openSUSE from the Gnoblin package repository. See [platform support](platform-support.md) for the supported releases and architectures.

## Install

Add the Gnoblin repository, then install Gnoblin:

```sh
sudo zypper addrepo --refresh https://repo.gnoblin.org/gnoblin/stable/opensuse/tumbleweed/gnoblin-stable.repo
sudo zypper refresh
sudo zypper install gnoblin
```

`zypper` shows the signing key and asks you to trust it. Check that its fingerprint matches the one in [package repositories](package-repositories.md#signing-key), then accept it.

To follow the main branch instead of releases, use `nightly` in place of `stable` and `gnoblin-nightly.repo`.

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

```sh
sudo zypper refresh
sudo zypper update gnoblin
```

Log out and back in to use the new version.

## Remove

Log in to another session first, then run:

```sh
sudo zypper remove gnoblin
```

Your shell and your configuration stay. Remove the shell separately if you do not want it.

See [troubleshooting](troubleshooting.md) if the session does not start.

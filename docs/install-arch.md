# Arch Linux

Gnoblin is built for Arch Linux and CachyOS, on x86_64 and aarch64.

## Install

Trust the Gnoblin signing key, add the repository to `/etc/pacman.conf`, then install Gnoblin:

```sh
curl -fsSL -o gnoblin.key https://repo.gnoblin.org/gpg.key
sudo pacman-key --add gnoblin.key
sudo pacman-key --lsign-key 91BAA3790941B9B3BBA63BC4F1B8D899DF3D786E
curl -fsSL https://repo.gnoblin.org/gnoblin/stable/arch/gnoblin.conf | sudo tee -a /etc/pacman.conf
sudo pacman -Syu gnoblin
```

The key fingerprint in the third command is the one in [package repositories](package-repositories.md#signing-key).

To follow the newest build instead of releases, use `nightly` in the address and `gnoblin-nightly.conf` as the file name.

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

```sh
sudo pacman -Syu
```

Log out and back in to use the new version.

## Remove

Log in to another session first, then run:

```sh
sudo pacman -Rns gnoblin
```

Your shell and your configuration stay. Remove the shell separately if you no longer want it.

See [troubleshooting](troubleshooting.md) if the session does not start.

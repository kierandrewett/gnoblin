# Arch Linux

Install Gnoblin on Arch Linux or CachyOS from the Gnoblin package repository. See [platform support](platform-support.md) for the supported releases and architectures.

## Install

1. Download the signing key and check its fingerprint. It must match the one in
   [package repositories](package-repositories.md#signing-key):

    ```sh
    curl -fsSL -o gnoblin.key https://repo.gnoblin.org/gpg.key
    gpg --show-keys gnoblin.key
    ```

2. Trust the key:

    ```sh
    sudo pacman-key --add gnoblin.key
    sudo pacman-key --lsign-key "$(gpg --show-keys --with-colons gnoblin.key | awk -F: '/^fpr/ {print $10; exit}')"
    ```

3. Add the repository to `/etc/pacman.conf`, then install Gnoblin:

    ```sh
    curl -fsSL https://repo.gnoblin.org/gnoblin/stable/arch/gnoblin.conf | sudo tee -a /etc/pacman.conf
    sudo pacman -Syu gnoblin
    ```

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

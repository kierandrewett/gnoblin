# Alpine

Install Gnoblin on Alpine from the Gnoblin package repository. See [platform support](platform-support.md) for the supported releases and architectures.

## Install

Add the Gnoblin signing key and repository, then install Gnoblin. Run these commands as root, or with `doas`:

```sh
wget -O /etc/apk/keys/gnoblin.rsa.pub https://repo.gnoblin.org/gnoblin.rsa.pub
echo https://repo.gnoblin.org/gnoblin/stable/alpine/edge >> /etc/apk/repositories
apk update
apk add gnoblin
```

To follow the main branch instead of releases, use `nightly` in place of `stable` in the address.

## Next steps

[Install a desktop shell and log in](installation.md#after-installing).

## Update

```sh
apk update
apk upgrade gnoblin
```

Log out and back in to use the new version.

## Remove

Log in to another session first, then run:

```sh
apk del gnoblin
```

Your shell and your configuration stay. Remove the shell separately if you do not want it.

See [troubleshooting](troubleshooting.md) if the session does not start.

## More information

- [Alpine Wiki: Alpine Package Keeper](https://wiki.alpinelinux.org/wiki/Alpine_Package_Keeper)

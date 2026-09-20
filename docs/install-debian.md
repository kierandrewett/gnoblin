# Debian and Ubuntu

Official packages for Intel and AMD 64-bit PCs.

## 1. Download your package

| System           | Download                                                                                                         |
| ---------------- | ---------------------------------------------------------------------------------------------------------------- |
| Debian 13        | [Download .deb](https://github.com/kierandrewett/gnoblin/releases/latest/download/gnoblin-debian13-amd64.deb)    |
| Ubuntu 24.04 LTS | [Download .deb](https://github.com/kierandrewett/gnoblin/releases/latest/download/gnoblin-ubuntu24.04-amd64.deb) |
| Ubuntu 26.04 LTS | [Download .deb](https://github.com/kierandrewett/gnoblin/releases/latest/download/gnoblin-ubuntu26.04-amd64.deb) |

## 2. Install

Open a terminal in the folder containing the download. Refresh APT's package
list, then install the file for your system.

Debian 13:

```sh
sudo apt update
sudo apt install ./gnoblin-debian13-amd64.deb
```

Ubuntu 24.04 LTS:

```sh
sudo apt update
sudo apt install ./gnoblin-ubuntu24.04-amd64.deb
```

Ubuntu 26.04 LTS:

```sh
sudo apt update
sudo apt install ./gnoblin-ubuntu26.04-amd64.deb
```

APT installs the required system packages. Gnoblin's newer runtime libraries
stay under `/usr/lib/gnoblin`; your GNOME session remains installed separately.

## 3. Choose a shell and log in

[Install a desktop shell](bring-your-own-shell.md) for your bar, launcher and
other desktop controls. Then log out, select **Gnoblin** in the login screen's
session menu, and log in.

Next: [configure Gnoblin](configuration.md).

## Update

Download the new package for your system and run the same `apt install` command.
Log out and back in to use the updated compositor.

These downloads do not add an APT repository, so normal system updates do not
fetch new Gnoblin releases automatically.

## Remove

Log into another session, then:

```sh
sudo apt remove gnoblin
```

Your configuration in `~/.config/gnoblin` is kept.

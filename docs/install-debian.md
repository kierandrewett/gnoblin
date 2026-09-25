# Debian and Ubuntu

Debian 13, Ubuntu 24.04 LTS and Ubuntu 26.04 LTS have APT package candidates
for Intel and AMD 64-bit PCs. They pass package build, clean installation,
stock GNOME coexistence and removal checks, but do not yet have a verified
graphical Gnoblin login. Keep GNOME or another session available. See
[platform support](platform-support.md) for the release status.

## 1. Add the archive key

Open a terminal and run:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://gnoblin.org/apt/gnoblin-archive-keyring.asc |
    sudo gpg --dearmor --yes -o /etc/apt/keyrings/gnoblin-archive-keyring.gpg
```

This key verifies that packages and updates came from Gnoblin. If `curl` or
`gpg` is missing, install the `curl` and `gpg` packages first.

## 2. Add your system's repository

Debian 13:

```sh
echo 'deb [arch=amd64 signed-by=/etc/apt/keyrings/gnoblin-archive-keyring.gpg] https://gnoblin.org/apt/debian 13 main' |
    sudo tee /etc/apt/sources.list.d/gnoblin.list
```

Ubuntu 24.04 LTS:

```sh
echo 'deb [arch=amd64 signed-by=/etc/apt/keyrings/gnoblin-archive-keyring.gpg] https://gnoblin.org/apt/ubuntu 24.04 main' |
    sudo tee /etc/apt/sources.list.d/gnoblin.list
```

Ubuntu 26.04 LTS:

```sh
echo 'deb [arch=amd64 signed-by=/etc/apt/keyrings/gnoblin-archive-keyring.gpg] https://gnoblin.org/apt/ubuntu 26.04 main' |
    sudo tee /etc/apt/sources.list.d/gnoblin.list
```

## 3. Install

```sh
sudo apt update
sudo apt install gnoblin
```

APT installs the required system packages. Gnoblin's newer runtime libraries
stay under `/usr/lib/gnoblin`; your GNOME session remains installed separately.

## 4. Choose a shell and test the session

[Install a desktop shell](bring-your-own-shell.md) for your bar, launcher and
other desktop controls. Then log out, select **Gnoblin** in the login screen's
session menu, and test whether it reaches a usable desktop. Return to GNOME if
the session does not start.

Next: [configure Gnoblin](/config).

## Update

Run your normal system update, then log out and back in to use a new compositor:

```sh
sudo apt update
sudo apt upgrade
```

Direct `.deb` downloads remain available on the [release page](https://github.com/kierandrewett/gnoblin/releases/latest).

## Remove

Log into another session, then:

```sh
sudo apt remove gnoblin
```

Your configuration in `~/.config/gnoblin` is kept.

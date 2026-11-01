# Debian

Gnoblin publishes a `.deb` package for Debian testing (`forky`). The package needs glibc 2.43,
GLib 2.86, GTK 4.20 and the other library versions of the pinned GNOME 51 sources.

Debian 13 (`trixie`) and the Ubuntu releases have older libraries, so the package does not
install there. Use [Build from source](install-source.md) on those systems if their libraries meet the
requirements.

Keep stock GNOME or another session available while you test Gnoblin. See
[platform support](platform-support.md).

## 1. Install Gnoblin

Add Gnoblin's signed repository, then install the package. The `stable` channel carries releases.
Use `nightly` in the file names for the newest build of the main branch. See
[package repositories](package-repositories.md) for the key fingerprint to check.

```sh
sudo install -d -m 0755 /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/gnoblin.asc https://repo.gnoblin.org/gpg.key
sudo curl -fsSL -o /etc/apt/sources.list.d/gnoblin-stable.sources \
  https://repo.gnoblin.org/gnoblin/stable/deb/gnoblin-stable-forky.sources
sudo apt update
sudo apt install gnoblin
```

To install a release file instead, download the `.deb` from the
[Gnoblin release](https://github.com/kierandrewett/gnoblin/releases) and run
`sudo apt install ./gnoblin_*_amd64.deb`. Use `arm64` in the name on an Arm system.

`gnoblin` installs the Lua-supervised session, its private Mutter runtime and the portal route
for Gnoblin sessions under `/usr/lib/gnoblin`. It does not install GNOME Shell or a portal backend.

## 2. Add the optional packages

| Package                       | What it adds                                               |
| ----------------------------- | ---------------------------------------------------------- |
| `xdg-desktop-portal-gnoblin`  | Gnoblin's GTK-based portal backend                         |
| `gnoblin-geoclue-integration` | Lets Gnoblin authorize location requests through GeoClue   |
| `gnoblin-ananicy-rules`       | Normal scheduling for Gnoblin when `ananicy-cpp` is in use |

```sh
sudo apt install xdg-desktop-portal-gnoblin
```

The default portal route uses the Gnoblin backend when it is installed and another installed
backend otherwise. For GNOME applications on a minimal system, install the
[shared desktop services](gnome-apps.md) first.

## 3. Install a shell and log in

[Choose a desktop shell](bring-your-own-shell.md), log out, select your user in the login screen,
then choose **Gnoblin** in the session menu. Return to GNOME if the session does not start.
Continue with [configuration](/config).

## Update and remove

```sh
sudo apt update
sudo apt install --only-upgrade gnoblin
```

Log out and back in to load the updated compositor. To remove Gnoblin, log into another session
first:

```sh
sudo apt purge gnoblin xdg-desktop-portal-gnoblin
```

Your shell and your personal configuration are separate and stay.

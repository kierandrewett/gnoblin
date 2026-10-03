# Fedora

The COPR build targets Fedora 45. Check the latest release and COPR results
before installing: the GNOME library requirements changed with the pinned
upstream source. Keep GNOME or another session available.
See [platform support](platform-support.md) for the current status.

## 1. Install Gnoblin

```sh
sudo dnf install dnf-plugins-core
sudo dnf copr enable kierandrewett/gnoblin
sudo dnf --setopt=install_weak_deps=False install --refresh gnoblin
```

`gnoblin` installs the Lua-supervised session and its private Mutter runtime.
It does not install GNOME Shell, GJS, or a portal backend. Install
`gnoblin-portal` if you want Gnoblin's GTK-based backend; otherwise install and
select any portal backend you prefer. To add Gnoblin's backend, run
`sudo dnf install gnoblin-portal`. This command skips packages recommended by
dependencies. Install any optional services you need using the commands below.

Gnoblin uses the portal backend selected for its session. If GNOME is also
installed, that session keeps using its own portal configuration and backend.
Gnoblin does not install `xdg-desktop-portal-gtk` or require
`gnome-desktop4`. Package dependencies may still bring GTK3 onto a system;
inspect the transaction before installing if that matters to you.

For GNOME apps on a minimal Fedora install, add the optional
[`gnoblin-gnome-integration` package](gnome-apps.md). It supplies common
desktop services; install the apps you want separately.

If your Thunderbolt devices need authorization, install `bolt` separately with
`sudo dnf install bolt`. Gnoblin does not provide GNOME's Thunderbolt menu.
For a shell that needs UPower battery data, install it with
`sudo dnf install upower`. Gnoblin's lean login does not use GNOME's power menu.

To let Gnoblin handle GeoClue location authorization, install the optional
`gnoblin-geoclue-integration` package and restart GeoClue:

```sh
sudo dnf install gnoblin-geoclue-integration
sudo systemctl restart geoclue.service
```

The package keeps GeoClue's standard agent IDs and adds Gnoblin. If you have
custom GeoClue agent IDs, include them in the effective whitelist too.
Run `gnoblinctl privacy` after restarting GeoClue. The location source should
report `inactive` when no application is using it.

For IBus input methods, install `ibus` separately with `sudo dnf install ibus`.
The basic session needs only `ibus-libs` and does not start the daemon until
an IBus input source is configured.

## 2. Install a shell

[Choose a desktop shell](bring-your-own-shell.md) for your bar and launcher.
Use a complete shell or combine individual tools.

## 3. Test the session

Log out, select **Gnoblin** from the login screen's session selector, and test
whether it reaches a usable desktop. In GDM, select your user first, then use
the gear menu. Return to GNOME if the session does not start.

Continue with [configuration](/config).

## Update

```sh
sudo dnf upgrade --refresh gnoblin gnoblin-mutter
```

If you installed `gnoblin-portal`, update it separately:

```sh
sudo dnf upgrade --refresh gnoblin-portal
```

Log out and back in to load the updated compositor.

## Remove

Log into GNOME or another session first, then run:

If you installed the optional integration package, remove it with
`sudo dnf remove gnoblin-gnome-integration` before removing the session.

```sh
sudo dnf remove gnoblin gnoblin-mutter
```

If you installed `gnoblin-portal`, remove it with
`sudo dnf remove gnoblin-portal`.

Your shell and personal configuration are separate. Remove your desktop shell separately if you no longer want it.

## If installation fails

- **Package not found:** check your Fedora release and architecture against
  the repository above. Do not use another Fedora release's RPMs.
- **Missing dependency:** keep DNF's exact error when reporting it. Do not use
  `--skip-broken` to produce a partial desktop install.

See [login troubleshooting](troubleshooting.md) if the session does not start.

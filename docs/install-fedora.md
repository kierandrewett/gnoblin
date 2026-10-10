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

`gnoblin` installs the Lua-supervised session, its private Mutter runtime, and
the portal route for Gnoblin sessions. The session and compositor run from one
`gnoblin` executable. It does not install GNOME Shell, GJS, or a portal
backend. Install `gnoblin-portal` to add Gnoblin's GTK-based backend.
The route uses it when installed and otherwise selects another installed
backend.

To route individual interfaces to other backends, use the Lua config. Install
the backend with `sudo dnf install gnoblin-portal`; this command skips packages
recommended by dependencies. Install optional services you need using the
commands below.

Gnoblin uses the portal backend selected for its session. If GNOME is also
installed, that session keeps using its own portal configuration and backend.
Gnoblin does not install `xdg-desktop-portal-gtk` or require
`gnome-desktop4`. Package dependencies may still bring GTK3 onto a system;
inspect the transaction before installing if that matters to you.

For GNOME apps on a minimal Fedora install, install the
[shared desktop services](gnome-apps.md) first. Install the apps you want
separately.

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

For IBus input methods, install `ibus` and an engine with
`sudo dnf install ibus ibus-m17n`. The basic session needs only `ibus-libs`.
Gnoblin includes the input method but never starts the IBus daemon. See
[Type with an input method](/config/configure/input_sources#type-with-an-input-method).

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
sudo dnf upgrade --refresh gnoblin
```

If you installed `gnoblin-portal`, update it separately:

```sh
sudo dnf upgrade --refresh gnoblin-portal
```

Log out and back in to load the updated compositor.

## Remove

Log into GNOME or another session first, then run:

```sh
sudo dnf remove gnoblin
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

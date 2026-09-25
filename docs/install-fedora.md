# Fedora

Fedora 44 has a COPR package candidate. It passes package build, clean
installation, stock GNOME coexistence and removal checks, but does not yet have
a verified graphical Gnoblin login. Keep GNOME or another session available.
See [platform support](platform-support.md) for the release status.

## 1. Install Gnoblin

```sh
sudo dnf install dnf-plugins-core
sudo dnf copr enable kierandrewett/gnoblin
sudo dnf install --refresh gnoblin-session
```

`gnoblin-session` pulls in Gnoblin's Mutter and Shell packages.

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
sudo dnf upgrade --refresh gnoblin-session gnoblin-shell gnoblin-mutter
```

Log out and back in to load the updated compositor.

## Remove

Log into GNOME or another session first, then run:

```sh
sudo dnf remove gnoblin-session gnoblin-shell gnoblin-mutter
```

Your shell and personal configuration are separate. Remove your desktop shell separately if you no longer want it.

## If installation fails

- **Package not found:** check your Fedora release and architecture against
  the repository above. Do not use another Fedora release's RPMs.
- **Missing dependency:** keep DNF's exact error when reporting it. Do not use
  `--skip-broken` to produce a partial desktop install.

See [login troubleshooting](troubleshooting.md) if the session does not start.

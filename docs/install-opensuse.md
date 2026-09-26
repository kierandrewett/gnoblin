# openSUSE Tumbleweed

Gnoblin's Tumbleweed RPM set is attached to each GitHub release. It installs
beside stock GNOME. Tumbleweed has package build, co-installation and removal
checks, but has not passed a real graphical login check. Keep a working session
available while testing. See [platform support](platform-support.md).

## Install

Download all RPM assets from the
[latest Gnoblin release](https://github.com/kierandrewett/gnoblin/releases),
then install the complete set together. With GitHub CLI:

```sh
mkdir -p gnoblin-rpms
gh release download --repo kierandrewett/gnoblin --pattern 'opensuse-*.rpm' --dir gnoblin-rpms
sudo zypper install --allow-unsigned-rpm ./gnoblin-rpms/*.rpm
```

Install a desktop shell such as [Bingux](bring-your-own-shell.md), log out, and
select **Gnoblin** at the login screen. If the session does not start, return
to your existing session.

## Remove

Log into another session first, then remove the Gnoblin packages:

```sh
sudo zypper remove \
  gnoblin gnoblin-session gnoblin-shell gnoblin-mutter \
  gnoblin-mutter-devel gnoblin-gsettings-desktop-schemas
```

Your existing GNOME packages remain installed.

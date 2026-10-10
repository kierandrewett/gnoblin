# openSUSE Tumbleweed

Use the source tarball on a system whose development libraries meet the pinned
GNOME requirements. The Tumbleweed RPM adapter can produce packages when its
repositories provide those versions, including the required desktop-schema
major version. Check [source build prerequisites](install-source.md) before
choosing a release asset.

The `gnoblin` RPM contains the compositor and session. The
`gnoblin-portal` RPM is optional.
Gnoblin runs as a standalone Lua session and does not install GNOME Shell or
GJS. Keep a working session available while installing. See
[platform support](platform-support.md).

## Install

Download the RPM assets from the
[latest Gnoblin release](https://github.com/kierandrewett/gnoblin/releases),
if it includes files named `opensuse-*.rpm`. Install the runtime RPMs
together. With GitHub CLI:

```sh
mkdir -p gnoblin-rpms
gh release download --repo kierandrewett/gnoblin --pattern 'opensuse-*.rpm' --dir gnoblin-rpms
sudo zypper install --allow-unsigned-rpm \
  ./gnoblin-rpms/opensuse-gnoblin-[0-9]*.rpm
```

To use Gnoblin's GTK-based portal backend, install its RPM too:

```sh
sudo zypper install --allow-unsigned-rpm ./gnoblin-rpms/opensuse-gnoblin-portal-[0-9]*.rpm
```

The core `gnoblin` package installs the default route for Gnoblin sessions.
It prefers Gnoblin's backend when installed and falls back to another installed
backend. The optional package only adds the GTK backend. You can route
interfaces to a different backend in the Lua config. GNOME keeps its own
portal selection.

Install a desktop shell such as [Bingux](bring-your-own-shell.md), log out, and
select **Gnoblin** at the login screen. If the session does not start, return
to your existing session.

For GNOME apps on a minimal install, install the
[shared desktop services](gnome-apps.md) first.

## Remove

Log into another session first. Then remove the Gnoblin packages you installed:

```sh
sudo zypper remove \
  gnoblin gnoblin-portal
```

Your existing GNOME packages remain installed.
Gnoblin and GNOME use the backend selected for each session. Install
`gnoblin-portal` only if you want Gnoblin's GTK-based backend.

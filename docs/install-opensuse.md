# openSUSE Tumbleweed

Use the source tarball on a system whose development libraries meet the pinned
GNOME requirements. The Tumbleweed RPM adapter can produce packages when its
repositories provide those versions, including the required desktop-schema
major version. Check [source build prerequisites](install-source.md) before
choosing a release asset.

The packages are designed to install beside stock GNOME. The revised package
set, including Gnoblin's portal backend, still needs build, co-installation,
removal, and graphical login checks. Keep a working session available. See
[platform support](platform-support.md).

## Install

Download all RPM assets from the
[latest Gnoblin release](https://github.com/kierandrewett/gnoblin/releases),
if it includes files named `opensuse-*.rpm`. Install the runtime RPMs
together. With GitHub CLI:

```sh
mkdir -p gnoblin-rpms
gh release download --repo kierandrewett/gnoblin --pattern 'opensuse-*.rpm' --dir gnoblin-rpms
sudo zypper install --allow-unsigned-rpm \
  ./gnoblin-rpms/opensuse-gnoblin-[0-9]*.rpm \
  ./gnoblin-rpms/opensuse-gnoblin-mutter-[0-9]*.rpm \
  ./gnoblin-rpms/opensuse-gnoblin-shell-[0-9]*.rpm \
  ./gnoblin-rpms/opensuse-gnoblin-portal-[0-9]*.rpm \
  ./gnoblin-rpms/opensuse-gnoblin-session-[0-9]*.rpm
```

Install a desktop shell such as [Bingux](bring-your-own-shell.md), log out, and
select **Gnoblin** at the login screen. If the session does not start, return
to your existing session.

For GNOME apps on a minimal install, add the optional
[`gnoblin-gnome-integration` RPM](gnome-apps.md). It is separate from the
runtime packages above.

## Remove

Log into another session first. If you installed the optional integration
package, remove it with `sudo zypper remove gnoblin-gnome-integration`.
Then remove the Gnoblin runtime packages:

```sh
sudo zypper remove \
  gnoblin gnoblin-session gnoblin-portal gnoblin-shell gnoblin-mutter
```

Your existing GNOME packages remain installed.
Gnoblin selects its own portal backend; GNOME keeps using its backend when it
is installed for that session.

# Installation

gnoblin isn't a package you install from a repo (yet — see [Packaging](#packaging)
below); it's a source tree you build into a local prefix. There's nothing to
uninstall if you change your mind: delete `./install` and `./build`.

## What you end up with

A `./install` prefix containing a patched Mutter + patched GNOME Shell, plus
the `gnoblin` GNOME Shell session mode, gnome-session definition, and login
entry. You select "Gnoblin" at your login manager like any other session; it
boots to a bare compositor with **no top bar, no dash, no overview** — chrome
is bring-your-own (Quickshell, waybar, a custom layer-shell client, or none).

## Requirements

gnoblin patches Mutter and GNOME Shell in place, so it needs their normal
build dependencies plus `meson`, `ninja`, `git`, and [`just`](https://github.com/casey/just)
(the task runner every recipe in this repo goes through).

Packaging is Fedora-first right now (`packaging/rpm/*.spec` is maintained
and build-verified; `packaging/deb/` and `packaging/arch/` are scaffolds —
see their READMEs). On Fedora, resolve the base build dependencies straight
from the spec files instead of hand-copying a package list that will drift:

```sh
sudo dnf install just meson ninja-build rpmdevtools
sudo dnf builddep packaging/rpm/mutter.spec packaging/rpm/gnome-shell.spec
```

On Arch or Debian/Ubuntu, translate the `BuildRequires:`/`pkgconfig(...)`
lines in those same spec files into your distro's package names — there
isn't a maintained dependency list for those distros yet.

## Get the source

```sh
git clone <this-repo> gnoblin
cd gnoblin
just init      # fetches the pinned mutter (49.5) + gnome-shell (49.6) submodules
```

`just init` also prints what each submodule resolved to, so you can confirm
you're on the pinned tags.

## Build

```sh
just dev
```

This builds patched Mutter, then patched GNOME Shell against it, then
installs the gnoblin session data — all into `./install` (no system files
touched). It's the same as running `just dev-mutter`, `just dev-gnome-shell`,
and `just dev-session` in sequence; see the root README's "Build" section if
you want to run a single stage (e.g. while iterating on one component).

A clean build takes a while (you're compiling Mutter and GNOME Shell).
Re-running `just dev` after an edit only rebuilds what changed, except
`dev-gnome-shell`, which always does a clean rebuild of `build/gnome-shell` —
see the comment on that recipe in the `Justfile` for why (a reused build dir
picks up half-stale generated sources).

## Confirm it built

```sh
just gnome-verify
```

Headless: boots the patched shell in the `gnoblin` session mode, checks that it
advertises `zwlr_layer_shell_v1`, and verifies Gnoblin-only Shell policy. Run
`just gnome-stock-protocol-isolation-verify` to prove the same packages retain
the native panel, extension validation and protocol isolation in stock mode.
See [Testing](testing.md) for the rest of the verify suite.

## Try it without logging out

```sh
just gnome-devkit
```

Opens a visible nested gnoblin session (a window in your current session)
plus a terminal wired to it, so you can run your own chrome against a real
gnoblin compositor before touching your login session at all. See
[Devkit](devkit.md).

## Install the session for real

```sh
just dev-session              # already run as part of `just dev`
just dev-session-register     # links the session with your login manager
```

`dev-session` installs `gnoblin.desktop`, the `gnoblin` gnome-session, the
`gnoblin` GNOME Shell session mode, and a `gnoblin`-specific pair of
systemd --user units (`org.gnoblin.Shell.target` /
`org.gnoblin.Shell@wayland.service`) into `./install`. It's additive and
reversible: everything lands under `./install`, nothing else is touched.

`dev-session-register` is the one step that reaches outside `./install` —
run separately (not part of `just dev`) because of that:

- Links `org.gnoblin.Shell.target`/`@wayland.service` into systemd --user's
  search path (`~/.config/systemd/user/`). These are gnoblin-specific unit
  names, not the shared `org.gnome.Shell@wayland.service` every other
  gnome-session mode resolves — so this does **not** shadow or affect a
  system GNOME Shell install; your regular GNOME session (if you have one)
  keeps using its own unit untouched. No root needed for this part.
- Prints (doesn't run) the one remaining command, which does need root:
  copying `gnoblin.desktop` into `/usr/share/wayland-sessions/`. Login
  managers read session `.desktop` files from a fixed system directory —
  there's no user-writable equivalent, so this can't be avoided short of a
  real package install (see [Packaging](#packaging)).

```sh
sudo install -Dm644 ./install/share/wayland-sessions/gnoblin.desktop \
  /usr/share/wayland-sessions/gnoblin.desktop
```

"Gnoblin" then appears at your login manager's session picker. Fully
reversible — `scripts/register-session.sh` prints the exact undo commands
(`rm` the two linked units + `daemon-reload`, `sudo rm` the `.desktop`), and
picking any other session at the login screen is unaffected either way.

The full checklist for what to expect once you're logged in (bare session,
`gnoblinctl` sanity checks, bringing up your own chrome) is in
[Real-hardware verification](real-hardware-verification.md#1-log-in-to-a-real-gnoblin-session).

## Optional components

Neither of these is part of `just dev` — build them explicitly once you need
them.

### Unattended screensharing (`xdg-desktop-portal-gnome`)

The optional portal backend can remember exact, portal-scoped Screen Cast and
Remote Desktop permissions for a verified requester. It needs one extra build
dependency:

```sh
sudo dnf install xdg-desktop-portal-devel
just dev-portal
```

Then run the patched backend so it owns the impl portal:

```sh
./install/libexec/xdg-desktop-portal-gnome -r
```

The [real-hardware verification guide](real-hardware-verification.md#7-persistent-screen-cast-and-remote-desktop-grants)
shows the first approval, exact-capability restore, storage, and revocation flow.

### gnoblin Settings (forked `gnome-control-center`)

A `gnoblin` panel in GNOME Settings driving `org.gnoblin.Shell` (feature
toggles, Screen Cast and Remote Desktop grants, and a reload button):

```sh
sudo dnf install accountsservice-devel colord-gtk4-devel cups-devel gsound-devel \
  ibus-devel libgtop2-devel libnma-gtk4-devel malcontent-devel \
  ModemManager-glib-devel libpwquality-devel libsmbclient-devel libudisks2-devel
just dev-settings
./install/bin/gnome-control-center gnoblin
```

`dev-settings` also hides the Multitasking panel (no top bar/overview/dash
under gnoblin, so it doesn't apply) and handles the `blueprint-compiler`
build-side quirk automatically. Details in
[Real-hardware verification §8](real-hardware-verification.md#8-gnoblin-settings-forked-gnome-control-center).

## Packaging

RPM (Fedora) is the maintained, build-verified path. `just rpm PROJ` produces
a real installable RPM for `mutter` or `gnome-shell`:

```sh
just rpm mutter
just rpm gnome-shell
# or: just rpm-all       # both, per Justfile's rpm_projects
```

`gnome-shell.spec` also builds a `gnoblin-session` subpackage: the `gnoblin`
session mode, its login-manager entry, gnoblin-specific systemd --user units
(`org.gnoblin.Shell.target`/`@wayland.service` — distinct from the shared
`org.gnome.Shell@wayland.service`, so installing it never shadows a system
GNOME Shell install's own units), and the control/wrapper tools
(`gnoblinctl`, `gnoblin-session`, `gnoblin-shell-service`). `Requires:` pins
it to the exact matching `gnome-shell` build. RPMs land in
`~/rpmbuild/RPMS/`.

**Installing these RPMs replaces your system's Mutter and GNOME Shell
packages** — that's the point (patches are session-mode-gated and meant to
be transparent for non-gnoblin sessions, per the architecture in the root
README), but it's a real system change, not something to run without
meaning to:

```sh
sudo dnf install \
  ~/rpmbuild/RPMS/x86_64/mutter-49.5-*.rpm \
  ~/rpmbuild/RPMS/noarch/mutter-common-49.5-*.rpm \
  ~/rpmbuild/RPMS/x86_64/gnome-shell-49.6-*.rpm \
  ~/rpmbuild/RPMS/noarch/gnome-shell-common-49.6-*.rpm \
  ~/rpmbuild/RPMS/x86_64/gnoblin-session-49.6-*.rpm
```

(`mutter`/`gnome-shell` each `Requires:` their own `-common` noarch
subpackage at the exact same build — they won't resolve against whatever
`mutter-common`/`gnome-shell-common` your system repos already have, since
the version+release won't match a `.gnoblin` build. Skipping either
`-common` RPM above fails dependency resolution.)

`dnf` will show exactly what it's replacing before you confirm. After that,
"Gnoblin" appears at your login manager's session picker with no further
registration step (unlike the dev-prefix path in
[§ Install the session for real](#install-the-session-for-real) — a system
install needs none of the `dev-session-register` systemd-unit-collision
workaround, since there's no dev prefix to disambiguate from). Roll back
with `sudo dnf downgrade mutter gnome-shell` (or `dnf history undo`) and
`sudo dnf remove gnoblin-session`.

Debian/Ubuntu and Arch packaging are scaffolded but not implemented — see
`packaging/deb/README.md` and `packaging/arch/README.md` for the intended
approach, which mirrors this RPM path including the `gnoblin-session` split.

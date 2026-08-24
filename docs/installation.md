# Installation

Gnoblin has a reproducible Nix flake for NixOS and a source-prefix development
path for other systems. The source-prefix path does not install a system package:
delete `./install` and `./build` to remove it.

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

For non-NixOS systems, packaging is Fedora-first right now
(`packaging/rpm/*.spec` is maintained and build-verified; `packaging/deb/`
and `packaging/arch/` are scaffolds - see their READMEs). On Fedora, resolve
the base build dependencies from the spec files instead of hand-copying a
package list that will drift:

```sh
sudo dnf install just meson ninja-build rpmdevtools
sudo dnf builddep packaging/rpm/mutter.spec packaging/rpm/gnome-shell.spec
```

On Arch or Debian/Ubuntu, translate the `BuildRequires:`/`pkgconfig(...)`
lines in those same spec files into your distro's package names — there
isn't a maintained dependency list for those distros yet.

## NixOS

The flake exports `packages.x86_64-linux.gnoblin` and
`nixosModules.default`. The package builds the pinned Mutter 49.5 and GNOME
Shell 49.6 sources, their required Meson subprojects, the Gnoblin session
files, and its distinct systemd user units in one Nix store path.

Use a Nixpkgs release with the GNOME 49 stack. The flake uses
`nixos-25.11`. Do not make its `nixpkgs` input follow a parent input with a
different GNOME major version. A mixed GDM, gnome-session, Mutter, and
GNOME Shell stack is not supported.

```nix
# flake.nix
inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

    gnoblin = {
        url = "github:kierandrewett/gnoblin";
        inputs.nixpkgs.follows = "nixpkgs";
    };
};
```

Import the module in a NixOS host or profile, then enable a display manager
separately. The module does not select a display manager or install desktop
chrome.

```nix
{
    imports = [ inputs.gnoblin.nixosModules.default ];

    programs.gnoblin.enable = true;
    services.displayManager.gdm.enable = true;
    services.displayManager.defaultSession = "gnoblin";
}
```

Build the package alone while you develop packaging changes:

```sh
nix flake check
nix build .#gnoblin
```

The package declares its `gnoblin` Wayland session to
`services.displayManager.sessionPackages` and installs the required
`org.gnoblin.Shell` user units. It does not replace a stock GNOME Shell
package globally. Gnoblin still starts without a top bar, dock, overview,
notifications, or on-screen display. A selected desktop shell must provide
those surfaces.

## Get the source

```sh
git clone <this-repo> gnoblin
cd gnoblin
just init      # fetches pinned GNOME checkouts and mandatory Meson subprojects
```

`just init` also materialises the pinned `gvdb`, `gvc`, `libshew`, and
`jasmine-gjs` source trees used by no-download package builds, then prints the
resolved Mutter and GNOME Shell tags.

## Build

```sh
just dev
```

This builds patched Mutter, then patched GNOME Shell against it, then
installs the gnoblin session data — all into `./install` (no system files
touched). It's the same as running `just dev-mutter`, `just dev-gnome-shell`,
and `just dev-session` in sequence; see the root README's "Build" section if
you want to run a single stage (e.g. while iterating on one component).

The default layout is `GNOBLIN_PREFIX=$PWD/install` with
`GNOBLIN_LIBDIR=lib64`. Override both through the environment when the target
uses another prefix layout:

```sh
GNOBLIN_PREFIX=/tmp/gnoblin GNOBLIN_LIBDIR=lib just dev
```

`GNOBLIN_LIBDIR` must be relative to the prefix. Session installation records
it in `<prefix>/libexec/gnoblin-libdir`, so later GDM and systemd launches use
the same library directory without inheriting the build environment.

A clean build takes a while (you're compiling Mutter and GNOME Shell).
Re-running `just dev` after an edit only rebuilds what changed, except
`dev-gnome-shell`, which always does a clean rebuild of `build/gnome-shell` —
see the comment on that recipe in the `Justfile` for why (a reused build dir
picks up half-stale generated sources).

## Confirm it built

```sh
just verify-installed-headless
```

This runs every isolated GNOME Shell integration recipe against the prefix
that `just dev` built, including Gnoblin and stock session policy, protocol
boundaries and gating, D-Bus state, reloads, scripting, notifications, and the
devkit environment. Use `just gnome-verify` when you only need the basic boot
smoke. See [Testing](testing.md) for the full local and release gates.

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
packages.** Gnoblin-only Shell behaviour and privileged Wayland globals are
session-mode-gated. Lower-level correctness and crash fixes apply to the
shared packages, so this is still a real system change. Review `dnf`'s
transaction before confirming:

```sh
just install-session          # prompts, shows the transaction
just install-session dry      # resolve + print only, changes nothing
just install-session yes      # no prompt
```

That resolves the RPM set from the spec versions, installs it in one
transaction, and verifies afterwards. It exists because three things bite in
sequence, and hitting them one at a time from a login screen is miserable:

- Dev-prefix unit symlinks in `~/.config/systemd/user` **shadow** the
  packaged units — that search path outranks `/usr/lib/systemd/user`, so
  after a clean package install `gnome-session` would still resolve
  `org.gnoblin.Shell` to whatever `./install` had, or fail outright once
  that prefix is stale or gone. `just dev-session-register` is what puts
  them there, so anyone who tried the dev path first is carrying them. The
  recipe removes them (symlinks only; a real file is left alone and warned
  about).
- The RPM release is `1.gnoblin`, which rpm sorts *older* than a
  `1.<anything-after-g>` build of the same version (`1.kdr`, say). `dnf`
  calls that a downgrade and declines without `--allow-downgrade`.
- `mutter`/`gnome-shell` each `Requires:` their own `-common` noarch
  subpackage at the exact same build — they won't resolve against whatever
  `mutter-common`/`gnome-shell-common` your system repos already have, since
  the version+release won't match a `.gnoblin` build. All five RPMs have to
  go in one transaction.
- Any *other* installed subpackage pins the base package the same way.
  `mutter-devel` carries `Requires: mutter = %{version}-%{release}`, so if you
  have it installed, swapping `mutter` out from under it fails the whole
  transaction with *"none of the providers can be installed"*. The recipe
  pulls in the matching `.gnoblin` build of anything already installed
  (`mutter-devel`, `mutter-tests`) so they move together.
- An earlier iteration of this packaging installed under `/opt/gnoblin` as
  `gnoblin-mutter`/`gnoblin-gnome-shell`. If you have those, the recipe
  removes them first so the two layouts never coexist.

The equivalent by hand, if you'd rather drive it yourself:

```sh
sudo dnf install --allow-downgrade \
  ~/rpmbuild/RPMS/x86_64/mutter-49.5-*.gnoblin*.rpm \
  ~/rpmbuild/RPMS/noarch/mutter-common-49.5-*.gnoblin*.rpm \
  ~/rpmbuild/RPMS/x86_64/gnome-shell-49.6-*.gnoblin*.rpm \
  ~/rpmbuild/RPMS/noarch/gnome-shell-common-49.6-*.gnoblin*.rpm \
  ~/rpmbuild/RPMS/x86_64/gnoblin-session-49.6-*.gnoblin*.rpm
```

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

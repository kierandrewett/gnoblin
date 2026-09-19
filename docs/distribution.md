# Distribution work

Gnoblin supplies the compositor and login session. Bingux supplies the desktop
shell. They need separate packages and separate release checks.

Install Gnoblin first, then install a layer-shell client such as Bingux. A
successful Gnoblin package install intentionally leaves the session without a
bar, dock or notification centre until that client is enabled.

## Release checklist

- [x] Build Gnoblin source RPMs from a clean checkout with all patches included.
- [x] Create Fedora COPR projects for Gnoblin and Bingux.
- [x] Build Mutter before GNOME Shell in COPR.
- [ ] Verify installation, login and rollback on a clean supported Fedora host.
- [x] Give Bingux a standalone build, install manifest and service definitions.
- [x] Move Bingux packaging and runtime dependencies to its native shell tree.
- [ ] Build and test Bingux RPMs with the matching Qt and Quickshell runtime.
- [ ] Add signed APT and pacman repositories after distribution-specific builds pass.
- [x] Publish installation instructions for the Fedora COPR package set.

## Package boundaries

Every package must install alongside GNOME. RPMs use `gnoblin-mutter`,
`gnoblin-shell` and `gnoblin-session`, with a private runtime in
`/usr/lib/gnoblin`. Only Gnoblin's login entry, control tool, service units
and separately named backlight policy enter system directories. Private
libraries must not provide dependencies for stock GNOME packages.

NixOS exposes the same limited set of entry points from a separate store
output. Source builds use a private prefix and reject `/usr` and `/usr/local`.
Future Debian and Arch packages must follow the same rule.

Bingux needs Quickshell, native QML plugins, its search and metrics daemons,
and helper programs. Copying its QML directory alone is not a complete install.
The Bingux repository owns its native build, install manifest, RPM spec and
user-systemd units; it is no longer a machine-configuration repository.

## Repository status

The [Gnoblin COPR](https://copr.fedorainfracloud.org/coprs/kierandrewett/gnoblin/)
and [Bingux COPR](https://copr.fedorainfracloud.org/coprs/kierandrewett/bingux/)
exist for Fedora 44 x86_64. Gnoblin's private Mutter and Shell packages are
published; fresh-install, login and rollback checks on a clean host remain.
Creating a repository does not establish release readiness.

The side-by-side RPM layout still needs a clean-host login and removal check.
Old replacement RPMs are not supported by the new installer; restoring a host
already using them is a separate migration.

## Publishing access

COPR requires an authenticated Fedora account. Configure credentials locally
using https://copr.fedorainfracloud.org/api/. Do not commit API tokens.
APT and pacman repositories also need a publication host and a signing key.
No repository should be described as available before its packages have built.

## Prepare and submit Fedora builds

Install `rpm-build` and `copr-cli`. Use a clean release checkout: preparing
sources applies overlays to the pinned upstream trees.

```sh
just init
scripts/make-tarball.sh mutter ./dist/sources
scripts/make-tarball.sh gnome-shell ./dist/sources
scripts/build-srpm.sh mutter ./dist/sources ./dist/srpms
scripts/build-srpm.sh gnome-shell ./dist/sources ./dist/srpms
```

Source RPM creation uses the prepared archives. It never downloads the
unpatched upstream archives named by the Fedora specs.

## GNOME major upgrades

`gnome-versions.json` is the single source of truth for the GNOME release train.
The weekly `Check for a new GNOME release` workflow fails when all five pinned
upstream projects publish a newer stable major, making the new release visible
in GitHub's workflow notifications.

Start an upgrade with:

```sh
./scripts/gnome-versions.py update 52
```

This verifies each upstream `52.0` tag, records its exact commit, and updates
the generated RPM, Nix, runtime API, and CI fields. Then rebase every patch
stack onto the recorded commits, update the submodule gitlinks, and run:

```sh
just check-gnome-version
just verify
```

Patch rebasing remains deliberate because upstream API changes need review;
release discovery and version propagation are automated.

Create a COPR project with a Fedora 44 chroot that supplies the GNOME 50
runtime dependencies while building Gnoblin's private GNOME 51 stack. For
example, use `fedora-44-x86_64`; confirm the
chroot is still available in COPR before creating the project.

```sh
copr-cli create --chroot fedora-44-x86_64 gnoblin
scripts/publish-copr.sh OWNER/gnoblin PATH_TO_MUTTER_SRPM PATH_TO_SHELL_SRPM
```

Replace the owner and file paths with the actual account and generated files.
The submission script waits for Mutter to build before submitting GNOME Shell.
Do not use `--nowait`: Shell must build against the published Mutter headers.
Confirm both builds succeed before testing a fresh installation.

After installing the packages, select **Gnoblin** at GDM and enable the shell
from [Bring your own shell](bring-your-own-shell.md). Keep the regular GNOME
session as the rollback path while the first login is being verified.

COPR usage and authentication are documented in the
[official user guide](https://docs.copr.fedorainfracloud.org/user_documentation.html).

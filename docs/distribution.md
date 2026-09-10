# Distribution work

Gnoblin supplies the compositor and login session. Bingux supplies the desktop
shell. They need separate packages and separate release checks.

Install Gnoblin first, then install a layer-shell client such as Bingux. A
successful Gnoblin package install intentionally leaves the session without a
bar, dock or notification centre until that client is enabled.

## Release checklist

- [ ] Build Gnoblin source RPMs from a clean checkout with all patches included.
- [x] Create Fedora COPR projects for Gnoblin and Bingux.
- [ ] Build Mutter before GNOME Shell in COPR.
- [ ] Verify installation, login and rollback on a clean supported Fedora host.
- [x] Give Bingux a standalone build, install manifest and service definitions.
- [x] Move Bingux packaging and runtime dependencies to its native shell tree.
- [ ] Build and test Bingux RPMs with the matching Qt and Quickshell runtime.
- [ ] Add signed APT and pacman repositories after distribution-specific builds pass.
- [ ] Publish installation instructions only for verified package sets.

## Package boundaries

The current Gnoblin RPMs replace Fedora's Mutter and GNOME Shell packages.
The `gnoblin-session` package depends on the matching GNOME Shell build. Build
and update the complete set together. Do not claim compatibility with another
GNOME major version without rebuilding and testing it.

Bingux needs Quickshell, native QML plugins, its search and metrics daemons,
and helper programs. Copying its QML directory alone is not a complete install.
The Bingux repository owns its native build, install manifest, RPM spec and
user-systemd units; it is no longer a machine-configuration repository.

## Repository status

The [Gnoblin COPR](https://copr.fedorainfracloud.org/coprs/kierandrewett/gnoblin/)
and [Bingux COPR](https://copr.fedorainfracloud.org/coprs/kierandrewett/bingux/)
exist for Fedora 43 x86_64. Package publication and fresh-install checks are
pending. Creating a repository does not establish release readiness.

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

Create a COPR project with a chroot that supplies GNOME 49. For example,
`fedora-43-x86_64` matches the current source major version; confirm the
chroot is still available in COPR before creating the project.

```sh
copr-cli create --chroot fedora-43-x86_64 gnoblin
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

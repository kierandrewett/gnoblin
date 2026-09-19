# Distribution work

Gnoblin supplies the compositor and login session. Bingux supplies the desktop
shell. They need separate packages and separate release checks.

Install Gnoblin first, then install a layer-shell client such as Bingux. A
successful Gnoblin package install intentionally leaves the session without a
bar, dock or notification centre until that client is enabled.

## Packaging architecture

Nix is the source of truth for native packaging, not the only installation
format. `nix/native-packages.nix` describes Gnoblin's package outputs,
capability-level dependencies, minimum versions, and RPM/DEB/Arch package-name
translations. Inspect the interface with:

```sh
nix eval --json .#lib.nativePackages
```

The same command materializes the audit snapshot and concrete `gnoblin`
metapackage recipes for RPM/COPR, Debian, and Arch. Refresh or verify every
generated adapter with:

```sh
just package-manifest write
just package-manifest
```

RPM/COPR, Debian repositories, and Arch PKGBUILDs remain native delivery
adapters. They install ordinary native dependencies, so an existing compatible
GNOME userspace is reused and only missing or outdated packages are resolved.
Build flags, source revisions, package relationships, and minimum versions must
come from the Nix interface rather than being independently redefined by each
adapter.

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

## GitHub releases

Push the release commit first, then create and push the tag matching the version
in `gnome-versions.json`:

```sh
git tag -s v51.0 -m "Gnoblin 51.0"
git push origin v51.0
```

The `Release` workflow builds the patched Mutter and GNOME Shell source
archives, the `gnoblin-mutter`, `gnoblin-shell`, and `gnoblin` source RPMs, and
`SHA256SUMS`. It publishes those files to the GitHub release only after every
artifact has built. A failed or interrupted release can be repaired with the
workflow's manual dispatch for the existing tag; uploaded assets are replaced
atomically by name.

GitHub releases do not submit to COPR. COPR credentials are deliberately kept
out of the tag workflow, and binary publication remains the ordered, explicit
step documented below.

## Package boundaries

Every package must install alongside GNOME. RPMs use `gnoblin-mutter`,
`gnoblin-shell` and `gnoblin-session`, with a private runtime in
`/usr/lib/gnoblin`. Only Gnoblin's login entry, control tool, service units
and separately named backlight policy enter system directories. Private
libraries must not provide dependencies for stock GNOME packages.

NixOS exposes the same limited set of entry points from a separate store
output. Source builds use a private prefix and reject `/usr` and `/usr/local`.
Debian and Arch packages must follow the same rule.

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
scripts/build-srpm.sh gnoblin ./dist/sources ./dist/srpms
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
scripts/publish-copr.sh OWNER/gnoblin PATH_TO_MUTTER_SRPM PATH_TO_SHELL_SRPM PATH_TO_META_SRPM
```

Replace the owner and file paths with the actual account and generated files.
The submission script waits for Mutter to build before submitting GNOME Shell.
Do not use `--nowait`: Shell must build against the published Mutter headers.
Confirm all three builds succeed before testing `dnf install gnoblin` on a
fresh host.

After installing the packages, select **Gnoblin** at GDM and enable the shell
from [Bring your own shell](bring-your-own-shell.md). Keep the regular GNOME
session as the rollback path while the first login is being verified.

COPR usage and authentication are documented in the
[official user guide](https://docs.copr.fedorainfracloud.org/user_documentation.html).

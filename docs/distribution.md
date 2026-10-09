# Packaging and releases

For user installation, see [Install Gnoblin](installation.md).
This page is for release maintainers.

The public [platform support](platform-support.md) page distinguishes package
candidates from fully supported session installations. Do not describe a target
as supported until its graphical-session gate has passed.

## Package layout

Gnoblin installs as its own session and can coexist with GNOME.

- RPM names: `gnoblin`, the optional `gnoblin-portal`, and the optional
  `gnoblin-gnome-integration` subpackage.
- The RPM and Arch `gnoblin` packages contain the single `gnoblin` compositor
  executable, login entry, Lua runtime, session services, and Gnoblin-specific
  portal route. They do not require a portal backend or a separate Mutter
  runtime package.
- Private runtime: `/usr/lib/gnoblin`.
- Public files: login entry, control tool, service units and named policy files.
- Private libraries must not satisfy stock GNOME dependencies.
- `gnoblinctl` uses GLib/GIO and JSON-GLib at runtime. Python is a build tool,
  not a base package requirement.

Nix uses separate store outputs. Source builds use a private prefix.
Bingux owns and releases its shell package separately.

## Package definitions

`packaging/native-packages.json` defines the RPM and Arch package requirements
and their distribution names. Generate or verify the adapters with Python:

```sh
./scripts/sync-package-manifest.py write
./scripts/sync-package-manifest.py check
```

The package check also compares pinned versions with `gnome-versions.json` and
`gnoblin-version.json`. Nix consumes the same JSON for its optional package
adapter; it is not required to prepare native packages.

Fedora's COPR packages use `gnoblin` as the install entry point. There is
currently no Gnoblin APT or pacman repository.

RPM and Arch login entries launch the lean session directly, as
registering a source build with `make install` does. GNOME Session and
Settings Daemon are not package requirements. The source tarball remains the
primary install route until a distribution package passes its login gate.

The optional `gnoblin-portal` package provides Gnoblin's GTK-based backend.
Users can choose another portal instead. The optional integration subpackage
adds GVfs, GNOME Keyring, and user-directory setup. The GTK folder-name updater
is separate. Neither package installs applications.
Arch publishes it as a separate metadata-only PKGBUILD and package archive.

## Prepare the source tarball

From a clean release checkout, build the source assets first:

```sh
./build.sh package --output ./dist/release
```

`--output` sets the directory, which must be empty. `--srpm` also builds the Fedora
source RPMs and needs `rpm-build`. `--release-tag TAG` checks `TAG` against the
version in the tree, for a release build.

The `gnoblin-*-source.tar.xz` file contains Gnoblin and its patched, pinned
Mutter and portal sources. Extract it and run `make` to check the
same source route users receive. The command also writes component archives,
an Arch recipe, and checksums into `dist/release`.

A release tag starts the release workflow:

1. It creates a draft GitHub release with the source tarball.
2. It builds and installs every package.
3. It publishes the release only when every build and install check has passed.

A failed check leaves the draft in place. A push to `main` runs the same builds and
checks and publishes nothing.

## Prepare Fedora source RPMs

Use a clean release checkout with `rpm-build` installed. The package does not
build the optional Adwaita vector cursor theme.

```sh
./build.sh package --output ./dist/rpm-assets --srpm
```

The command writes the source tarball before building source RPMs for the
unified `gnoblin` runtime and optional portal backend. Archives include
Gnoblin's overlays and patches. They must not be replaced with unpatched
upstream archives.

## Publish to COPR

Configure a Fedora account using the [COPR API page](https://copr.fedorainfracloud.org/api/).
Keep credentials outside the repository.

```sh
scripts/publish-copr.sh OWNER/gnoblin PATH_TO_PORTAL_SRPM PATH_TO_GNOBLIN_SRPM
```

Replace the owner and paths. The script publishes the unified `gnoblin`
runtime and portal extension. It does not build or publish GNOME Shell.

Use a currently available chroot supplying the required dependencies.
After all builds succeed, test package resolution, login and removal on a clean
host using [hardware verification](real-hardware-verification.md).

## GitHub releases

Gnoblin and GNOME version independently. `gnoblin-version.json` is the
canonical Gnoblin [SemVer](https://semver.org/) identity; `gnome-versions.json`
continues to pin the compatible GNOME train. For example, Gnoblin `0.1.0` can
target GNOME `51.0` without claiming that it is GNOME version `0.1.0`.

Before release, update `gnoblin-version.json` deliberately: use a major version
for incompatible Gnoblin configuration or protocol changes, a minor version for
backwards-compatible features, and a patch version for compatible fixes. Then
push the release commit and a signed `gnoblin-v<semver>` tag:

```sh
git tag -s gnoblin-v0.1.0 -m "Gnoblin 0.1.0 (GNOME 51.0)"
git push origin gnoblin-v0.1.0
```

The release workflow publishes a self-contained Gnoblin source tarball and
the Mutter and portal source archives first, then source RPMs and binary packages.
The main tarball builds with `make` and does not need Git or submodules.
The Fedora source RPM and openSUSE jobs use the component archives inside that
tarball, after checking the published source assets' SHA-256 sums.

Tagged commits create versioned releases. Other commits on
`main` create prereleases named after their commit SHA. Manual dispatch can
repair assets for an existing SemVer tag.

`v<gnome-version>` tags predate this convention and remain historical releases.

The source release is public as soon as its archives are ready. Package jobs
add their artifacts when they complete. The Arch package is attached only after
a clean installation resolves its runtime dependencies, then installs the
optional GNOME app integration package.

A package job can fail because its
distribution repository lacks a required dependency version; the release still
publishes source archives and checksums for the assets that succeeded. Check
the package jobs before advertising a package for that distribution.

The COPR job installs the published Fedora package without stock GNOME, adds
the optional app integration package, then checks coexistence with GNOME.
The openSUSE package job checks the same installation order from its built RPMs.

COPR requires the repository secret `COPR_CONFIG`, containing the publisher's
`copr-cli` configuration. Configure it once before the first automated release:

```sh
gh secret set COPR_CONFIG < ~/.config/copr
```

Fedora users receive the resulting COPR update through normal `dnf` updates.
Check the completed release workflow before telling users a package candidate
is available. Do not claim graphical-session support until the target's login
gate passes.

## Upgrade the GNOME base

`gnome-versions.json` pins the release train.

```sh
./scripts/gnome-versions.py update 52
```

Use the intended major version. The script verifies upstream tags and updates
generated version fields. Then rebase patches, update submodule references,
refresh the pinned Nix inputs, and run:

```sh
./scripts/gnome-versions.py check --upstream
nix flake update mutter-src portal-src gxdp-src
make check
make test
```

Patch rebasing and runtime compatibility still require review.

## Release checklist

1. Build all artifacts from the release checkout.
2. Verify private package paths and stock GNOME coexistence.
3. Publish packages and check dependency resolution.
4. Test login, desktop-shell integration, portal access and removal on a clean host.
5. Update installation commands and version notes to match published packages.

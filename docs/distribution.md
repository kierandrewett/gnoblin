# Packaging and releases

For user installation, see [Install Gnoblin](installation.md).
This page is for release maintainers.

## Package layout

Gnoblin must install alongside GNOME.

- RPM names: `gnoblin-mutter`, `gnoblin-shell`, `gnoblin-session`.
- Debian/Ubuntu name: `gnoblin` (compositor, session and private libraries together).
- Private runtime: `/usr/lib/gnoblin`.
- Public files: login entry, control tool, service units and named policy files.
- Private libraries must not satisfy stock GNOME dependencies.

Nix uses separate store outputs. Source builds use a private prefix.
Bingux owns and releases its shell package separately.

## Package definitions

`nix/native-packages.nix` defines the RPM and Arch adapters and their package-name
mappings. Generate or verify those adapters with:

```sh
nix eval --json .#lib.nativePackages
just package-manifest write
just package-manifest
```

Fedora's COPR packages use `gnoblin-session` as the entry point.
Debian and Ubuntu use the `gnoblin` package from the signed
[Gnoblin APT archive](install-debian.md). There is no pacman repository.

## Build Debian and Ubuntu packages

The supported targets are Debian 13, Ubuntu 24.04 LTS and Ubuntu 26.04 LTS.
Build separately in each distribution's container; do not reuse a newer
distribution's binary package on an older one.

Follow the [container build instructions](../packaging/deb/README.md).
The builder compiles the required newer libraries into `/usr/lib/gnoblin/deps`
and produces a `.deb` with the remaining system dependencies recorded for APT.
No Nix installation is required.

The package tests install stock GNOME first, then exercise Gnoblin's installed
CLI and compositor in a headless session. They also check removal and verify
that GNOME's binary is unchanged. A real login test remains part of release
verification.

## Prepare Fedora source RPMs

Use a clean release checkout and install `rpm-build` and `copr-cli`.

```sh
just setup
scripts/make-tarball.sh mutter ./dist/sources
scripts/make-tarball.sh gnome-shell ./dist/sources
scripts/build-srpm.sh mutter ./dist/sources ./dist/srpms
scripts/build-srpm.sh gnome-shell ./dist/sources ./dist/srpms
scripts/build-srpm.sh gnoblin ./dist/sources ./dist/srpms
```

Archives include Gnoblin's overlays and patches. They must not be replaced
with unpatched upstream archives.

## Publish to COPR

Configure a Fedora account using the [COPR API page](https://copr.fedorainfracloud.org/api/).
Keep credentials outside the repository.

```sh
scripts/publish-copr.sh OWNER/gnoblin PATH_TO_MUTTER_SRPM PATH_TO_SHELL_SRPM PATH_TO_META_SRPM
```

Replace the owner and paths. The script waits for Mutter before building Shell.
Do not use asynchronous submission that bypasses this dependency.

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

The release workflow builds source archives, source RPMs, Debian/Ubuntu binary
packages and checksums. All three Debian/Ubuntu build and install tests must
pass before assets are published. Asset names and package metadata include both
versions; for example, a Debian package is versioned
`51.0+gnoblin0.1.0-1~debian13`. Dependency sources accompany the binaries.
Manual dispatch can repair assets for an existing SemVer tag. Historical
`v<gnome-version>` tags predate this convention and remain historical releases.

The release workflow then publishes the same source RPMs to COPR and installs
the result in a Fedora 44 container before it completes. The workflow requires
the repository secret `COPR_CONFIG`, containing the publisher's `copr-cli`
configuration. Configure it once before the first automated release:

```sh
gh secret set COPR_CONFIG < ~/.config/copr
```

Debian and Ubuntu packages are added to the signed APT archive after the GitHub
release; Fedora users receive the resulting COPR update through normal `dnf`
updates. Check the completed release workflow before telling users a release is
available.

## Upgrade the GNOME base

`gnome-versions.json` pins the release train.

```sh
./scripts/gnome-versions.py update 52
```

Use the intended major version. The script verifies upstream tags and updates
generated version fields. Then rebase patches, update submodule references and run:

```sh
just check-gnome-version
just verify
```

Patch rebasing and runtime compatibility still require review.

## Release checklist

1. Build all artifacts from the release checkout.
2. Verify private package paths and stock GNOME coexistence.
3. Publish packages and check dependency resolution.
4. Test login, shell startup, portal access and removal on a clean host.
5. Update installation commands and version notes to match published packages.

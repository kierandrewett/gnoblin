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
Debian and Ubuntu use the `gnoblin` package downloaded from GitHub Releases.
There is no APT or pacman repository.

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
just init
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

Push the release commit, then a signed tag matching `gnome-versions.json`.
Use `v51.0` for the base release or `v51.0-1`, `v51.0-2`, etc. for later package
revisions of the same base:

```sh
git tag -s v51.0-1 -m "Gnoblin 51.0, package revision 1"
git push origin v51.0-1
```

The release workflow builds source archives, source RPMs, Debian/Ubuntu binary
packages and checksums. All three Debian/Ubuntu build and install tests must
pass before assets are published. Dependency sources accompany the binaries.
Manual dispatch can repair assets for an existing tag.

This does not publish binary packages to COPR.
Check the release result before telling users assets are available.

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

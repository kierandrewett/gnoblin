# Packaging and releases

For user installation, see [Install Gnoblin](installation.md).
This page is for release maintainers.

The public [platform support](platform-support.md) page distinguishes package
candidates from fully supported session installations. Do not describe a target
as supported until its graphical-session gate has passed.

## Package layout

Gnoblin installs as its own session and can coexist with GNOME.

- RPM names: `gnoblin`, the optional `xdg-desktop-portal-gnoblin`, and the optional
  `gnoblin-geoclue-integration` subpackage.
- The RPM and Arch `gnoblin` packages contain the single `gnoblin` compositor
  executable, login entry, Lua runtime, session services, and Gnoblin-specific
  portal route. They do not require a portal backend or a separate Mutter
  runtime package.
- Private runtime: `/usr/lib/gnoblin`.
- Public files: login entry, control tool, service units and named policy files.
- Private libraries must not satisfy stock GNOME dependencies.
- `gnoblinctl` uses GLib/GIO and JSON-GLib at runtime. Python is a build tool,
  not a base package requirement.

Nix uses separate store outputs. Source builds use their own prefix, `/usr/local/lib/gnoblin` by default.
Bingux owns and releases its shell package separately.

## Package definitions

`packaging/native-packages.json` defines the RPM and Arch package requirements
and their distribution names. Generate or verify the adapters with Python:

```sh
./scripts/sync-package-manifest.py write
./scripts/sync-package-manifest.py check
```

The package check also compares pinned versions with `src/gnome-versions.json` and
`src/gnoblin-version.json`. Nix consumes the same JSON for its optional package
adapter; it is not required to prepare native packages.

Fedora's COPR packages use `gnoblin` as the install entry point. There is
currently no Gnoblin APT or pacman repository.

RPM and Arch login entries launch the lean session directly, as
installing a source build with `make install` does. GNOME Session and
Settings Daemon are not package requirements. The source tarball remains the
primary install route until a distribution package passes its login gate.

The optional `xdg-desktop-portal-gnoblin` package provides Gnoblin's GTK-based backend.
Users can choose another portal instead. Packages do not install
applications. [Use GNOME applications](gnome-apps.md) lists the shared desktop
services to install with your package manager.

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

A release tag starts the release workflow. It runs these stages:

1. **Source.** It builds the source bundle once and uploads it as a workflow artifact.
2. **Packages.** From that bundle, in parallel, it builds the Fedora source and binary RPMs, the
   Arch packages and the openSUSE Tumbleweed RPMs. Each route then installs its packages on a
   clean system, installs stock GNOME beside them and removes Gnoblin.
3. **Publish.** It checksums every asset, records build provenance, attaches everything to one
   release and makes the release public.
4. **COPR.** It builds the Fedora packages in COPR and installs them from there.

Nothing is public until every package job and install check has passed. A failed check leaves no
release behind, and the run keeps the built assets for 30 days.

A push to `main`, a pull request that changes the packaging or the release workflow, and a manual
run without a tag do stages 1 and 2 and publish nothing. They do not run COPR.

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

Gnoblin and GNOME version independently. `src/gnoblin-version.json` is the
canonical Gnoblin [SemVer](https://semver.org/) identity; `src/gnome-versions.json`
continues to pin the compatible GNOME train. For example, Gnoblin `0.1.0` can
target GNOME `51.0` without claiming that it is GNOME version `0.1.0`.

Before release, choose the version deliberately: use a major version
for incompatible Gnoblin configuration or protocol changes, a minor version for
backwards-compatible features, and a patch version for compatible fixes. Then run
the preparation script from a clean `main`:

```sh
./scripts/prepare-release.sh 0.1.11
```

The script does these steps:

- Sets the version in `src/gnoblin-version.json` and `packaging/native-packages.json`.
- Regenerates the package recipes and commits the result.
- Creates the annotated tag `gnoblin-v0.1.11`.

Add `--sign` for a signed tag, `--no-tag` to skip the tag, and `--dry-run` to check the version
without changing anything. The script never pushes. Push the commit first, wait for the Release
workflow to pass on `main`, then push the tag:

```sh
git push origin main
git push origin gnoblin-v0.1.11
```

The release workflow publishes a self-contained Gnoblin source tarball, the Mutter and portal
source archives, source and binary packages, and a checksum file.
The main tarball builds with `make` and does not need Git or submodules.
The Fedora and openSUSE jobs use the component archives inside that
tarball, after checking its SHA-256 sums.

### Release assets

Each release page lists these files. `SHA256SUMS` covers every other file.

| Asset name                                                                    | What it is                                                        | Used by           |
| ----------------------------------------------------------------------------- | ----------------------------------------------------------------- | ----------------- |
| `gnoblin-<version>-gnome-<gnome>-source.tar.xz`                               | Complete source tarball, built with `make`                        | All distributions |
| `mutter-<gnome>.tar.xz`                                                       | Patched Mutter source archive                                     | RPM and Arch      |
| `xdg-desktop-portal-gnome-<gnome>.tar.xz`                                     | Patched portal backend source archive                             | RPM and Arch      |
| `gnoblin-<version>.PKGBUILD`                                                  | Arch recipe for `gnoblin`                                         | Arch              |
| `xdg-desktop-portal-gnoblin-<portal-version>.PKGBUILD`                        | Arch recipe for `xdg-desktop-portal-gnoblin`                      | Arch              |
| `gnoblin-<version>-<pkgrel>-x86_64.pkg.tar.zst`                               | Arch package built from the recipe                                | Arch              |
| `xdg-desktop-portal-gnoblin-<portal-version>-<pkgrel>-x86_64.pkg.tar.zst`     | Arch portal package built from the recipe                         | Arch              |
| `gnoblin-<version>-<release>.fc<N>.src.rpm`                                   | Fedora source RPM for `gnoblin`, built by COPR                    | Fedora            |
| `xdg-desktop-portal-gnoblin-<portal-version>-<release>.fc<N>.src.rpm`         | Fedora source RPM for `xdg-desktop-portal-gnoblin`, built by COPR | Fedora            |
| `gnoblin-<version>-<release>.fc<N>.x86_64.rpm`                                | Fedora binary RPM for `gnoblin`                                   | Fedora            |
| `xdg-desktop-portal-gnoblin-<portal-version>-<release>.fc<N>.x86_64.rpm`      | Fedora binary RPM for `xdg-desktop-portal-gnoblin`                | Fedora            |
| `gnoblin-<version>-<release>.tumbleweed.x86_64.rpm`                           | Binary RPM for `gnoblin`                                          | openSUSE          |
| `xdg-desktop-portal-gnoblin-<portal-version>-<release>.tumbleweed.x86_64.rpm` | Binary RPM for `xdg-desktop-portal-gnoblin`                       | openSUSE          |
| `SHA256SUMS`                                                                  | SHA-256 checksums of all other assets                             | All distributions |

`<gnome>` and `<portal-version>` are the pinned GNOME version, for example
`51.0`. `<pkgrel>` and `<release>` are separate packaging counters for Arch and
RPM, so they do not match each other.

A release tag creates a versioned release. A manual run with a tag publishes that tag, or repairs a
release that is still a draft. It refuses to change a release that is already public. To repair a
public release, turn it back into a draft first with `gh release edit TAG --draft`.

`v<gnome-version>` tags predate this convention and remain historical releases.

The publish job refuses to start unless the release has the source bundle and at least one package
of each kind, and unless `SHA256SUMS` lists exactly the other files. It uploads to a draft release
and makes the release public as its last step, so the release page never shows a half-filled
release.

If a job fails, for example because its distribution repository lacks a required dependency
version, nothing is published. Fix the cause and run the workflow again for the same tag.

Each asset has build provenance recorded by GitHub. To check a download:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify FILE --repo kierandrewett/gnoblin
```

The release notes list the merged pull requests, grouped by label. `.github/release.yml` defines the
groups.

The release workflow installs the Fedora RPMs it built on a clean Fedora system, with and without
the optional portal package, then installs stock GNOME beside them and removes Gnoblin.
The COPR job runs the same checks, with `scripts/check-fedora-install.sh`, against the published
COPR package. The openSUSE and Arch jobs check the same installation order from their built packages.

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

`src/gnome-versions.json` pins the release train.

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

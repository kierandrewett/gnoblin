# Packaging and releases

For user installation, see [Install Gnoblin](../docs/installation.md).
This page is for release maintainers.

The public [platform support](../docs/platform-support.md) page distinguishes package
candidates from fully supported session installations. Do not describe a target
as supported until its graphical-session gate has passed.

## Package layout

Gnoblin installs as its own session and can coexist with GNOME. Four packages come from one build:

| Package                       | Contents                                                               | Optional |
| ----------------------------- | ---------------------------------------------------------------------- | -------- |
| `gnoblin`                     | Compositor, Lua runtime, session services, private Mutter, login entry | No       |
| `xdg-desktop-portal-gnoblin`  | Gnoblin's GTK-based portal backend                                     | Yes      |
| `gnoblin-geoclue-integration` | The GeoClue agent authorization file                                   | Yes      |
| `gnoblin-ananicy-rules`       | The `ananicy-cpp` rules for the Gnoblin processes                      | Yes      |

- The private runtime is `/usr/lib/gnoblin`. Public files are the login entry, `gnoblin` and
  `gnoblinctl` links, service units and named policy files.
- `gnoblin` does not require a portal backend or a separate Mutter runtime package. It provides
  `gnoblin-session` and replaces `gnoblin-session`, `gnoblin-shell`, `gnoblin-mutter` and
  `gnoblin-mutter-devel`, so systems with the old packages upgrade.
- Private libraries must not satisfy stock GNOME dependencies. `gnoblinctl` needs GLib and
  JSON-GLib at runtime. Python is a build tool.
- The four packages have the same version, the Gnoblin version in `src/gnoblin-version.json`,
  and epoch 1. The portal backend keeps the version of its GNOME source (`51.0.0`). Its release
  number follows the Gnoblin release, so each release has a distinct portal package.

Nix uses separate store outputs. Source builds use their own prefix, `/usr/local/lib/gnoblin` by default.
Bingux owns and releases its shell package separately.

## How the packages are made

Nothing is built by a distribution recipe. One build serves every package format:

1. `scripts/build-prefix.sh` runs `./build.sh --layout system` on Fedora and writes a prefix, a
   folder laid out like an installed system. It links Lua statically and puts libraries in `lib`,
   because Fedora, openSUSE, Arch and Debian name the Lua library and the library directory
   differently.
2. `scripts/build-packages.sh` runs [pkgsmith](https://github.com/kierandrewett/pkgsmith) on the
   prefix with the definitions in `pkgsmith/`. It reads the architecture, the libraries the
   programs need and the glibc floor from the prefix files.
3. `scripts/check-package-install.sh` installs the packages on a clean system, installs stock GNOME
   beside them and removes Gnoblin.

To try it on a Fedora system with Rust installed, where `43` is that system's Fedora release:

```sh
./scripts/install-fedora-build-deps.sh
./scripts/build-prefix.sh --prefix prefix
PATH="$(./scripts/install-pkgsmith.sh):$PATH" ./scripts/build-packages.sh --prefix prefix --output dist --fedora 43
```

These files make up the definitions:

- `scripts/pkgsmith.ref` pins the pkgsmith commit.
- `pkgsmith/runtime.star` writes `gnoblin` and the portal backend. It lists the portal's files, so
  everything else in the prefix belongs to `gnoblin`.
- `pkgsmith/integrations.star` writes the two configuration packages.
- `pkgsmith/dependencies.star` maps each library to the Debian and Arch package that ships it.

The build stops when a program needs a library that is not listed. Run
`python3 tests/pkgsmith-packages.test.py` after you change these files.

| Format      | File                                  | Systems                                |
| ----------- | ------------------------------------- | -------------------------------------- |
| `rpm`       | `gnoblin-<v>-1.fc<N>.x86_64.rpm`      | Fedora release `<N>`, one file each    |
| `rpm`       | `gnoblin-<v>-1.tumbleweed.x86_64.rpm` | openSUSE Tumbleweed                    |
| `deb`       | `gnoblin_<v>-1_amd64.deb`             | Debian testing (`forky`)               |
| `archlinux` | `gnoblin-1:<v>-1-x86_64.pkg.tar.zst`  | Arch Linux                             |
| `apk`       | `gnoblin-<v>-r1.apk`                  | Alpine edge                            |
| `tar`       | `gnoblin-<v>-amd64.tar.xz`            | Other glibc systems, extract below `/` |

CI builds amd64 and arm64. The Fedora and openSUSE rpm files differ only in dependency names. The
glibc floor and the library names come from the programs of the prefix, so each format asks for
what its prefix needs.

### One prefix for each Fedora release

`.github/workflows/packages.yml` builds a prefix on every Fedora release that
`scripts/package-targets.json` lists with status `existing-path` (`scripts/fedora-releases.py`
reads the list), and writes the Fedora rpm of that release from its prefix with
`scripts/build-packages.sh --lanes fedora --fedora <N>`. The rpm of Fedora 43 therefore asks for
the glibc and the library sonames of Fedora 43. A library that only an older Fedora release has
under that soname, such as `libdisplay-info.so.2` on Fedora 43, is allowed in the Fedora rpm
alone (`FEDORA_ONLY` in `pkgsmith/dependencies.star`).

The openSUSE rpm, the deb, Arch and tar packages run on systems that are newer than Fedora, so
they come from one prefix whose library sonames those systems have: the release marked
`"shared_prefix": true` in the inventory (`--lanes other`). Fedora 43 links `libdisplay-info.so.2`,
which Debian, Arch and Tumbleweed do not ship, so it cannot be that release. The shared prefix is
never a pre-release, so those packages do not wait on one.

A release that is not out yet has lifecycle `upstream-pre-release` in the inventory. Its jobs have
`continue-on-error`, so a failure there is visible and does not fail the run.

### Choosing the releases

`scripts/update-release-matrix.py` proposes the releases; a person decides. It compares
endoflife.date, Fedora's own release data (Bodhi and the published release directory) and the
`fedora:<N>` image tag, and writes `scripts/release-candidates.json`: the newest four releases
that are out and the pre-release, each with whether it meets the build floors. A release on which
the three sources disagree goes to `needs_review` with its three observations and is never a
candidate. `.github/workflows/release-matrix.yml` runs it weekly, checks the floors in each
image, and opens one issue with the diff. It never pushes. To build a candidate, add or edit its
entry in `scripts/package-targets.json` and set `status` to `existing-path`. Its `support` stays
`unsupported` or `candidate` until its gates pass, as `scripts/check-packaging-targets.py`
enforces, and that script warns about a candidate without an entry and an entry past its end of
life.

### The apk packages

A prefix built on Fedora links glibc and does not run on Alpine, which uses musl. The apk packages
therefore come from a second prefix that `packages.yml` builds in `alpine:edge` with the same
`scripts/build-prefix.sh`. `scripts/install-alpine-build-deps.sh` installs its build requirements.

- Alpine edge is the only target, because it is the only Alpine release with GNOME 51 libraries.
- Dependencies name each library as `so:<soname>`, which `apk` resolves itself.
- apk has no epoch, so an apk is ordered by version and release only.
- The apk packages are release assets and are in the signed repositories. `pkgrepo` writes one
  `APKINDEX.tar.gz` for each architecture, signed with `REPO_APK_PRIVATE_KEY`, under
  `alpine/edge/<arch>/` of the channel.

A glibc prefix makes no apk, and a musl prefix makes only apk packages.

### Upgrading from the old packages

Package names, the Fedora epoch and the `Provides` and `Obsoletes` entries are those of the old
Fedora and Arch packages, and the new version is higher, so `dnf`, `zypper` and `pacman` upgrade
them.

Fedora packages came from COPR before. The COPR project `kierandrewett/gnoblin` keeps its old
builds, but releases no longer go there: COPR builds from a source RPM, and pkgsmith writes
binary packages only. Switch to the [package repositories](../docs/package-repositories.md) or the
release assets.

## Prepare the source tarball

From a clean release checkout, build the source assets first:

```sh
./build.sh package --output ./dist/release
```

`--output` sets the directory, which must be empty. `--release-tag TAG` checks `TAG` against the
version in the tree, for a release build.

The `gnoblin-*-source.tar.xz` file contains Gnoblin and its patched, pinned
Mutter and portal sources. Extract it and run `make` to check the
same source route users receive. The command also writes component archives,
an Arch recipe, and checksums into `dist/release`.

A release tag starts the release workflow. It runs these stages:

1. **Source.** It builds the source bundle once and uploads it as a workflow artifact.
2. **Packages.** From a checkout of the same ref, for amd64 and arm64, it builds a prefix on
   Fedora and another on Alpine, writes the packages with pkgsmith, installs them on clean Fedora,
   openSUSE, Arch, Debian and Alpine systems, installs stock GNOME beside them and removes Gnoblin.
3. **Publish.** It checksums every asset, records build provenance, attaches everything to one
   release and makes the release public.
4. **Repository.** It builds the signed repository from the packages and uploads it. See
   [package repositories](../docs/package-repositories.md).

Nothing is public until every package job and install check has passed. A failed check leaves no
release behind, and the run keeps the built assets for 30 days.

A push to `main`, a pull request that changes the packaging or the release workflow, and a manual
run without a tag do stages 1 and 2 and publish nothing.

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

- Sets the version in `src/gnoblin-version.json`, the one place that holds it, and commits it.
- Creates the annotated tag `gnoblin-v0.1.11`.

Add `--sign` for a signed tag, `--no-tag` to skip the tag, and `--dry-run` to check the version
without changing anything. The script never pushes. Push the commit first, wait for the Release
workflow to pass on `main`, then push the tag:

```sh
git push origin main
git push origin gnoblin-v0.1.11
```

The release workflow publishes a self-contained Gnoblin source tarball, the Mutter and portal
source archives, the packages and a checksum file.
The main tarball builds with `make` and does not need Git or submodules. The package build uses
that tarball, after checking its SHA-256 sums.

### Release assets

Each release page lists these files for each architecture. `SHA256SUMS` covers every other file.

| Asset name                                      | What it is                                   |
| ----------------------------------------------- | -------------------------------------------- |
| `gnoblin-<version>-gnome-<gnome>-source.tar.xz` | Complete source tarball, built with `make`   |
| `mutter-<gnome>.tar.xz`                         | Patched Mutter source archive                |
| `xdg-desktop-portal-gnome-<gnome>.tar.xz`       | Patched portal backend source archive        |
| `<package>-<version>-1.fc<N>.<arch>.rpm`        | Fedora RPM                                   |
| `<package>-<version>-1.tumbleweed.<arch>.rpm`   | openSUSE Tumbleweed RPM                      |
| `<package>_<version>-1_<arch>.deb`              | Debian package                               |
| `<package>-1.<version>-1-<arch>.pkg.tar.zst`    | Arch package, with the epoch written as `1.` |
| `<package>-<version>-<arch>.tar.xz`             | Files of the package, for any glibc system   |
| `SHA256SUMS`                                    | SHA-256 checksums of all other assets        |

`<package>` is one of the four package names. `<gnome>` is the pinned GNOME version, for example
`51.0`. GitHub does not keep the colon of an Arch file name that has an epoch, so the workflow writes it as a dot.
The Arch epoch belongs to the package, which pacman reads from the file, not from its name.

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

The release workflow installs the packages it built on clean Fedora, openSUSE, Arch and Debian
systems with `scripts/check-package-install.sh`. The script installs `gnoblin`, runs it, adds the
portal backend and the two configuration packages, installs stock GNOME beside them and removes
Gnoblin. Stock GNOME must be unchanged afterwards, and no Gnoblin path may remain.

The signed repositories need these repository secrets and variables. `package-repository.yml`
lists them, and `design/package-repositories.md` explains each one.

| Name                                          | Kind     | Purpose                            |
| --------------------------------------------- | -------- | ---------------------------------- |
| `REPO_GPG_PRIVATE_KEY`, `REPO_GPG_PASSPHRASE` | secret   | Sign the packages and the metadata |
| `REPO_APK_PRIVATE_KEY`                        | secret   | Sign the Alpine package index      |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`  | secret   | Upload to the R2 bucket            |
| `REPO_BACKEND`, `REPO_DESTINATION`            | variable | `s3` and `s3://gnoblin-repo`       |
| `AWS_ENDPOINT_URL`                            | variable | The R2 S3 endpoint                 |

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

## Build a package tree

A distribution package uses the same build. It adds `--layout system` and a staging directory.

`--layout` takes two values:

- `private` is the default. Every file goes below `--prefix`. Use it for a
  development build, `--preview` and `--register-session`.
- `system` also writes the files a package ships outside the private prefix, below
  `--system-prefix` (default `/usr`). `cmake/system-layout.cmake` makes them:
    - links to `gnoblin` and `gnoblinctl` in `bin`
    - the session file, the systemd user units and the portal files
    - the polkit action

`--destdir DIR` installs below `DIR`, as a package build root. Nothing outside `DIR`
changes.

```sh
./build.sh --layout system --prefix /usr/lib/gnoblin --destdir "$PWD/stage"
```

The runtime is now in `stage/usr/lib/gnoblin/`, and `stage/usr/bin/gnoblin` links to
it. Copy `stage/` into the package root. Add `--without-portal` for a package that
ships the portal backend separately. To check the result against the RPM file lists,
run `tests/system-layout.test.sh --stage stage`.

`--system-prefix` needs `--layout system`. `--preview` and `--register-session` use
the private layout. Set `GNOBLIN_BUILD_DIR` to build in a directory other than
`build/ninja`, so a package build can sit next to a development build.

# Release packaging status and procedure

This is internal release engineering material. Keep it current whenever the
release pipeline or COPR publication changes.

## Current state (2026-09-24)

- `gnome-versions.json` pins the private runtime to GNOME/Mutter/Shell 51.0.
- The latest COPR release is Gnoblin 0.1.4, build 11021823, with Mutter
  51.0-20 and Shell 51.0-14. The project currently enables only
  `fedora-44-x86_64`.
- The running host is Fedora 43 and has Fedora 43 Gnoblin 49.6-era packages.
- The source-build and COPR-install jobs in `.github/workflows/verify.yml` and
  the source-RPM/COPR release jobs in `.github/workflows/release.yml` and
  `.github/workflows/copr.yml` currently use Fedora 44.
- The GitHub source tree can be newer than the latest tagged COPR release.
  Check the latest release tag and COPR build before describing an installed
  package as current.

## Release flow

1. Set the intended GNOME source pins in `gnome-versions.json` and the
   independent Gnoblin version in `gnoblin-version.json`.
2. Build reproducible source archives and SRPMs with
   `scripts/build-release-assets.sh`; release automation downloads upstream
   tags and validates that the release tag matches the source commit.
3. Publish the release assets to GitHub. The COPR workflow submits schemas,
   Mutter, Shell, then the metapackage so dependencies are available in order.
4. COPR builds each SRPM in every enabled project chroot. The workflow then
   installs the published metapackage in a clean Fedora container and verifies
   the session files and executables.
5. Record successful COPR build IDs and package NVRs, then verify a fresh
   installed session separately from the build result.

## Fedora compatibility checks

Adding an older Fedora target requires all of the following:

- Add that Fedora release to the clean source-build matrix.
- Enable its x86_64 chroot in the COPR project and confirm the repository
  metadata publishes packages for that release.
- Build all source RPMs in that chroot; resolve actual compiler, API, and
  dependency failures instead of changing the release label.
- Check RPM `Requires` against the target's repositories. The Fedora 43 host
  currently reports PipeWire 1.4.11, libinput 1.30.3, and libwayland-client
  1.25.0; current package metadata asks for 1.6.0, 1.31.0, and 1.26.
- Lower a dependency floor only after compiling against that library version
  and checking the symbols/APIs the built runtime uses. Keep shared package
  manifest requirements valid for every supported distribution.
- Install the built COPR packages in a clean target-release container. Then
  verify a fresh graphical session on suitable hardware before marking the
  release usable.

The current Fedora 43 work starts by adding a Fedora 43 source-build job. The
runtime package floors and COPR install job must be updated only after that
build establishes which APIs the pinned GNOME 51 sources use.

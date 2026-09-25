# Release packaging status and procedure

This is internal release engineering material. Keep it current whenever the
release pipeline or COPR publication changes.

## Current state (2026-09-24)

- `gnome-versions.json` pins the private runtime to GNOME/Mutter/Shell 51.0.
- The latest COPR release is Gnoblin 0.1.4, build 11021823, with Mutter
  51.0-20 and Shell 51.0-14. The project currently enables
  `fedora-43-x86_64` and `fedora-44-x86_64`; no Fedora 43 RPM build has been
  published yet.
- The running host is Fedora 43 and has Fedora 43 Gnoblin 49.6-era packages.
- The source-build job in `.github/workflows/verify.yml` covers Fedora 43 and 44. The initial matrix run exposed a stale GNOME Shell patch hunk and is
  being corrected. The source-RPM job uses Fedora 44; COPR compiles SRPMs in
  each enabled chroot.
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
- Check RPM `Requires` against the target's repositories. Fedora 43 updates
  currently provide PipeWire 1.4.11, libinput 1.30.3, and Wayland 1.26.
- Lower a dependency floor only after compiling against that library version
  and checking the symbols/APIs the built runtime uses. Keep shared package
  manifest requirements valid for every supported distribution.
- Install the built COPR packages in a clean target-release container. Then
  verify a fresh graphical session on suitable hardware before marking the
  release usable.

The Fedora 43 work enables the COPR chroot and adds clean source-build and
post-publication package-install matrices. Mutter 51 calls the libinput 1.31
DWT-timeout API; this setting is now compiled conditionally so older libinput
retains normal disable-while-typing behavior.

The shared package manifest floors are set to PipeWire 1.4 and libinput 1.30.
The COPR build/install run is still required to prove RPM build and runtime
compatibility on Fedora 43.

The first Fedora 43/44 source matrix run reached Shell patch application and
failed because the session-lock patch had malformed unified-diff context and
its new resource entry made the notification patch stale. Both patches now
apply in sequence.

On commit `25af4af`, Fedora 43 completed the full source build successfully.
Fedora 44 checkout hit a transient upstream GitLab 503 before compilation;
rerun that job. The Fedora 43 COPR package build and clean install remain the
release compatibility gates.

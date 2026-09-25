# Release packaging status and procedure

This is internal release engineering material. Keep it current whenever the
release pipeline or COPR publication changes.

## Current state (2026-09-25)

- `gnome-versions.json` pins the private runtime to GNOME/Mutter/Shell 51.0.
- The latest COPR release is Gnoblin 0.1.4, build 11021823, with Mutter
  51.0-20 and Shell 51.0-14. No Fedora 43 RPM build has been published yet.
- The running host is Fedora 43 and has Fedora 43 Gnoblin 49.6-era packages.
- Fedora support policy is the three newest Fedora releases. The clean source
  build and post-publication COPR install gates cover Fedora 43, 44, and 45;
  Fedora 43 remains the oldest supported target while it is in this set.
- COPR has Fedora 43, 44, and 45 x86_64 chroots enabled. The existing
  published build has no Fedora 45 packages yet; the 0.1.7 release must build
  and pass clean installs in all three chroots.
- The current source candidate is Gnoblin 0.1.7 on GNOME 51.0. The
  `gnoblin-v0.1.7` tag points at `682a8f5`, which is also on `main`.
- Release run `36132144715` built all packages and passed Debian 13 and Ubuntu
  26.04 install tests. Ubuntu 24.04 failed because the package declared
  `gir1.2-gtk4layershell-1.0`, which Ubuntu 24.04 does not provide. This was a
  fixed service dependency; no Gnoblin source uses GTK4LayerShell.
- Commit `682a8f5` removes that unavailable dependency and adds a regression
  assertion. Release run `36136202878` was cancelled during the package builds
  before install or publication, so the candidate did not reach GitHub, APT, or
  COPR.
- A packaging audit found the generated RPM and Arch package metadata pointed
  at a deleted GitHub owner (`kdrew7`). Commit `e1472bb8` fixes the shared
  generator and both generated outputs. The old URL returned HTTP 404; the
  canonical URL returned HTTP 200.
- The requested distro scope now includes Fedora/EL, Debian/Ubuntu, Arch,
  openSUSE, and NixOS. The family-by-family gap analysis and researched
  implementation direction are in `design/packaging-research.md`. Do not
  republish 0.1.7 until its target matrix, package recipes, and GNOME
  co-install/remove gates have been brought into line with that scope.
- Commits `bcbc5d46`, `2d367232`, and `d847369a` begin that work: Fedora's
  COPR smoke test now installs stock GNOME first and verifies ownership before
  and after Gnoblin install/removal; `packaging/targets.json` records 21 fixed
  and rolling x86_64 targets across the requested families; and the research
  log records disposable-image compatibility probes. The inventory validator
  runs in `.github/workflows/packaging-targets.yml` and keeps every target
  unsupported or candidate until all evidence gates pass.
- Probe results rule out unchanged package recipes as a solution for the older
  targets. EL 8/9/10 and openSUSE Leap 15.6/16.0 miss GNOME 51 host GLib/GJS
  floors; Debian 11/12 and Ubuntu 22.04 lack build/runtime pieces absent from
  the private bundle. Tumbleweed now has SUSE-native specs and a remote
  end-to-end RPM build/coinstall/removal workflow. Arch has a generated native
  package recipe plus both continuous and pre-release build/coinstall/removal
  gates. Neither adapter has a successful full package run yet; keep these
  targets unsupported until their gates pass.
- Fedora workflow run `36139101542` passed the first RPM-side stock-GNOME
  install/coexist/remove gate, along with Fedora 43/44/45 builds and the
  existing Arch source-build/dependency checks. The RPM gate does not prove
  graphical login or session selection.
- Commit `6e377de6` replaces the broken Arch placeholder with a real
  source-build PKGBUILD and a deterministic release source bundle containing
  Gnoblin plus materialised patched schemas, Mutter, and Shell sources.
  Commit `25cd6d54` makes the release recipe generator independent of Nix.
  Commit `7c1bfa15` adds a pre-publication `makepkg`, stock-GNOME co-install,
  and Gnoblin removal gate and publishes the resulting Arch package asset.
  Bundle determinism, source inventory, release checksum generation, and
  recipe syntax pass; the new release gate has not yet run for a tag, and
  graphical login remains unverified.
- Commits `6ea70e32` and `03ed5012` add pinned NixOS 25.05, 25.11, 26.05, and
  unstable package/module evaluations and use each channel's compiler ABI
  consistently, falling back to `stdenv` when `gcc16Stdenv` is absent. All
  four channel evaluations pass. 25.05 lacks libglycin and misses several
  host dependency floors; 25.11 and 26.05 evaluate but are blocked at the
  required Wayland 1.26 floor (and 25.11 also misses Wayland Protocols and
  libinput floors). Unstable has no recorded host-floor blocker. Evaluation
  does not establish a package build, graphical session, or coexistence.
- Commits `76d82d9e`, `ea85af0c`, and `a53afd86` make openSUSE Tumbleweed
  dependency provisioning work without an assumed `busybox-gawk`, add native
  RPM specs/workflow, and correct SUSE runtime library names. `rpmspec` parses
  the specs and Zypper resolves external build and host requirements. The
  remote build then exposed missing upstream tags, container Git trust and
  committer identity, and omitted build tools (`inkscape`, `hyprcursor-util`);
  each is fixed in the workflow. Re-run its full build/coinstall/removal gate
  before changing the target inventory.
- Commits `03562b7a` and `508c1f7e` add a repeatable Debian 12 / Ubuntu 22.04
  capability probe and run it on pushes and pull requests. Both images are
  blocked by GTK below Mutter 51's 4.14 floor and missing GIRepository 2,
  GCR4, libei/eis, libdisplay-info, Glycin, and Hyprcursor interfaces. The
  probe emits a JSON artifact and leaves both targets unsupported; adding them
  to the package build matrix requires a deliberate private-runtime extension.
- The target inventory now reflects the automated Arch source-bundle/release
  recipe, Nix channel evaluations, and Tumbleweed dependency-resolution path.
  These are implementation paths, not support claims. Check
  `packaging/targets.json` before describing per-distro status.
- Manual DEB workflow run `36143640335` completed private package builds and
  clean install/coinstall/removal checks for Debian 13, Ubuntu 24.04, and
  Ubuntu 26.04. Debian 12 and Ubuntu 22.04 remain probe-only due to missing
  host runtime interfaces. The newer DEB targets remain candidates: no
  graphical login gate has run.
- A real Arch `makepkg` run caught that the generated recipe expanded `$srcdir`
  before `makepkg` initialized it. The generator now computes that private
  build path inside the build/package functions. The builder also installs
  `brightnessctl`, and the Tumbleweed source builder installs the Hyprcursor
  utility. The new exact-main package runs must still pass before recording
  Arch or Tumbleweed package gates.
- Commit `4cb52603` changed Nix full-build concurrency to preserve a long
  package build when later commits are pushed. Stable Nix channels still have
  dependency-floor blockers and do not have installable channel-specific
  package outputs; successful evaluation is not NixOS release support.
- The first Tumbleweed SRPM reached `%build` but exposed the RPM Meson helper
  resolving its executable under `/usr/lib/gnoblin`. Commit `0614f077` replaces
  the helper with explicit host Meson commands and explicit private install
  directories. The new full RPM/coinstall/removal workflow must pass before
  updating target gate values.
- Commit `a55f4069` stages GNOME 51 schemas in Arch's temporary build prefix
  before Mutter configuration and replaces the Tumbleweed schema spec's
  private-prefix Meson lookup. Run `36148646285` then progressed through the
  schema RPM and failed configuring Mutter because `pkgconfig(udev)` was not
  required. The current spec fix adds that capability; package-chain and
  stock-GNOME install/removal evidence still need a successful exact-main run.
- The earlier install failure in run `36074745709` was caused by
  `next.cursor` being undefined while reloading a partial config. Commits
  `752d016` and `3daf6dc` added default cursor values, validation, and
  Mutter's live cursor preference setter; a later full Debian/Ubuntu install
  run passed that config reload smoke test.
- `COPYING` credits Gnoblin's original code to Working Directory Ltd. and
  preserves separate GNOME author/contributor attribution. Debian metadata
  includes both notices and the bundled Adwaita cursor attribution; RPM
  packages include the repository license file.
- Workflow `36079736824` passed package builds and clean install tests on
  Debian 13, Ubuntu 24.04, and Ubuntu 26.04, including the config reload smoke
  test. This verifies the Debian/Ubuntu package path; it does not verify an
  RPM build, COPR publication, or a Fedora 43 host install.
- The source-build job in `.github/workflows/verify.yml` covers Fedora 43, 44,
  and 45. COPR compiles SRPMs in each enabled chroot.
- The GitHub source tree can be newer than the latest tagged COPR release.
  Check the latest release tag and COPR build before describing an installed
  package as current.

- Commits `bf30386b` and `fa9b3b94` corrected the Tumbleweed udev requirement and raised the Fedora Mutter/Shell build dependency declarations to match the pinned GNOME 51 source floors. A fresh Tumbleweed CI build then exposed missing `argcomplete`; that is now declared in the native SUSE spec. The exact-main build/coinstall/removal workflow has not yet verified these fixes.
- The current packaging audit adds automated RPM repository probes for Rocky 8/9/10 and openSUSE Leap 15.6/16.0/Tumbleweed. The probe separates five host runtime floors from build-only API requirements and the Mutter development package's Wayland Protocols floor. Rocky and Leap are expected to remain blocked by their repository-provided versions; Tumbleweed is expected to clear package-build floors. Probe readiness alone does not establish build, installation, co-installation, or graphical-session support.
- Arch's generated `PKGBUILD` now stages the privately built Mutter into a temporary build prefix before configuring Shell. It feeds staged pkg-config, GIR, and typelib paths to Shell and checks that headers and libraries resolve from that private stage. This addresses the exact `mutter-clutter-51` configure failure from Verify run `36148646223`; the corrected full Arch package run is still required.
- Stable NixOS channel package attributes now fail early with recorded dependency blockers instead of implying support through `nixpkgs-unstable`. The 25.05, 25.11, and 26.05 channels remain unsupported for GNOME 51 pending viable dependency/runtime boundaries and package/install/coexistence evidence.

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
post-publication package-install matrices. Fedora 43, 44, and 45 are the
rolling support set. Mutter 51 calls the libinput 1.31 DWT-timeout API; this
setting is now compiled conditionally so older libinput retains normal
disable-while-typing behavior.

The shared package manifest floors are set to PipeWire 1.4 and libinput 1.30.
The COPR build/install run is still required to prove RPM build and runtime
compatibility on Fedora 43, 44, and 45. Broader distribution-family targets
and clean stock-GNOME coexistence tests must be added before the `0.1.7` release
workflow is restarted.

The first Fedora 43/44 source matrix run reached Shell patch application and
failed because the session-lock patch had malformed unified-diff context and
its new resource entry made the notification patch stale. Both patches now
apply in sequence. The Fedora 43 COPR package build and clean install remain
the release compatibility gates.

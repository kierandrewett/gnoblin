# Release packaging status and procedure

This is internal release engineering material. Keep it current whenever the
release pipeline or COPR publication changes.

## Candidate release validation (2026-09-26)

- Release run `36236893302` passed the package builds and install checks for
  Debian 13, Ubuntu 24.04/26.04, Arch and openSUSE Tumbleweed. APT refused to
  replace Debian revision `-4` because its published checksum differs from the
  rebuilt package; the next release uses `-5`. COPR built Mutter successfully
  for Fedora 43/44/45, then exposed a missing `gcc-c++` build requirement for
  private GJS in the Shell package. Shell RPM release `51.0-22` adds it.

- Release run `36230307572` built Debian 13, Ubuntu 24.04/26.04, Arch,
  openSUSE Tumbleweed, and NixOS 26.05 artifacts. Their package installation,
  GNOME co-installation, and removal gates passed. COPR Mutter build `11038026`
  passed Fedora 44/45 and failed Fedora 43 because PipeWire 1.4 lacks the
  capability/HDR APIs and device-ID SPA property used by Mutter 51.
- The compatibility patch now keeps common tag parameters outside the optional
  capability code, uses numeric logging for color enums missing in PipeWire
  1.4, and gates device ID separately at PipeWire 1.6. The next COPR Mutter
  release is `51.0-27.gnoblin`; revisions `-1` through `-4` of the 0.1.7 DEB
  are already published and immutable. The corrected package must use `-5`.
- APT suite indexes were corrected and republished by `36232880003` at
  `gh-pages` commit `f95cc80e`. The index contents now match each Ubuntu suite;
  confirm the newest Pages build and live custom domain before treating the
  APT repository as available.
- `gnoblin.org` is served by the Cloudflare Pages VitePress project, not the
  legacy GitHub Pages build. Include the `gh-pages` APT archive in that deploy.
  Cloudflare Pages rejects files over 25 MiB, so publish the small signed
  indexes/keyring and redirect each indexed `.deb` path to that release's
  GitHub asset. Index only the release version being published so redirects
  never point at an asset removed from the current release draft.

- The Wayland config fix is validated by DEB builds and installed reload/window
  smoke checks on Debian 13 and Ubuntu 24.04/26.04 in release runs
  `36218054872` and `36220527312`.
- Run `36218054872` found two Arch release-gate defects: the runner omitted the
  declared `brightnessctl` dependency, and its package selector chose the
  `gnoblin-debug` split. Both are fixed in `main`; run `36220527312` passed the
  Arch build and stock-GNOME co-install/removal check.
- Run `36220527312` created the GitHub Release draft and published the APT
  repository, but its COPR job failed while resolving
  `pkgconfig(hyprcursor) >= 0.1.13` in Fedora 43/44/45. The Gnoblin Mutter
  patch itself requires Hyprcursor 0.1.11; the RPM floor has been corrected to
  that patch's API requirement.
- Run `36223273109` passed the Fedora schema build, then failed the Mutter
  build because Mutter 51 includes `pipewire/capabilities.h`, which is absent
  from Fedora 43's PipeWire 1.4.11. Lowering only the dependency floor was not
  sufficient. The compatibility patch now compiles device-ID negotiation only
  when PipeWire 1.5.84 or newer provides those APIs; the 1.4 path retains normal
  screen casting without that optional negotiation. Fedora and openSUSE RPM
  declarations now use the 1.4 API floor. Rebuild all three COPR chroots
  before claiming Fedora 43 package availability.
- The next Mutter RPM release is 51.0-25.gnoblin, and the generated RPM
  metapackage now requires that exact build. Keep the release manifest and
  metapackage synchronized whenever the Mutter RPM release changes.
- That run's APT job correctly refused different bytes for the already
  published 0.1.7 package versions. The exact immutable Debian/Ubuntu package
  payloads were restored to the GitHub release draft and its checksums updated;
  rerunning only the APT job then passed without changing those packages.
- Since the corrected Mutter source changes the 0.1.7 build, the next release
  workflow run uses Debian package revision `-2`; revision `-1` remains
  immutable. The `gnoblin-v0.1.7` tag must point at the corrected release commit
  before rerunning the full Release workflow.
- Release run `36226484341` was cancelled after Nix found that the first
  compatibility patch used no-context hunks that landed at the wrong locations
  after earlier Mutter patches shifted the source. The patch now anchors its
  edits to the PipeWire dependency and affected functions; rerun the source and
  Nix builds before allowing publication.
- Release run `36226790588` passed the source, Nix, openSUSE, Arch, and all three
  Debian/Ubuntu build/install gates. Its APT publication passed with Debian
  revision `-2`. COPR initially stopped at source-RPM selection because the
  draft still contained the obsolete Mutter 51.0-24 asset beside 51.0-25. The
  stale asset was removed and the COPR job rerun. The release workflow now
  removes draft assets absent from the fresh build before upload, preventing
  this repair failure on later releases.
- COPR build `11037820` then exposed three more PipeWire 1.5-only headers absent
  from Fedora 43's PipeWire 1.4.11: `spa/param/dict.h`, `dict-utils.h`, and
  `peer-utils.h`. The compatibility patch now guards those includes with the
  same capability check. Since Debian revisions `-1` and `-2` are published,
  the next release run uses Debian revision `-3`; the matching Mutter RPM
  release is `51.0-26.gnoblin`.
- The APT revision `-3` publication revealed that each Ubuntu suite index
  included both Ubuntu builds from the shared pool. Filter each index by its
  version suffix and republish the archive before treating APT availability
  as correct. The release workflow now tests the suite-specific index rule.
- The current family scope has distinct remaining work: Debian 12 and Ubuntu
  22.04 have only a private dependency-closure build; EL 8/9/10 and openSUSE
  Leap have no viable host dependency floor for the GNOME 51 packages; Arch,
  Tumbleweed, and NixOS 26.05 still need graphical-session acceptance. Keep
  those states aligned with `packaging/targets.json` and
  `docs/platform-support.md`.

## Release workflow coverage added 2026-09-26

- The tagged Release workflow now builds the openSUSE Tumbleweed RPM chain from
  the release ref, runs its stock-GNOME co-install/removal checks, and attaches
  the resulting RPMs to the GitHub release. This is a direct RPM download, not
  an OBS repository.
- The tagged Release workflow calls the Nix workflow against the same ref and
  builds the pinned NixOS 26.05 package before GitHub release publication. The
  Nix distribution path remains the tagged flake source; this does not create a
  binary cache or prove graphical login.
- EL 8/9/10 remain without a release package path. The RPM capability probes
  show that the unchanged host dependency contract does not resolve there. A
  private dependency closure and EL package adapter are still required before
  an EL artifact can be built honestly.
- These paths are implemented but are not release evidence until a tagged
  Release workflow has completed and the assets or flake build are verified.

## Current state (2026-09-25)

- `gnome-versions.json` pins the private runtime to GNOME/Mutter/Shell 51.0.
- The latest COPR release is Gnoblin 0.1.4, build 11021823, with Mutter
  51.0-20 and Shell 51.0-14. No Fedora 43 RPM build has been published yet.
- The running host is Fedora 43 and has Fedora 43 Gnoblin 49.6-era packages.
- Fedora support policy is the three newest Fedora releases. Clean source
  builds cover Fedora 43, 44, and 45. Only Fedora 44 has a recorded
  package-install, stock-GNOME coexistence, and removal transaction, so it is
  the sole Fedora candidate; none has passed a graphical-session gate.
- COPR has Fedora 43, 44, and 45 x86_64 chroots enabled. The existing
  published build has no Fedora 43 or 45 packages. Inspection of COPR's public
  build API showed that `copr-cli build` received no `--chroot` arguments, so
  the `.fc44` source RPM built only in Fedora 44 despite all three project
  chroots being enabled. `scripts/publish-copr.sh` now requests Fedora 43, 44,
  and 45 explicitly for each dependency-ordered source RPM. The fix remains
  unverified until a tagged release completes the three-chroot builds and
  version-pinned install/removal checks.
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

- The current packaging audit adds automated RPM repository probes for Rocky 8/9/10 and openSUSE Leap 15.6/16.0/Tumbleweed. The probe separates five host runtime floors from build-only API requirements and the Mutter development package's Wayland Protocols floor. Rocky and Leap remain blocked by their repository-provided versions. Tumbleweed's later package and coexistence gate is recorded below; a probe alone would not establish build, installation, co-installation, or graphical-session support.
- Arch's generated `PKGBUILD` stages the privately built Mutter into a temporary build prefix before configuring Shell. It feeds staged pkg-config, GIR, and typelib paths to Shell and checks that headers and libraries resolve from that private stage. This fixed the `mutter-clutter-51` configure failure from Verify run `36148646223`; the completed package result is recorded below.
- Stable NixOS channel package attributes fail early with recorded dependency blockers instead of implying support through `nixpkgs-unstable`. The 25.05 and 25.11 channels remain unsupported. The separate 26.05 closure now has a successful package-build gate, but remains unsupported pending installation, stock-GNOME coexistence, removal, and graphical-session evidence.

- Commit `c54e3d4b` corrects the pinned GNOME 51 host floors to GJS 1.87.1
  and Wayland Protocols 1.48, sourced directly from the exact Shell and Mutter
  Meson commits. Its first Tumbleweed co-install attempt found a generated
  `typelib(GnomeQR)` dependency escaping from Gnoblin's private Shell tree.
  The subsequent private typelib filters are covered by the completed
  Tumbleweed gate recorded below. Fedora 43, 44, and 45 clean source builds
  passed on `c54e3d4b`; this is source-build evidence, not an RPM package gate.
- The 26.05 Nix adapter pins Wayland 1.26 and its matching scanner for the
  private Gnoblin package and exports a separate
  `nixosModules.nixos_26_05`. A full local build first failed because Shell's
  pkg-config lookup found host Wayland 1.25 through private Mutter's `.pc`
  dependency. `nix/package.nix` now adds the private Wayland dev output to
  Shell's pkg-config path when that adapter is selected. The full package
  output and `nix flake check --no-build` pass, and CI now builds both Mutter
  and the complete package. Nix workflow run `36159534134` on `861df019`
  passed all channel evaluations, the full 26.05 package build, and flake
  checks. Installation, stock-GNOME coexistence, and graphical login remain
  unverified; the public guide continues to label this path experimental.
- The exact-main Arch release-style gate on `c54e3d4b` built the package, then
  failed its co-install transaction because Meson reinstalled schema outputs
  under the absolute temporary build-prefix path. Commit `187e95cc` packages
  only the runtime typelib and schema XML beneath `/usr/lib/gnoblin`; the
  corrected exact-main result is recorded below.
- Arch run `36157414890` on `4b3e5daf` passed package build, installation
  beside stock GNOME, and Gnoblin removal. Tumbleweed run `36158322082` on
  `b2a36578` passed its private RPM chain, package-isolation check,
  installation beside stock GNOME, and removal after filtering the private
  `Meta` typelib requirement. Fedora 43/44/45 clean source builds and Fedora
  44's published-package install, co-install, and removal check passed on
  run `36157414890`. The matrix records Arch and Tumbleweed as candidates,
  Fedora 44 as a candidate, NixOS 26.05 as package-build-only, and all other
  targets as unsupported. Every graphical-session gate remains false.
- Commit `ac70ae2b` corrects NixOS host-floor reporting against Gnoblin's
  Mutter compatibility patches: libinput's required floor is 1.30, while
  PipeWire 1.4 is accepted, so neither should be reported at raw upstream
  Mutter's higher floor. `nix flake check --no-build -L` passes. The channel
  assessment still reports actual Wayland/GJS/GLib blockers on the default
  stable-channel package outputs; the separate 26.05 adapter supplies private
  Wayland for its complete package build as described above.
- The initial direct Debian 12 GLib 2.90 configure failed because its system
  `g-ir-scanner` 1.74 is below the required 1.80. A new, separate
  `packaging/deb/compat-bootstrap.json` now builds GLib without introspection,
  GObject Introspection 1.80.1, and final GLib with GIRepository2. Its source
  checksums match GNOME's published checksum files. A disposable Debian 12
  build completed and verified the private GIRepository pkg-config interface,
  runtime ELF RPATH, and scanner isolation under a sibling build-tools prefix.
  The production DEB manifest and workflow remain unchanged. GTK 4.14/GCR4,
  Mutter/Shell, Ubuntu 22.04, package transactions, and graphical sessions
  remain unverified; both targets stay unsupported.
- The exact-main records above supersede the earlier in-progress results.
  Inventory workflow run `36160948365` validates the 21-target schema at the
  current revision. It validates consistency of the recorded states; it does
  not exercise a graphical desktop session.

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

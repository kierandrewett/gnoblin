# Gnoblin distribution packaging research

Status: research and implementation direction, updated 2026-10-02.

This research began while Gnoblin packaged GNOME Shell. The current standalone
runtime builds private Mutter and portal components and has no GNOME Shell or
GJS target. Older compatibility findings below that cite GJS or a private Shell
closure are historical and do not describe current build requirements.

## What other desktop projects do

Desktop projects do not solve distribution support with one universal package.
They release source and coordinate with each distribution's packagers. Those
packagers build against the target distribution's libraries, package policy,
session services, and update lifecycle.

- GNOME's release guidance explicitly treats releases as a signal to downstream
  developers to update dependencies. GNOME publishes source releases; Fedora,
  Debian, Ubuntu, openSUSE, Arch, and other distributions integrate them through
  their own package systems.
- KDE documents per-distribution installation and packaging. Its packaging
  recommendations emphasize a complete desktop metapackage and avoiding
  dependency or package-splitting changes that create runtime failures. KDE
  neon is a separate Ubuntu-LTS-based channel; it is not one binary package
  working unchanged on every distribution.
- Hyprland explicitly says it officially runs and tests Arch and NixOS; the
  remaining distribution instructions are community-maintained and carry no
  support guarantee. Its recommended installation path is the distribution's
  package manager because the compositor and its dependencies are tightly
  coupled. This is a useful support-policy model: keep an honest tested target
  set rather than implying that build instructions equal support.
- openSUSE Build Service (OBS) is an upstream-facing option for native package
  builds. It builds and publishes distribution-specific binary packages in
  clean build environments for several RPM and DEB distributions. It shares
  the build infrastructure, but it does not remove the need for target-specific
  dependency rules and integration tests.
- NixOS uses a separate package/module path. Nixpkgs stable channels are
  released twice a year, while `nixos-unstable` is a rolling channel; a flake
  pinned only to unstable does not establish compatibility with stable releases.
- Flatpak solves application distribution against a shared runtime, but a
  compositor still needs a native session entry, display-manager integration,
  device access, host session services, and package coexistence. It is not a
  replacement for native desktop-session packages.

### Research sources

- [GNOME release process](https://handbook.gnome.org/maintainers/making-a-release.html)
- [KDE packaging recommendations](https://community.kde.org/Distributions/Packaging_Recommendations)
- [KDE distribution list](https://community.kde.org/Distributions)
- [Hyprland installation guidance](https://wiki.hypr.land/Getting-Started/Installation)
- [GNOME platform components and runtimes](https://developer.gnome.org/documentation/introduction/components.html)
- [Open Build Service](https://openbuildservice.org/)
- [Nixpkgs channel model](https://wiki.nixos.org/wiki/Nixpkgs)
- [Fedora release lifecycle](https://fedoraproject.org/wiki/User:Jkurik/Fedora_Release_Life_Cycle)
- [Debian release status](https://www.debian.org/releases/)
- [Ubuntu release cycle](https://ubuntu.com/about/release-cycle)
- [openSUSE Leap lifecycle](https://news.opensuse.org/2025/09/03/leap-16-doubles-support/)

## Gnoblin audit

The release and package paths present in this checkout do not yet match the
requested coverage:

| Family   | Present path                                                                          | Missing coverage                                                                                                                |
| -------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Fedora   | COPR; Fedora 43, 44, 45                                                               | EL adapter; complete release install/coexistence and graphical-session proof                                                    |
| Debian   | Signed APT archive; Debian 13                                                         | Debian 11/12 host runtime and package compatibility                                                                             |
| Ubuntu   | Signed APT archive; Ubuntu 24.04 and 26.04                                            | Ubuntu 22.04 host runtime compatibility                                                                                         |
| Arch     | Self-contained PKGBUILD, deterministic source bundle, build/install/remove gate       | Exact-main build, stock-GNOME co-install and removal passed; graphical-session proof and repository publication                 |
| openSUSE | Tumbleweed RPM specs, private package-chain build and stock-GNOME install/remove gate | Exact-main RPM build, isolation, co-install and removal passed; graphical-session proof and OBS publication remain              |
| NixOS    | Pinned package/module paths for 25.05, 25.11, 26.05 and unstable                      | 26.05 full package builds with private Wayland; install/session/coexistence proof missing; older stable channels remain blocked |

The package URL generator emitted `https://github.com/kdrew7/gnoblin` for both
RPM and Arch metadata. That owner returns HTTP 404; the canonical
`https://github.com/kierandrewett/gnoblin` returns HTTP 200. Commit `e1472bb8`
corrects the generator and regenerated outputs.

The platform package models remain different. Debian bundles the runtime in
one package; RPM divides it into Gnoblin-named runtime packages; Arch now has a
self-contained single-package recipe with a deterministic release source
bundle. The exact-main Arch release-style gate passed `makepkg`, stock-GNOME
co-install, and removal in run `36157414890`. Tumbleweed passed its native RPM
chain, package-isolation, co-install, and removal checks in run `36158322082`.
Neither result proves graphical session login or session switching. Other RPM
and Debian layouts keep files under `/usr/lib/gnoblin` and do not replace
GNOME; each target still needs its own clean install/removal evidence.

The supported source build does not compile or run GJS. It checks the host
requirements declared by pinned Mutter and portal sources with
`scripts/check-build-deps.py`. Mutter's schema floor is now 49.1. The schema-51
stylus eraser mode and action keys now flow through Gnoblin's typed input
adapter, and Mutter uses a local enum for that mode. Its other newer optional
input keys are checked against the installed schema before use. The GTK 4.22.0
and xdg-desktop-portal 1.21.1 requirements belong to the optional
`gnoblin-portal` package; they do not block the core session build. Support
still requires measured package availability and a successful clean
build/install per target. For older targets, decide whether each missing
library is safe to use privately under `/usr/lib/gnoblin` or is a required
system integration dependency. Lower version floors only after auditing the
source and validating against the older interfaces.

The `0.1.7` release workflow was cancelled at run `36136202878` before package
install or publication. This keeps the candidate out of GitHub Releases, APT,
and COPR while the target matrix and packaging recipes are audited.

### Coexistence and delivery audit

The current package isolation is a useful base, but test evidence is uneven.
The RPM `fakeroot` check validates file ownership and package metadata without
running DNF dependency solving or scriptlets. The Fedora 44 COPR smoke test
now installs stock GNOME first, installs and removes Gnoblin, and verifies
stock package versions and binary ownership before and after; run
`36139101542` passed. The release COPR workflow has the same co-install and
removal gate across Fedora 43, 44, and 45, pending a release that exercises
it. Debian has the strongest package-level proof: it starts with stock GNOME
installed, installs Gnoblin, checks session files and package ownership,
removes Gnoblin, then checks stock GNOME remains. Nix's
`combinedProfile` asserts that stock `gnome-shell` and `mutter` still resolve
to stock Nix packages, but it is an evaluation/composition check rather than a
graphical login/removal test.

The Nix flake has isolated, lock-file-pinned inputs for `nixos-25.05`,
`nixos-25.11`, `nixos-26.05`, and `nixos-unstable`, still only on x86_64 Linux.
Its `gcc16Stdenv` argument falls back to a stable channel's default `stdenv`:
hyprcursor and its C++ closure come from that same channel, so Mutter and
hyprcursor use one compiler ABI. NixOS 26.05 now has a separate experimental
package and module. A private Wayland 1.26 and matching scanner feed only its
private Mutter build; the default rolling package and host package set are
unchanged.

The 2026-09-25 channel probe predates Shell retirement. Its GJS findings and
private Shell build status are superseded. It recorded missing `libglycin` and
older GLib, Wayland, Wayland Protocols, libinput, and PipeWire interfaces;
check those against the current source requirements before changing the target
matrix. The separate 26.05 adapter supplies a private Wayland 1.26 for its
compositor build. Current CI evaluates NixOS 25.05, 25.11, 26.05, and unstable,
but evaluation alone does not establish a complete package build, graphical
session, stock GNOME coexistence, or removal. The workflow builds the
private-compositor slice for the supported channel so channel composition
errors are caught before release.

Fedora packaging supports Fedora COPR chroots but has no EL publication target.
Its `%rhel` condition only omits an optional portal helper; it does not adapt
the Fedora package names, macros, or dependency versions for EL. Arch now has
a single-package source recipe and a deterministic release bundle; its release
workflow gates publication on `makepkg` and install/remove checks, pending the
first release run. Tumbleweed now has SUSE-native specs and clean-image
dependency-resolution CI. The complete RPM chain builds and passes private
path isolation. Its first GNOME co-install attempt exposed an automatically
generated dependency on Gnoblin's private `GnomeQR` typelib; the spec now filters
that private requirement, and the install/coexistence/removal gate is rerunning.
Graphical login remains unverified.

This means coexistence is a package-layout invariant, not yet a demonstrated
cross-distro guarantee. Each target gate should install the stock desktop
first, solve/install the exact Gnoblin package set, verify stock session
executables and package ownership remain intact, select and log in to both
sessions where a graphical runner exists, remove Gnoblin, and verify GNOME
still logs in. Where CI cannot provide a graphical runner, report that gap
separately instead of treating metadata or a successful package transaction as
full coexistence proof.

### Clean-image compatibility probes (2026-09-25)

The probes used disposable stock images and the current Gnoblin 51 dependency
requirements. A distro's GNOME version is not itself a blocker because Gnoblin
ships a private Mutter runtime; the host libraries and services it links to
remain real constraints.

| Targets                          | Probe result                                                                                                                                                                                                                                                                                                                                                                      | Practical consequence                                                                                                                                                                                                                                                                                                        |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Debian 11                        | Build solve lacks GTK4, libadwaita, GCR4, GI Repository 2.0, GNOME Desktop 4, libei, libdisplay-info, modern C++/Rust toolchains; stock runtime lacks WirePlumber and the GNOME portal                                                                                                                                                                                            | Current private bundle cannot support it. Requires a much larger toolchain, session, and portal compatibility effort.                                                                                                                                                                                                        |
| Debian 12, Ubuntu 22.04          | A clean-image CI probe records missing GCR4, GI Repository 2.0, libei/libeis, libdisplay-info, Glycin, and Hyprcursor development interfaces. Debian 12 has GTK 4.8 and Ubuntu 22.04 has GTK 4.6; Mutter 51 needs GTK 4.14. Host Rust on Debian 12 is 1.63. Stock GNOME session/settings components are GNOME 43 and 42 respectively.                                             | Not installable with today's declared dependencies and runtime closure. A full route needs a separately reviewed private GTK, GIRepository, and GCR chain, followed by clean package, stock-GNOME coexistence, graphical-session, and removal gates. Do not add either to the package build matrix until that design exists. |
| EL 8, 9, 10 (Rocky Linux images) | GLib is 2.56, 2.68, and 2.80; each image misses the current GLib 2.86 floor.                                                                                                                                                                                                                                                                                                      | Adding an EL repository or changing RPM macros cannot make these targets work. They need a separately reviewed private GLib/runtime path and per-EL session integration.                                                                                                                                                     |
| openSUSE Leap 15.6 and 16.0      | GLib is 2.78 and 2.84; both miss the current 2.86 floor. Leap 16 also misses libinput 1.30 and Wayland Protocols 1.48.                                                                                                                                                                                                                                                            | Do not add these as Gnoblin 51 targets by reusing the Fedora spec. A private runtime closure is required.                                                                                                                                                                                                                    |
| openSUSE Tumbleweed              | The earlier host-capability probe passed for GLib 2.88, PipeWire 1.6, libinput 1.32, libei 1.6, and Wayland Protocols 1.49. Its RPM resolver previously stopped at the `gsettings-desktop-schemas >= 51.0` requirement. That floor is now 49.1 after an interface audit; the full package solve and source build have not been repeated. GJS is not a Gnoblin runtime dependency. | Re-run the package solve and build against current Tumbleweed repositories before adding it as a supported target. The earlier schema-version failure is no longer current evidence.                                                                                                                                         |

The Arch placeholder is now replaced by a source-addressable single-package
recipe. Commit `6e377de6` adds a deterministic release bundle containing the
tracked Gnoblin source and materialised, patch-applied schemas, Mutter, and
Shell archives; commit `25cd6d54` generates a SHA-pinned release PKGBUILD
without requiring Nix in the release builder. It removes the old
unpublished-runtime-dependency and checkout-relative-path failures. Commit
`7c1bfa15` makes a real `makepkg` build, stock-GNOME install, Gnoblin install,
and removal checks a pre-publication release gate. That gate has not yet run
for a release tag, and graphical login is still a separate support gate.

### Current host and private runtime boundary

Gnoblin builds private Mutter and portal components under `/usr/lib/gnoblin`.
The `gnoblin` supervisor runs Mutter and the Lua worker as a standalone
session. GNOME Shell, GJS, `gnome-session`, and GNOME Settings Daemon are not
part of this runtime. Shell presentation belongs to separate Wayland clients;
see `design/lean-runtime.md` for the runtime architecture.

Keep GDM and PAM, systemd/logind and the user manager, D-Bus, PipeWire and
WirePlumber, udev, Mesa, and kernel/device drivers host-owned. Gnoblin supplies
its own portal backend and session-specific backend selection while using the
host `xdg-desktop-portal` broker. The GNOME portal remains available to GNOME,
so installing Gnoblin does not replace the host's GNOME session or portal
packages.

Build against host libraries when they meet the minima read from the pinned
Mutter and portal sources. Keep the checked requirements in
`scripts/check-build-deps.py` and packaging manifests aligned. A private
dependency prefix can verify a source build, but does not establish that a
distribution package can resolve the same runtime dependencies. Record package
resolution and session evidence separately for each target.

This boundary fits the GNOME distribution model, where upstream releases
signal downstreams to update distro packages, and the portal design supports
backend selection for multiple desktop environments. Relevant service
contracts: [systemd PAM session setup](https://www.freedesktop.org/software/systemd/man/251/pam_systemd.html),
[portal design considerations](https://flatpak.github.io/xdg-desktop-portal/docs/design-considerations.html),
[portal backend selection](https://flatpak.github.io/xdg-desktop-portal/docs/configuration-file.html),
and [PipeWire session management](https://docs.pipewire.org/page_session_manager.html).

Probe constraints and negative results are useful evidence. Keep them in the
target inventory, but don't describe a target as supported until its exact
package build, stock-GNOME coexistence, graphical session, and removal gates
pass.

## Recommended support contract

Treat supported releases as native package targets, not just source-build
instructions. Cover the newest three numbered releases in fixed-release
families, plus the current image for rolling distributions:

| Family              | Initial CI target set                                                                  |
| ------------------- | -------------------------------------------------------------------------------------- |
| Fedora              | Fedora 43, 44, 45                                                                      |
| Enterprise Linux    | EL 8, 9, 10 (RHEL-compatible rebuilds share targets only after repo/dependency checks) |
| Debian              | Debian 11, 12, 13                                                                      |
| Ubuntu LTS          | 22.04, 24.04, 26.04                                                                    |
| Arch                | Current rolling image                                                                  |
| openSUSE Leap       | 15.5, 15.6, 16.0 compatibility targets; label upstream EOL accurately                  |
| openSUSE Tumbleweed | Current rolling image                                                                  |
| NixOS               | 25.05, 25.11, 26.05 stable channels plus unstable                                      |

Upstream OS security support and Gnoblin package compatibility are separate
claims. Some third-oldest targets in this initial matrix have reached upstream
end of maintenance; do not imply that Gnoblin provides OS security updates.
Refresh the numbers and lifecycle state for every release cycle.

Use the package ecosystems where they fit: RPM for Fedora/EL and openSUSE,
DEB for Debian/Ubuntu, PKGBUILD for Arch, and a Nix module for NixOS. A shared
RPM/DEB build service such as OBS may replace bespoke build workers and custom
repositories where it covers the needed target, but it cannot remove the
per-family recipe or clean install test. Keep COPR only where its Fedora/EL
targets and publication flow provide a concrete advantage; avoid publishing
the same RPM set to multiple repositories.

Every target must prove these checks before it is called supported:

1. Resolve build and runtime dependencies from that target's repositories.
2. Build the exact GNOME 51 source pins and Gnoblin patch set in a clean target.
3. Install with stock GNOME present; retain the stock Shell/Mutter paths, files,
   packages, and session choice.
4. Log in to each session on a graphical test host and verify that removing
   Gnoblin leaves GNOME usable.
5. Publish only the package built and tested for that exact target.

## Packaging shape to simplify toward

- Keep Gnoblin-specific binaries, libraries, schemas, D-Bus services, and user
  units namespaced under `/usr/lib/gnoblin` and `org.gnoblin.*`.
- Keep package names Gnoblin-specific. Never provide, replace, obsolete, or
  conflict with the distro's `mutter` or `gnome-shell` packages.
- Maintain one runtime file layout and small native adapters for RPM, DEB,
  Arch, and Nix. Put distro version names, dependency mappings, and build target
  definitions in one checked matrix so package CI and user documentation cannot
  silently drift apart.
- Build and install-test each adapter directly in its distro image. Use OBS or
  native distribution infrastructure for repeatable builds where possible;
  avoid hand-maintained shell branches that guess package names or dependency
  versions.
- Treat repository signing, package publication, and release tags as outputs of
  passing target gates. A source archive or an RPM build alone is not a release.

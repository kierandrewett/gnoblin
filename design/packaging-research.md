# Gnoblin distribution packaging research

Status: research and implementation direction, 2026-09-25.

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
- Hyprland's install matrix is deliberately honest about dependency limits:
  Fedora offers distro packages for recent versions and a COPR for faster
  updates; openSUSE packages it in Tumbleweed but not Leap because Leap's
  dependencies are too old; Debian Bookworm is unavailable and its packaged
  version is described as outdated; Arch users use pacman/AUR; NixOS has a
  native module; other systems can build from source.
- openSUSE Build Service (OBS) is an upstream-facing option for native package
  builds. It builds and publishes distribution-specific binary packages in
  clean build environments for several RPM and DEB distributions. It shares
  the build infrastructure, but it does not remove the need for target-specific
  dependency rules and integration tests.
- NixOS uses a separate package/module path. Nixpkgs stable channels are
  released twice a year, while `nixos-unstable` is a rolling channel; a flake
  pinned only to unstable does not establish compatibility with stable releases.

### Research sources

- [GNOME release process](https://handbook.gnome.org/maintainers/making-a-release.html)
- [KDE packaging recommendations](https://community.kde.org/Distributions/Packaging_Recommendations)
- [KDE distribution list](https://community.kde.org/Distributions)
- [Hyprland installation guidance](https://wiki.hypr.land/Getting-Started/Installation)
- [Open Build Service](https://openbuildservice.org/)
- [Nixpkgs channel model](https://wiki.nixos.org/wiki/Nixpkgs)
- [Fedora release lifecycle](https://fedoraproject.org/wiki/User:Jkurik/Fedora_Release_Life_Cycle)
- [Debian release status](https://www.debian.org/releases/)
- [Ubuntu release cycle](https://ubuntu.com/about/release-cycle)
- [openSUSE Leap lifecycle](https://news.opensuse.org/2025/09/03/leap-16-doubles-support/)

## Gnoblin audit

The release and package paths present in this checkout do not yet match the
requested coverage:

| Family   | Present path                                                         | Missing coverage                                                         |
| -------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Fedora   | COPR; Fedora 43, 44, 45                                              | Enterprise Linux targets; install/coexistence gates per target           |
| Debian   | Signed APT archive; Debian 13                                        | Debian 11 and 12 build, dependency, install and coexistence gates        |
| Ubuntu   | Signed APT archive; Ubuntu 24.04 and 26.04                           | Ubuntu 22.04 and per-LTS install/coexistence gates                       |
| Arch     | Generated metapackage only                                           | Runtime packages, a buildable PKGBUILD/AUR submission, and install tests |
| openSUSE | Source-build instructions                                            | RPM package, OBS build targets, and install/coexistence tests            |
| NixOS    | Flake package/module evaluation for 25.05, 25.11, 26.05 and unstable | Package builds plus graphical-session/coexistence tests                  |

The package URL generator emitted `https://github.com/kdrew7/gnoblin` for both
RPM and Arch metadata. That owner returns HTTP 404; the canonical
`https://github.com/kierandrewett/gnoblin` returns HTTP 200. Commit `e1472bb8`
corrects the generator and regenerated outputs.

The candidate's platform package models are not consistent yet. Debian bundles
the runtime in one package; RPM divides it into Gnoblin-named runtime packages;
Arch currently emits only a metapackage whose runtime dependencies are not
published. The existing RPM and Debian layouts keep files under
`/usr/lib/gnoblin` and do not replace GNOME, but this must be demonstrated by
installing and removing the complete package set on each target with stock
GNOME already installed.

Gnoblin pins GNOME 51 and currently requires host GLib 2.86, GJS 1.85.90,
Wayland 1.26, Wayland Protocols 1.48, libinput 1.30, and PipeWire 1.4. Fedora
43 supplies the lowered libinput and PipeWire floors, but older distro releases
cannot be assumed to satisfy the full GNOME 51 build/runtime dependency set.
Support must come from measured package availability and a successful clean
build/install per target. For older targets, decide whether each missing
library is safe to use privately under `/usr/lib/gnoblin`, or is a required
system integration dependency. Do not lower version floors just to make a
solver accept a package.

The `0.1.7` release workflow was cancelled at run `36136202878` before package
install or publication. This keeps the candidate out of GitHub Releases, APT,
and COPR while the target matrix and packaging recipes are audited.

### Coexistence and delivery audit

The current package isolation is a useful base, but the test evidence is
uneven. Fedora's RPM check uses `fakeroot`, so it checks file ownership and
package metadata without running DNF dependency solving or RPM scriptlets.
The COPR smoke test installs Gnoblin into a clean Fedora container without
stock GNOME installed. Debian has the strongest package-level proof: it starts
with stock GNOME installed, installs Gnoblin, checks session files and package
ownership, removes Gnoblin, then checks stock GNOME remains. Nix's
`combinedProfile` asserts that stock `gnome-shell` and `mutter` still resolve
to stock Nix packages, but it is an evaluation/composition check rather than a
graphical login/removal test.

The Nix flake has isolated, lock-file-pinned evaluation inputs for
`nixos-25.05`, `nixos-25.11`, `nixos-26.05`, and `nixos-unstable`, still only
on x86_64 Linux. On 2026-09-25, the 25.05 and 25.11 inputs resolve but are
blocked before package or module evaluation because they do not expose
`gcc16Stdenv`, required for Gnoblin's hyprcursor ABI. The 26.05 and unstable
inputs evaluate the package derivation and enabled module. These are evaluation
results only: they do not prove a package build, a graphical session, or GNOME
coexistence. The workflow records these exact outcomes so a channel change
cannot silently turn a known blocker into an unexamined result.

Fedora packaging supports Fedora COPR chroots but has no EL publication target.
Its `%rhel` condition only omits an optional portal helper; it does not adapt
the Fedora package names, macros, or dependency versions for EL. Arch's
generated `any` metapackage depends on Gnoblin runtime packages that are not
published, so it cannot provide a complete installation. openSUSE currently
has dependency-provisioning and source-build support, but no openSUSE-native
spec or tested OBS project; Fedora RPM specs are not portable to SUSE as
written.

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
ships private Shell/Mutter binaries; the host libraries and services it links
to remain real constraints.

| Targets                          | Probe result                                                                                                                                                                                             | Practical consequence                                                                                                                                                                                                                |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Debian 11                        | Build solve lacks GTK4, libadwaita, GCR4, GI Repository 2.0, GNOME Desktop 4, libei, libdisplay-info, modern C++/Rust toolchains; stock runtime lacks WirePlumber and the GNOME portal                   | Current private bundle cannot support it. Requires a much larger toolchain, session, and portal compatibility effort.                                                                                                                |
| Debian 12, Ubuntu 22.04          | Build solve lacks GCR4, GI Repository 2.0, libei, and libdisplay-info; host Rust on Debian 12 is 1.63. Stock GNOME session/settings components are GNOME 43 and 42 respectively.                         | Not installable with today's declared dependencies and runtime closure. Measure a deliberate backport/private-build design before adding package jobs.                                                                               |
| EL 8, 9, 10 (Rocky Linux images) | GLib/GJS are 2.56/1.56, 2.68/1.68, and 2.80/1.80; all miss Gnoblin 51's GLib 2.86 and GJS 1.85.90 floors.                                                                                                | Adding an EL repository or changing RPM macros cannot make these targets work. They need a privately namespaced GNOME runtime and per-EL session integration.                                                                        |
| openSUSE Leap 15.6 and 16.0      | GLib/GJS are 2.78/1.78 and 2.84/1.84; both miss the 51 floors. Leap 16 also misses libinput 1.30 and Wayland Protocols 1.48.                                                                             | Do not add these as Gnoblin 51 targets by reusing the Fedora spec. A private runtime closure is required.                                                                                                                            |
| openSUSE Tumbleweed              | Core host floors are met: GLib 2.88, GJS 1.88, PipeWire 1.6, libinput 1.32, libei 1.6, and Wayland Protocols 1.49. Dependency planning resolved, but an 844-package install was stopped before building. | The best openSUSE candidate. It still needs a SUSE-native spec/adapter, actual RPM build, and co-install/install/remove checks. Fedora names and paths differ, including Mesa, libxcvt, GCR, libadwaita, and GNOME Desktop packages. |

The same limitation shows up on Arch in a different form: there is no complete
package to test. The published standalone PKGBUILD depends on unpublished
Gnoblin runtime package names and refers to a repository-relative example file
that is absent from the release asset. It must become a real source-addressable
runtime package set before Arch can be counted as covered.

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

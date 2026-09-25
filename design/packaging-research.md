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

| Family   | Present path                                                            | Missing coverage                                                                      |
| -------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Fedora   | COPR; Fedora 43, 44, 45                                                 | Enterprise Linux targets; install/coexistence gates per target                        |
| Debian   | Signed APT archive; Debian 13                                           | Debian 11 and 12 build, dependency, install and coexistence gates                     |
| Ubuntu   | Signed APT archive; Ubuntu 24.04 and 26.04                              | Ubuntu 22.04 and per-LTS install/coexistence gates                                    |
| Arch     | Self-contained runtime PKGBUILD and deterministic release source bundle | `makepkg`, installed package/coexistence/removal tests, and binary or AUR publication |
| openSUSE | Source-build instructions                                               | RPM package, OBS build targets, and install/coexistence tests                         |
| NixOS    | Flake package/module evaluation for 25.05, 25.11, 26.05 and unstable    | Package builds plus graphical-session/coexistence tests                               |

The package URL generator emitted `https://github.com/kdrew7/gnoblin` for both
RPM and Arch metadata. That owner returns HTTP 404; the canonical
`https://github.com/kierandrewett/gnoblin` returns HTTP 200. Commit `e1472bb8`
corrects the generator and regenerated outputs.

The platform package models remain different. Debian bundles the runtime in
one package; RPM divides it into Gnoblin-named runtime packages; Arch now has a
self-contained single-package recipe with a deterministic release source
bundle. The release workflow now gates publication on `makepkg`, stock-GNOME
co-install, and removal checks, but no release run has exercised that gate yet.
The RPM and Debian layouts keep files under `/usr/lib/gnoblin` and do not
replace GNOME, but this must be demonstrated by installing and removing the
complete package set on each target with stock GNOME already installed.

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

The Nix flake has isolated, lock-file-pinned evaluation inputs for
`nixos-25.05`, `nixos-25.11`, `nixos-26.05`, and `nixos-unstable`, still only
on x86_64 Linux. Its `gcc16Stdenv` argument falls back to a stable channel's
default `stdenv`: hyprcursor and its C++ closure come from that same channel,
so this keeps Mutter and hyprcursor in one compiler ABI rather than mixing
GCC releases.

On 2026-09-25, 25.05 cannot evaluate because it lacks `libglycin`; it also
falls below the GNOME 51 floors for GLib, GJS, Wayland, Wayland Protocols, and
libinput. The 25.11 and 26.05 package/module evaluations pass with their
channel compiler, but builds remain blocked by Wayland 1.24 and 1.25,
respectively, where Gnoblin requires 1.26. Version floors also block 25.11's
Wayland Protocols and libinput. Unstable is the only evaluated target without
one of these recorded host-floor blockers. A real 25.11 build was attempted
and Mutter stopped at the Wayland 1.26 requirement, confirming the preflight.
These results do not prove a graphical session or GNOME coexistence. The
workflow records the exact evaluation and floor results so a channel change
cannot silently turn a known blocker into an unexamined result.

Fedora packaging supports Fedora COPR chroots but has no EL publication target.
Its `%rhel` condition only omits an optional portal helper; it does not adapt
the Fedora package names, macros, or dependency versions for EL. Arch now has
a single-package source recipe and a deterministic release bundle; its release
workflow gates publication on `makepkg` and install/remove checks, pending the
first release run. openSUSE dependency planning
works on Tumbleweed, but it still needs a SUSE-native spec and package test;
Fedora RPM specs are not portable to SUSE as written.

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

| Targets                          | Probe result                                                                                                                                                                                                                                                                                                                          | Practical consequence                                                                                                                                                                                                                                                                                                        |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Debian 11                        | Build solve lacks GTK4, libadwaita, GCR4, GI Repository 2.0, GNOME Desktop 4, libei, libdisplay-info, modern C++/Rust toolchains; stock runtime lacks WirePlumber and the GNOME portal                                                                                                                                                | Current private bundle cannot support it. Requires a much larger toolchain, session, and portal compatibility effort.                                                                                                                                                                                                        |
| Debian 12, Ubuntu 22.04          | A clean-image CI probe records missing GCR4, GI Repository 2.0, libei/libeis, libdisplay-info, Glycin, and Hyprcursor development interfaces. Debian 12 has GTK 4.8 and Ubuntu 22.04 has GTK 4.6; Mutter 51 needs GTK 4.14. Host Rust on Debian 12 is 1.63. Stock GNOME session/settings components are GNOME 43 and 42 respectively. | Not installable with today's declared dependencies and runtime closure. A full route needs a separately reviewed private GTK, GIRepository, and GCR chain, followed by clean package, stock-GNOME coexistence, graphical-session, and removal gates. Do not add either to the package build matrix until that design exists. |
| EL 8, 9, 10 (Rocky Linux images) | GLib/GJS are 2.56/1.56, 2.68/1.68, and 2.80/1.80; all miss Gnoblin 51's GLib 2.86 and GJS 1.85.90 floors.                                                                                                                                                                                                                             | Adding an EL repository or changing RPM macros cannot make these targets work. They need a privately namespaced GNOME runtime and per-EL session integration.                                                                                                                                                                |
| openSUSE Leap 15.6 and 16.0      | GLib/GJS are 2.78/1.78 and 2.84/1.84; both miss the 51 floors. Leap 16 also misses libinput 1.30 and Wayland Protocols 1.48.                                                                                                                                                                                                          | Do not add these as Gnoblin 51 targets by reusing the Fedora spec. A private runtime closure is required.                                                                                                                                                                                                                    |
| openSUSE Tumbleweed              | Core host floors are met: GLib 2.88, GJS 1.88, PipeWire 1.6, libinput 1.32, libei 1.6, and Wayland Protocols 1.49. Dependency planning resolved, but an 844-package install was stopped before building.                                                                                                                              | The best openSUSE candidate. It still needs a SUSE-native spec/adapter, actual RPM build, and co-install/install/remove checks. Fedora names and paths differ, including Mesa, libxcvt, GCR, libadwaita, and GNOME Desktop packages.                                                                                         |

The Arch placeholder is now replaced by a source-addressable single-package
recipe. Commit `6e377de6` adds a deterministic release bundle containing the
tracked Gnoblin source and materialised, patch-applied schemas, Mutter, and
Shell archives; commit `25cd6d54` generates a SHA-pinned release PKGBUILD
without requiring Nix in the release builder. It removes the old
unpublished-runtime-dependency and checkout-relative-path failures. Commit
`7c1bfa15` makes a real `makepkg` build, stock-GNOME install, Gnoblin install,
and removal checks a pre-publication release gate. That gate has not yet run
for a release tag, and graphical login is still a separate support gate.

### Host and private runtime boundary

The least fragile model is a private GNOME 51 compositor/runtime inside
`/usr/lib/gnoblin` with the host distribution owning login, device, and desktop
services. Gnoblin can privately build ABI-sensitive libraries such as GLib,
Wayland, libinput, GJS/mozjs, GNOME Desktop, libei, and its compositor
dependencies. Its session wrapper must scope those library paths to Gnoblin
Shell and clear them before launching the host session manager.

Keep these services host-owned: GDM and PAM, systemd/logind and the user
manager, host `gnome-session`, host GNOME Settings Daemon, D-Bus, PipeWire and
WirePlumber, udev, Mesa, and kernel/device drivers. They own authentication,
session tracking, device access, portals, and hardware policy. Installing a
private copy alongside the host can split session state and permissions rather
than solve an old-library dependency.

Use the host `xdg-desktop-portal` broker and its normal backend selection.
Add a Gnoblin portal backend only if clean-target tests show that the stock
portal configuration cannot select an appropriate backend with Gnoblin's
desktop identity. Verify ScreenCast, RemoteDesktop, and settings portals
against the host PipeWire/WirePlumber stack, including RustDesk's live connect,
disconnect, and reconnect path. PipeWire client-library version compatibility
must be tested against each host daemon; do not infer it from matching package
names.

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

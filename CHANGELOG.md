# Changelog

## Unreleased

### Added

- A package test suite runs after the packages are built and before anything is released
  (`tests/packages`, `scripts/run-package-tests.sh`, [design/package-testing.md](design/package-testing.md)). Each
  target on each architecture, experimental and rolling ones included, gets four layers: static checks of the files
  (metadata, paths, modes, links, build paths, checksums, ABI), install, upgrade, reinstall and removal in a clean
  image of the release, a headless session of the installed compositor (Wayland globals, compositor bridge,
  `gnoblinctl`, Lua configuration, a screencopy capture, clean shutdown), and a signed repository tree. Each job
  uploads JUnit XML, a summary and the evidence to repeat the run. It replaces `scripts/check-package-install.sh`.
- The release runs one leg for each target and architecture of the generated target list
  (`.github/workflows/target.yml`): prefix build, packages, package suite, graphical VM test, verdict. A leg waits
  for no other leg; the targets that use the shared prefixes or the shared packages wait for the shared stage only.
  The release gate (`tests/packages/gate.py`) publishes only the packages whose every target and architecture passed
  every stage, the graphical VM test on that architecture included. Each target is released on its own: a failed
  target is left out and named in the release notes ("left out: <name> <arch>, <stage>") and holds no other back. A
  failed built target or an empty publish set still makes the run fail. A stage after a failed one is reported as "not run", never as a
  pass, and each planned release gets a verdict with its blocker. See
  [design/release-pipeline.md](design/release-pipeline.md).
- The graphical VM test runs on arm64 too, in an aarch64 guest (machine `virt`, UEFI, `virtio-gpu-pci`) on TCG,
  because the arm64 runners have no KVM. The guest of each architecture is data in `scripts/targets.json`. An arm64
  package is published only after its own arm64 graphical test passed; until the first one passes, no arm64 package
  is published. Arch Linux publishes no aarch64 cloud image, so the job builds one from the signed Arch Linux ARM
  archive (`scripts/build-arch-arm-image.sh`, pinned key `68B3537F39A313B3E574D06777193F152BDBE6A6`). The chroot
  stages of that build and the aarch64 guest have never run on an aarch64 host; the first CI run is their first proof. The harness
  takes `--deadline`, so a slow guest ends with a result before the job timeout, and prints the time of each check.
- One GitHub issue for each release that fails a stage or has no build and is not at its end of life
  (`tests/pipeline/issues.py`, label `release-pipeline`). It names the architecture, stage, check and detail, the
  run and what unblocks it, gets a comment only when the failures change, and is closed when the release passes
  again. An end-of-life release gets a notice only. Release tags and the nightly run change issues; other runs
  print what they would do. It replaces the `graphical-test` issues of `tests/graphical/report.py`, and the
  graphical workflow no longer runs on a schedule: the nightly run tests every target.

### Fixed

- The packaged programs no longer hold paths of the build machine: `scripts/build-prefix.sh` maps the checkout to `.`
  (`-ffile-prefix-map`), and the system layout no longer builds the build-tree fallback path of the version file into
  `gnoblinctl`, which an installed `gnoblinctl` would have read.
- Every deb carries `/usr/share/doc/<package>/copyright` in the machine-readable Debian format, with the licence of
  the package (`pkgsmith/copyright/`).
- `gnoblin` depends on PyGObject in every format, so `/usr/lib/gnoblin/bin/gdctl` runs on a clean install.
- The graphical VM test reads the checksum file of an Alpine cloud image, which holds the digest only.
- The graphical VM test starts its session on Fedora 45, where `kmsconvt@tty1` (kmscon) holds tty1 instead of
  `getty@tty1`. The session unit now stops kmscon too, so the hang-up of tty1 no longer ends the session.
- The graphical VM test logs in to Alpine guests. Alpine's OpenSSH has no PAM and refuses a key login to a locked
  account, so the seed now gives the test user and root the password field `*` (no password, not locked) instead of
  `!`. Before, Alpine 3.21 to 3.24 and edge never got past the `boot` check.
- The graphical VM test starts the Gnoblin session on Alpine. Busybox login has no PAM, so no logind session was
  opened and Gnoblin refused to start. The guest now logs in with util-linux login and `pam_elogind`, runs eudev so
  that mutter finds the GPU, and starts a D-Bus session bus.
- An emulated graphical guest waits 10 minutes for its disks (a systemd credential, without changing the image), and
  a guest that goes to emergency mode or freezes fails `boot` at once and is booted once more from a fresh disk.
- The graphical VM test starts arm64 guests. Their network device asked for an option ROM (`efi-virtio.rom`) that
  QEMU on an Ubuntu arm64 runner does not have, so QEMU stopped at start. The device has no ROM now, and a QEMU that
  stops at start is reported with its own message.
- The runtime layer of the package suite passes the background colour to `swaybg` as `#ff00ff`. swaybg 1.2.0
  (Ubuntu 24.04) rejects `ff00ff`, so the output stayed black and the R6 check failed.
- `scripts/build-prefix.sh` fetches the upstream tags of the Mutter and portal checkouts only when the pinned tag is
  missing, so a checkout that has the tag builds without access to gitlab.gnome.org.
- `xdg-desktop-portal-gnoblin` no longer asks for libsecret, which it does not link or use.

- Packages for Debian testing (`.deb`) and Alpine edge (`.apk`, release assets only), and arm64
  builds of every package. A single build
  now writes the rpm, deb, Arch and tar packages: `scripts/build-prefix.sh` builds Gnoblin
  into a prefix and pkgsmith writes the packages from it with the definitions in `pkgsmith/`.
  See [packaging and releases](docs/distribution.md).
- Fedora packages for every Fedora release from Fedora 41 that is out, also after its end of life. Each
  release is built on its own image, so its rpm asks for the glibc and libraries of that release. The
  signed repository has one tree per release. See [platform support](docs/platform-support.md).
- `scripts/targets.json` is the one list of what Gnoblin lists and builds. `scripts/supported-targets.py`
  reads it with the release data of endoflife.date and starts the jobs. A family lists its newest three
  releases and every release of the last three years (Ubuntu: long-term support releases only). End of
  life is shown and never removes a release. Each release is `built` (blocking CI job), `experimental`
  (job that cannot fail the run, for a release that no full run has verified yet) or `planned` (no
  job, with the reason). `--table` shows all of it. It replaces `scripts/package-targets.json` and its
  checker. Every family has the same data (package kind, image, build requirements script, package lane), and
  the workflows name no release, image or family: one matrix of release and architecture starts a prefix
  build, a package set and an install check for each target, on the image of that target. Debian and Ubuntu
  releases make debs that are install checks only. Their revision carries the release (`1~deb13`) and no
  repository takes them yet.
- CI jobs for Fedora 41 and 42, Debian 13, Ubuntu 24.04 and 26.04 LTS, Alpine 3.21 to 3.24 and openSUSE Leap 16.0.
  Each builds a prefix on its own image, writes its own packages and install-checks them. Each Alpine branch makes
  its own apks, because an apk binds the library sonames of the branch that built it. Leap 16.0 makes its own rpm
  with the new `opensuse` lane (`scripts/build-packages.sh --lanes opensuse --opensuse-release leap16.0`), named
  `1.leap16.0`. These packages, except the Fedora rpm, are install checks only. `scripts/install-opensuse-build-deps.sh`
  installs the openSUSE build packages. In `scripts/targets.json`, `verified_from` and `verified_through` give the
  releases that a full run has verified, and `planned_reason` gives the reason of each planned release, such as the
  libraries below the build floors.
- Signed package repositories for Fedora, openSUSE Tumbleweed, Debian testing and Arch, each with
  a `stable` and a `nightly` channel. A release tag publishes the stable channel.
  A daily build of the main branch publishes the nightly channel when main has
  moved. `scripts/pkgrepo` builds, signs and uploads the repositories, and
  `design/package-repositories.md` describes the layout. See
  [package repositories](docs/package-repositories.md).
- A small notice at the top centre of the screen when a config reload finishes (`Config reloaded`,
  gone after about 2 seconds), when a reload is rejected (the first line of the error, until it is
  clicked or a reload succeeds), and when a shortcut or command does not start
  (`<name> could not start: <reason>`). See
  [reload and error notices](docs/guides/files_and_load_order.md#reload-and-error-notices).

- System drop-ins load on their own. Lua files in `gnoblin/conf.d` under each `XDG_DATA_DIRS` entry
  (such as `/usr/share/gnoblin/conf.d`) and in `/etc/gnoblin/conf.d` run before your `init.lua`, or
  after the built-in defaults when you have no `init.lua`, so a package no longer needs a `gnoblin.load`
  line in your config. A drop-in that fails is skipped and shown in the recovery notice. See
  [system drop-ins](docs/guides/files_and_load_order.md#system-drop-ins).

### Changed

- Fedora 41 to 45, Ubuntu 24.04 and 26.04 LTS, Debian 13, Alpine 3.22 to 3.24 and openSUSE Leap 16.0 are `built`
  CI targets, not experimental: a full release run passed every stage on them on amd64 and arm64. A failure there now
  fails the release run. Alpine 3.21 stays experimental: its Mesa 24.2.8 offers no
  `EGL_EXT_image_dma_buf_import_modifiers` to the software renderer, so the package suite finds no
  `zwp_linux_dmabuf_v1` global in a container.
- The source builds against the libraries of Debian 13 and Ubuntu 26.04 LTS, not only Fedora 43 and newer. The floors
  were then GLib 2.84, gsettings-desktop-schemas 48.0, libwayland 1.23, wayland-protocols 1.44, libinput 1.28, PipeWire
  1.4.2, IBus 1.5.32 and, for the portal backend, GTK 4.18. Glycin is optional. With these older libraries pens report
  no eraser button, no libinput Lua plugins load, clients get no `wl_fixes` global and repeat keys themselves, the IBus
  input method shows no preedit hints, the newer input settings come from the Lua configuration only, and without
  Glycin the portal backend decodes wallpaper previews with gdk-pixbuf in its own process. Builds against newer
  libraries keep every feature. `scripts/install-debian-build-deps.sh` installs the Debian and Ubuntu build packages,
  and a CI job compiles the compositor and the portal backend on the oldest release of each family. See
  [build from source](docs/install-source.md). Before this change a build against IBus 1.5.32 left out the IBus input
  method without a message.
- The source also builds against the libraries of Fedora 41 and Alpine 3.21. The floors are now GLib 2.82,
  gsettings-desktop-schemas 47.1, wayland-protocols 1.38, libinput 1.27, PipeWire 1.2.7, libei 1.3.0, IBus 1.5.31 and,
  for the portal backend, GTK 4.16, libadwaita 1.6 and xdg-desktop-portal 1.18.4. With libadwaita 1.6 the screen
  sharing dialog shows the older view switcher. With xdg-desktop-portal 1.18 sandboxed apps cannot ask for USB devices.
  Below libei 1.5 Gnoblin does not wait for queued input before it answers a sync request from a remote desktop
  client. Builds against newer libraries keep every feature.
- The source also builds against the libraries of Ubuntu 24.04 LTS, which is an experimental CI target. The floors are
  now GLib 2.80, gsettings-desktop-schemas 46.1, libwayland 1.22, libdisplay-info 0.1.1, libinput 1.25, PipeWire 1.0.5,
  libei 1.2.1, IBus 1.5.29 and, for the portal backend, GTK 4.14 and libadwaita 1.5. Ubuntu 24.04 loses the most:
  tablet pad dials do nothing, the pen pressure range is not applied, tap-and-drag lock ends after a timeout, screen
  sharing has no explicit sync (torn or partly drawn frames with a driver that needs it), each app connection has a 4
  KB message buffer instead of 1 MB, the switch-monitor and keybinding stylus actions come from the Lua configuration
  only, and apps get no accent colour from the Settings portal. The colour and HDR data of a display come from a copy
  of the libdisplay-info 0.2 code. Nobody has started a session on Ubuntu 24.04. Builds against newer libraries keep
  every feature. `scripts/install-debian-build-deps.sh` installs Meson 1.5.2 from PyPI when the release has an older
  Meson. The deb packages ask for GLib 2.80, gsettings-desktop-schemas 46.1 and GTK 4.14, and for `libdisplay-info1`
  when the prefix links `libdisplay-info.so.1`. See [platform support](docs/platform-support.md#older-system-libraries).
- The build dependency scripts for Fedora, Alpine and openSUSE name no versions. The floors come from the pinned Meson
  files, which `scripts/check-build-deps.py` checks before the build. Glycin is installed only where the release has
  it. Before this change the Fedora script failed on every release before Fedora 43.
- `gnoblin-geoclue-integration` requires GeoClue 2.7.0, the first version that reads the drop-in folder
  `/etc/geoclue/conf.d`. It required 2.7.2, which stopped the install on openSUSE Leap 16.0 (GeoClue 2.7.1).
- `scripts/check-package-install.sh` checks what the Gnoblin packages require, not what else is installed: `gnoblin`
  requires no GTK and no libadwaita, and no Gnoblin package requires GTK 3 or gnome-desktop. On Fedora 42 the
  distribution's own `gtk4` pulls GTK 3.
- The libinput floor of the packages comes from the libinput symbol versions in the programs, as the glibc floor does.
  `scripts/build-packages.sh --lanes deb --deb-release <name>` writes debs from a prefix built on a Debian or Ubuntu
  release, for install checks.
- The RPM specs, the PKGBUILDs, the openSUSE chain and `packaging/` are gone. Package
  names, the Fedora epoch and the `Provides` and `Obsoletes` entries are unchanged, so
  existing installs upgrade. The portal backend is now version `51.0.0` with a release
  number that follows the Gnoblin release. `gnoblin-ananicy-rules` and
  `gnoblin-geoclue-integration` are built for every format and take the Gnoblin version.
- COPR no longer receives releases, because pkgsmith writes binary packages and COPR builds
  source RPMs. The release no longer carries source RPMs, PKGBUILDs or `gnoblin.db`. Use
  the package repositories or the release packages. `scripts/install-system.sh`, which
  installed from COPR, is removed. The `gnoblin-ananicy-v*` releases are replaced by the
  package of the same name in every Gnoblin release.
- `scripts/prepare-release.sh` sets the version in `src/gnoblin-version.json` only.
- The Nix package and module moved from `packaging/nix/` to `nix/`. The flake's outputs are unchanged.
- Packages accept a pre-release version. The source bundle keeps the SemVer
  spelling (`0.1.13-nightly.…`) and pkgsmith spells the package version for the
  package manager (`0.1.13~nightly.…` for RPM). Release versions are unchanged.
- The platform support page lists Arch, Fedora 45 and openSUSE Tumbleweed as candidates. Release run
  39855522942 passed their package build, install, stock GNOME coexistence and removal gates. Graphical
  session support is not claimed.
- The X11 window frames client (`mutter-x11-frames`), the only program of the compositor that links GTK 4, is
  in a new package, `gnoblin-x11-frames`. The `gnoblin` rpm and deb ask the system for no GTK 4 or libadwaita and
  recommend the new package, so `dnf`, `zypper` and `apt` install it by default. Arch and apk cannot recommend a
  package, so `gnoblin` depends on it there (one constant in `pkgsmith/runtime.star`,
  `FRAMES_HARD_DEPENDENCY_ON_NO_WEAK_DEP_FORMATS`, sets this), and a plain install keeps the title bars. The
  `.tar.xz` frames package is needed for X11 apps. The split is not complete. Without the frames client, X11 apps
  that use the system title bar get no title bar and no borders, and some X11 windows may fail to open. This is
  not tested in a graphical session, so a no-GTK install is not a promise of safe X11 apps. The portal backend
  still needs GTK 4 and libadwaita. See [optional packages](docs/optional-packages.md#x11-window-frames).

### Fixed

- The touchpad disable-while-typing timeout now takes effect with libinput 1.31 or newer. The build had left the setting
  out because the generated `config.h` did not carry its define.
- The portal backend now destroys and releases each ScreenCast consent dialog
  and its hidden parent window when the session ends. Before, the dialog's
  widget tree stayed alive after every screen cast.

### Removed

Gnoblin no longer manages app permissions or persistent portal grants. Bingux
owns permission rules and enforcement.

| Surface      | Removed                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------ |
| Lua          | `gnoblin.permission_rule`, `gnoblin.configure.permissions`, `gnoblin.portals.grants`       |
| `gnoblinctl` | `permissions` and `grant` commands                                                         |
| Events       | `gnoblin.permission.changed`, `gnoblin.portal.grant-added`, `gnoblin.portal.grant-removed` |
| D-Bus        | `CheckPermission`                                                                          |

Gnoblin still routes portal interfaces to the selected backend. The default GTK
backend keeps its normal consent dialogs.

### Migration

1. Remove the `permissions` section, `gnoblin.permission_rule` calls, and
   `gnoblin.portals.grants` calls from your Lua config. A `permissions`
   section is ignored and the log names it as an unknown section. A call to a
   removed function is a Lua error and the config fails to load.
2. Write permission rules in the portal backend you use. Bingux reads its own
   rules.
3. Stop subscribing to the removed events and calling the removed commands.

### Changed

`org.gnoblin.Shell.GlobalShortcutsProvider` `BindShortcuts` and
`ConfigureShortcuts`, and `org.gnoblin.Shell.GlobalShortcutsRebind`
`RebindShortcuts`, take a trailing request handle (`o`), the caller's
`org.freedesktop.portal.Request` object path. The shell uses it, with the
application ID, to identify the requester. Calls with the old argument lists
are not supported. Deploy the portal backend and the shell provider together.

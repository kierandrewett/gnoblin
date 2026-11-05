# Packaging and releases

For user installation, see [Install Gnoblin](../docs/installation.md).
This page is for release maintainers.

The public [platform support](../docs/platform-support.md) page distinguishes package
candidates from fully supported session installations. Do not describe a target
as supported until its graphical-session gate has passed.

## Package layout

Gnoblin installs as its own session and can coexist with GNOME. Five packages come from one build:

| Package                       | Contents                                                               | Optional |
| ----------------------------- | ---------------------------------------------------------------------- | -------- |
| `gnoblin`                     | Compositor, Lua runtime, session services, private Mutter, login entry | No       |
| `gnoblin-x11-frames`          | `mutter-x11-frames`, the title bars and borders of X11 apps (GTK 4)    | Yes      |
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
- The five packages have the same version, the Gnoblin version in `src/gnoblin-version.json`,
  and epoch 1. The portal backend keeps the version of its GNOME source (`51.0.0`). Its release
  number follows the Gnoblin release, so each release has a distinct portal package.

### GTK and the `gnoblin` package

The split is not complete. The `gnoblin` package itself links no GTK 4 and no libadwaita, and the rpm and
deb ask the system for none. Arch and Alpine packages still pull GTK 4 in, through a dependency on
`gnoblin-x11-frames`, until the missing-helper path is tested (see below). A no-GTK install is
not yet a safe install for X11 apps. Two files link GTK: `mutter-x11-frames` links GTK 4, and
`xdg-desktop-portal-gnoblin` links GTK 4 and libadwaita. Each has its own package.
`libmutter-clutter` links ATK. That library has no GTK dependency and stays in `gnoblin`. GCR is
`gcr-libs` on Fedora, `libgcr-4-4` on Debian and `gcr4-base` on Alpine. These packages have no GTK
dependency either. `scripts/build-packages.sh` stops if a program other than the two links GTK, and
`scripts/check-rpm-isolation.py` rejects a `gnoblin` rpm that requires it.

The frames client is not optional in the way that the portal backend is, so read this part with care.

- **What it does.** Mutter marks each X11 window that needs a frame with the `_MUTTER_NEEDS_FRAME`
  property. The frames client sees the mark, makes the frame window and sets `_MUTTER_FRAME_FOR`.
  Mutter then finishes the set-up of the window (`events.c`, `meta_window_x11_initialize_state`).
  Apps that draw their own frame (`_GTK_FRAME_EXTENTS`) and Wayland apps do not use it.
- **Without it.** `meta_frame_launch_client` logs `Could not launch X11 frames client` and returns
  nothing. No frame is ever made, so X11 apps that need one get no title bar and no borders. Mutter
  has no second code path for that case. The set-up of such a window waits for the frame, so the window
  may not open at all. This is read from the source of the pinned Mutter and has not been run in a
  session. A build without the client is not possible either: `meson.build` requires GTK 4 for
  Xwayland (`X11 client support requires gtk4`), so `-Dxwayland=false` would remove X11 apps.
- **How a user keeps it.** `gnoblin` recommends `gnoblin-x11-frames` in rpm and deb. `dnf`, `zypper` and
  `apt` install a recommended package by default, so a normal install has frames. A user who wants no GTK
  refuses it (`--setopt=install_weak_deps=False`, `--no-recommends`, `--no-install-recommends`).
- **Formats without a weak dependency.** pkgsmith writes an Arch recommendation as `optdepends`,
  which pacman prints and does not install. apk has no weak dependency. A tar package has no
  dependency at all. For now `gnoblin` therefore DEPENDS on `gnoblin-x11-frames` in Arch and apk, so
  a plain `pacman -S gnoblin` or `apk add gnoblin` keeps the title bars. One constant,
  `FRAMES_HARD_DEPENDENCY_ON_NO_WEAK_DEP_FORMATS` in `pkgsmith/runtime.star`, sets this. Set it to
  `False` and Arch gets the optional dependency and apk gets nothing, so a user installs the frames
  package by name. Do that only when the missing-helper path is tested in a graphical session, or when
  Mutter is patched to run without the client. A tar package has no dependencies, so
  `docs/optional-packages.md` tells its user that the frames tarball is needed for X11 apps.
- **Upgrade from an earlier release.** The earlier `gnoblin` package carried the file. rpm moves it in
  one transaction. The deb has `Replaces: gnoblin (<< 1:VERSION)` for the same reason. On Arch and apk
  the dependency brings the package in on upgrade. An rpm or deb upgrade may not install a newly
  recommended package by itself, so a user whose X11 apps lose their title bars after it installs
  `gnoblin-x11-frames` once.
- **Same build.** The frames package needs the exact `gnoblin` build that it came from (`gnoblin = 1:VERSION-RELEASE`
  in rpm and deb). Where `gnoblin` depends on the frames package (Arch and apk now), it pins the build
  from that side, because two packages that need each other make a dependency cycle. On Arch the pin is
  exact. On apk it names the package only, because pkgsmith spells an apk nightly version in a way that
  `pkgsmith/common.star` does not.

A change that makes Mutter run without the frames client, for example a start that skips the frame when
the binary is missing, would let a user opt out with no loss of windows. It belongs in `patches/`.

Nix uses separate store outputs. Source builds use their own prefix, `/usr/local/lib/gnoblin` by default.
Bingux owns and releases its shell package separately.

## How the packages are made

Nothing is built by a distribution recipe. One build serves every package format:

1. `scripts/build-prefix.sh` runs `./build.sh --layout system` on a system and writes a prefix, a
   folder laid out like an installed system. It links Lua statically and puts libraries in `lib`,
   because Fedora, openSUSE, Arch and Debian name the Lua library and the library directory
   differently.
2. `scripts/build-packages.sh` runs [pkgsmith](https://github.com/kierandrewett/pkgsmith) on the
   prefix with the definitions in `pkgsmith/`. It reads the architecture, the libraries the
   programs need and the glibc floor from the prefix files.
3. The package suite (`scripts/run-package-tests.sh`, `design/package-testing.md`) installs, runs, upgrades and
   removes the packages on a clean system of each release, and the graphical VM test boots each release with them.
   The release gate (`tests/packages/gate.py`) keeps any package whose target failed a stage out of the release.

To try it on a Fedora system with Rust installed, where `43` is that system's Fedora release:

```sh
./scripts/install-fedora-build-deps.sh
./scripts/build-prefix.sh --prefix prefix
PATH="$(./scripts/install-pkgsmith.sh):$PATH" ./scripts/build-packages.sh --prefix prefix --output dist --fedora 43
```

On another system, use its `scripts/install-<system>-build-deps.sh` (Debian and Ubuntu, Alpine,
openSUSE) and the lane of that system: `--lanes deb --deb-release deb13`, `--lanes opensuse
--opensuse-release leap16.0`, or `--lanes other` on Alpine. The dependency scripts name no versions.
`scripts/check-build-deps.py` checks the floors of the pinned Meson files before `./build.sh`
compiles, and stops with the library and version that the system lacks.

These files make up the definitions:

- `scripts/pkgsmith.ref` pins the pkgsmith commit.
- `pkgsmith/runtime.star` writes `gnoblin`, `gnoblin-x11-frames` and the portal backend. It lists
  the files of the frames client and of the portal, so everything else in the prefix belongs to `gnoblin`.
- `pkgsmith/integrations.star` writes the two configuration packages.
- `pkgsmith/dependencies.star` maps each library to the Debian and Arch package that ships it.

The build stops when a program needs a library that is not listed. Run
`python3 tests/pkgsmith-packages.test.py` after you change these files.

| Format      | File                                  | Systems                                 |
| ----------- | ------------------------------------- | --------------------------------------- |
| `rpm`       | `gnoblin-<v>-1.fc<N>.x86_64.rpm`      | Fedora release `<N>`, one file each     |
| `rpm`       | `gnoblin-<v>-1.tumbleweed.x86_64.rpm` | openSUSE Tumbleweed                     |
| `rpm`       | `gnoblin-<v>-1.leap<N>.x86_64.rpm`    | openSUSE Leap `<N>`, install check only |
| `deb`       | `gnoblin_<v>-1_amd64.deb`             | Debian testing (`forky`)                |
| `archlinux` | `gnoblin-1:<v>-1-x86_64.pkg.tar.zst`  | Arch Linux                              |
| `apk`       | `gnoblin-<v>-r1.apk`                  | Alpine edge                             |
| `tar`       | `gnoblin-<v>-amd64.tar.xz`            | Other glibc systems, extract below `/`  |

CI builds amd64 and arm64. The Fedora and openSUSE rpm files differ only in dependency names.

The glibc floor, the libinput floor and the library names come from the programs of the prefix, so
each format asks for what its prefix needs. glibc and libinput version their symbols, so
`scripts/build-packages.sh` reads the newest `GLIBC_` and `LIBINPUT_` version from the programs.

GLib, gsettings-desktop-schemas, libwayland, PipeWire and GTK have fixed floors in `pkgsmith/`. The floors in
`pkgsmith/runtime.star` are the versions of the oldest systems of each format that the source builds on: Fedora 41
and Alpine 3.21 for rpm, Arch and apk, Ubuntu 24.04 for deb. The libwayland-server and PipeWire floors in `FLOORS` of
`pkgsmith/dependencies.star` are the Ubuntu 24.04 versions in every format.

### One prefix for each target

The target legs of `release.yml` (`.github/workflows/target.yml`) build a prefix on the image of every built or
experimental target that has its own prefix (`"prefix": "own"` in `scripts/targets.json`), on each architecture. The
shared prefixes are built in the shared stage of `release.yml`. The workflows name no release and no image. Each job
takes them from the output of `scripts/supported-targets.py`, which gives one entry for each release and
architecture:

| Entry field                                           | What the job does with it                                                       |
| ----------------------------------------------------- | ------------------------------------------------------------------------------- |
| `prefix_image`, `bootstrap`, `deps_script`            | Start the image, install the first tools, install the build requirements        |
| `lane`, `package_args`                                | Write the packages of this release from its prefix                              |
| `prefix_artifact`, `packages_artifact`, `slug`        | Name the artifacts so that two releases never share a name                      |
| `install_image`, `install_artifact`, `install_suffix` | Install the right package set on the right image                                |
| `install_prepare`                                     | A command that runs in the install image first, for a tool that the image lacks |

A target with a `lane` writes its own packages from its own prefix, so the packages ask for the
glibc and the library sonames of that release:

| Lane       | Arguments                                    | Writes                                         | Artifact                                 |
| ---------- | -------------------------------------------- | ---------------------------------------------- | ---------------------------------------- |
| `fedora`   | `--lanes fedora --fedora <N>`                | The rpm of Fedora release `<N>`                | `packages-<tag>-fc<N>-<arch>`, published |
| `deb`      | `--lanes deb --deb-release <slug>`           | The debs, with the slug in the revision        | `testpkgs-<tag>-<slug>-<arch>`           |
| `opensuse` | `--lanes opensuse --opensuse-release <slug>` | The openSUSE rpm, with the slug in the release | `testpkgs-<tag>-<slug>-<arch>`           |
| `other`    | `--lanes other`                              | The apks, from a musl prefix                   | `testpkgs-<tag>-<slug>-<arch>`           |

The rpm of Fedora 43 asks for the glibc and the library sonames of Fedora 43. A library that only
an older release has under that soname, such as `libdisplay-info.so.2` on Fedora 42 and 43, Debian 13
and openSUSE Leap 16.0, or `libdisplay-info.so.1` on Ubuntu 24.04, is allowed in the Fedora rpm, in the deb and in
the rpm of the `opensuse` lane (`OLDER_SONAMES` in `pkgsmith/dependencies.star`). The shared openSUSE rpm is for
Tumbleweed, which has no package for it, so the shared lane stops on it. Arch has no package for it either.

Names do not collide between releases:

- The slug of a deb target is in the revision (`1~deb13`, `1~ubuntu26.04`). The debs of two
  releases never have the same file name. A deb of a release sorts before the deb of the shared
  prefix.
- The slug of an openSUSE Leap target is in the rpm release (`1.leap16.0`), as `.fc43` is in a Fedora
  rpm. The shared openSUSE rpm is `1.tumbleweed`.
- The slug comes from the `slug` field of the family.
- An apk has no such revision: apk orders by version and release only. The apks of different
  releases stay apart by their artifact names.

The packages of a target that is not `published` are install checks only. Their artifact name
starts with `testpkgs`, not `packages`.

The publish job and `package-repository.yml` read the artifacts that start with `packages`. So no
GitHub release and no repository suite (such as `deb/forky`) takes a per-release deb or apk. To put
them into a repository, change `scripts/pkgrepo` later.

The packages for the rolling systems run on systems that are newer than Fedora: the openSUSE rpm,
the deb, Arch, apk and tar packages. They come from the shared prefixes (`--lanes other`):

- The glibc prefix is the one of the release named `source_release` in `scripts/targets.json`.
- The musl prefix is the one of every rolling target with `"shared_source": true` (Alpine edge).

Their artifacts are called `prefix-shared-...`, so the `packages-shared` job finds them with one
pattern and runs the same lane on each one.

Fedora 43 links `libdisplay-info.so.2`, which Debian, Arch and Tumbleweed do not ship, so it
cannot be the source. The shared prefixes are never experimental, so those packages do not wait on
a release that may fail.

The rolling targets with a shared prefix (Tumbleweed, Arch, Debian testing) and every release
with `"prefix": "shared"` install the shared packages on their own image.

Every listed release has one of three states (see below). The jobs of an experimental release have
`continue-on-error`, so a failure there is visible and does not fail the run.

A release that Gnoblin promises never has `continue-on-error`. It is `built` and blocking, or
`planned` and has no job.

### Choosing the targets

`scripts/targets.json` is the one file that says what Gnoblin lists and builds. A person edits it.
`scripts/supported-targets.py` reads it with the `endoflife.date` data of each family and prints
the targets. Nothing it prints is kept in the repository.

The age goal is two rules together. A release of a family is listed when it is among the newest
`newest` releases of the family (three), or when it was first published in the last `window_years`
years (three). Both use the release date from `endoflife.date`.

The newest three alone would cover only about 18 months of Fedora or Alpine. Three years alone would
drop the oldest Debian and Ubuntu releases. End of life never removes a release. It is a column in the table
and nothing more.

Ubuntu lists long-term support releases only (`lts_only`). An interim release lives nine months,
so it is not a compatibility target.

The list can name releases that Gnoblin cannot build yet. Those stay in the table as `planned`,
with the reason. The table shows the owner where the age goal is not met.

| State          | Meaning                                                                                                         |
| -------------- | --------------------------------------------------------------------------------------------------------------- |
| `built`        | `verified_from` <= release <= `verified_through`. A CI job builds or install-checks it. A failure fails the run |
| `experimental` | `build_from` <= release, and the release is not `built`. Its job runs. A failure does not fail the run          |
| `planned`      | Release < `build_from`, or the family has no `build_from`. It has no job                                        |

`experimental` is for a release that no full run has proved yet. That is:

- a release newer than the last verified release of its family
- a release between `build_from` and `verified_from`: a job covers it, and no full run has passed
  on it yet (Alpine 3.21 while Alpine 3.22 and newer are verified)
- a release of a family that has `build_from` and no verified release
- a release that is not out yet

The workflows run it, and a failure is visible. Set `verified_from` and `verified_through` only when
a full run on GitHub has proved that the releases between them build and install.

The file has these fields at the top level:

| Field           | What it does                                                                                         |
| --------------- | ---------------------------------------------------------------------------------------------------- |
| `newest`        | How many of the newest releases of each family are listed                                            |
| `window_years`  | A release first published in this many years is listed as well                                       |
| `architectures` | The architectures of every job, each with its runner: `{"arch": "amd64", "runner": "ubuntu-latest"}` |

Each family in `scripts/targets.json` has these fields:

| Field                 | What it does                                                                                                                                                                                                |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `product`             | The `endoflife.date` product that lists the releases of the family                                                                                                                                          |
| `image`               | The container image of a release, with `{release}` in it. It builds the prefix and runs the install check                                                                                                   |
| `install_image`       | Optional. The image of the install check, when it is not `image`                                                                                                                                            |
| `install_prepare`     | Optional. A command that the install check runs in its image first: `bash` on Alpine, `update-alternatives` on a minimal Leap image                                                                         |
| `<arch>_image`        | Optional, such as `arm64_image`. The install image on that architecture, for an image with no build for it                                                                                                  |
| `slug`                | The name of the release in job and artifact names, with `{release}` in it: `fc43`, `deb13`, `ubuntu26.04`                                                                                                   |
| `kind`                | The package format of the system: `rpm`, `deb`, `apk` or `pacman`                                                                                                                                           |
| `prefix`              | `own`: the release builds a prefix on its image. `shared`: it installs the shared packages                                                                                                                  |
| `bootstrap`           | Own prefix. The command that installs git, tar, xz and zstd in the image                                                                                                                                    |
| `deps_script`         | Own prefix. The script in `scripts/` that installs the build requirements                                                                                                                                   |
| `lane`                | Own prefix. The package lane that writes the packages of the release (table above). Without it, the prefix feeds the shared packages only                                                                   |
| `published`           | Optional. The packages of the release are release assets and go into the repository                                                                                                                         |
| `lts_only`            | Optional. List only the releases that `endoflife.date` marks as long-term support                                                                                                                           |
| `build_from`          | The oldest release that a CI job covers. `null` when no release has a job                                                                                                                                   |
| `verified_from`       | The oldest release that a full run on GitHub passed on. `null` when none has                                                                                                                                |
| `verified_through`    | The newest release that a full run on GitHub passed on. `null` when none has                                                                                                                                |
| `planned_reason`      | An object that maps each listed `planned` release to the reason it has no job, such as the libraries below the build floors. The table shows it. A listed planned release without an entry stops the script |
| `experimental_reason` | Optional. An object that maps an `experimental` release to the known cause of its failing job. The table shows it in place of the default note                                                              |
| `source_release`      | One family only. The release whose prefix builds the source bundle and makes the shared glibc packages                                                                                                      |
| `upcoming`            | `bodhi`: also add a Fedora release that Bodhi dates before `endoflife.date` lists it                                                                                                                        |

The script checks the data. It stops when a kind or a lane is unknown, when a lane writes another
kind of package than the family has, when a deps script does not exist in `scripts/`, when a
prefix that is `shared` names build data, when a shared source is not `built`, when only one of
`verified_from` and `verified_through` is set or they are outside `build_from` and each other, and
when a listed `planned` release has no reason.

`endoflife.date` lists a Fedora release some days after it is out. With `upcoming` set to
`bodhi`, the script also adds a newer release that Bodhi dates and whose container image exists.

The rolling targets have no release date, so the file lists them by hand and they are always
`built`. They have the fields of a family, except the ones about releases and dates.

Alpine edge has an own prefix and `"shared_source": true`. It makes the apk of the shared
packages.

`release.yml` runs the script in its `targets` job and writes `--table` to the step summary. It starts the shared
stage and one leg (`target.yml`) for each `built` and `experimental` target and architecture (`legs_own` and
`legs_shared`). `design/release-pipeline.md` describes the jobs.

`verify.yml` reads the same output for its compile gate. The oldest release of each family that has
a native build (`build_from`) is compiled with `./build.sh` on its own image.

| Stage             | One job for each                                |
| ----------------- | ----------------------------------------------- |
| `prefix`          | Target with an own prefix, on each architecture |
| `packages-shared` | Architecture                                    |
| `packages`        | Target with a lane, on each architecture        |
| `install`         | Target on each architecture                     |

Run `scripts/supported-targets.py --table` to see every listed release with its release
date, end-of-life date, state and reason, a summary line for each family, and the number of jobs
in each stage for the current data.

- To list a new family, add it with the fields above. Its releases are `planned` until a CI job
  covers them.
- To start a job for a release, set `build_from` to the oldest release that the job covers. Leave
  `verified_from` and `verified_through` as they are: the release is `experimental`, and a failure is
  visible without failing the run.
- When a full run on GitHub proves that the release builds and installs, move `verified_from` or
  `verified_through` to include it. The release is then `built`.
- A new release of a family joins by itself, as `experimental`, until `verified_through` covers it.
- When `build_from` moves to a newer release, give each release below it a `planned_reason`.

The list says what the workflows build and check. It does not make a support claim:
`docs/platform-support.md` states the support status of each release.

### Source guards for Fedora 41 and Alpine 3.21

The source builds on Fedora 41 and Alpine 3.21 with guards in `patches/mutter/32-older-systems` (`0005` to `0009`), in
`patches/xdg-desktop-portal-gnome/06-older-systems` (`0002` and `0003`) and in the libgxdp patch. Each guard keeps the
newer code when the newer library is installed. libei 1.3.0 has no `EIS_EVENT_SYNC`, so the sync case is compiled out.
wayland-protocols 1.38 lacks five protocol files or versions, so below 1.44 the build uses private copies of the 1.48
files. libadwaita 1.6 has no `AdwInlineViewSwitcher`, so the screen cast widget makes its switcher in code and uses
`AdwViewSwitcher` on 1.6. GTK 4.16 has no `gtk_disable_portals()`, so libgxdp adds `no-portals` to `GDK_DEBUG` before
GTK starts and restores `GDK_DEBUG` after. The guards are decided at build time, so each of these releases needs its
own prefix.

### Source guards for Ubuntu 24.04

Ubuntu 24.04 LTS is the oldest release that builds, and it loses the most. It stays `experimental` until a full run on
GitHub proves it, and nobody has started a session on it. The guards are `0010` to `0017` in
`patches/mutter/32-older-systems`, `0004` in `patches/xdg-desktop-portal-gnome/06-older-systems`, the GTK 4.14 floor of
the libgxdp patch and the IBus guards in `src/input/method/gnoblin-ibus-input-method.c`. Each keeps the newer code when
the newer library is installed. Each new `HAVE_*` define is in Meson and in `config.h.meson`, except
`HAVE_LIBDISPLAY_INFO_0_2`: `edid.h` tests it and some Gnoblin units include `edid.h` before `config.h`, so it is a
compiler argument.

| Library on Ubuntu 24.04                                 | Guard                                                                                               | What the person at the desktop notices                                                                  |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| GLib 2.80                                               | `0010`: `g_qsort_with_data()` for `g_sort_array()`                                                  | Nothing                                                                                                 |
| libwayland 1.22                                         | `0011`: the client object is kept in a late destroy listener; no 1 MB connection buffer             | An app that floods messages or reads slowly is disconnected sooner (4 KB buffer, as before Mutter 47)   |
| libdisplay-info 0.1.1                                   | `0012`: the 0.2 types, filled from the EDID with a copy of the 0.2 derivation                       | Nothing, if the copy is right                                                                           |
| libinput 1.25                                           | `0013`: no pad dials, no pressure range; drag lock in timeout mode; bus type from sysfs             | Pad dials do nothing; the pen pressure range is not applied; tap-and-drag lock ends after a timeout     |
| PipeWire 1.0.5                                          | `0014`: explicit sync compiled out; `node.supports-request` named but unknown to PipeWire           | Screen sharing without explicit sync: torn or partly drawn frames with a driver that needs it           |
| gsettings-desktop-schemas 46.1                          | `0015`: two stylus action values defined; pressure range and keybinding keys read only when present | Switch-monitor and keybinding stylus actions from the Lua configuration only; no pressure range setting |
| libei 1.2.1                                             | `0016`: floor only                                                                                  | Nothing more than with libei 1.3.0                                                                      |
| IBus 1.5.29                                             | `0017` and the overlay file: no preedit hint code                                                   | Nothing more than with IBus 1.5.31 (no preedit hints)                                                   |
| libadwaita 1.5, gsettings-desktop-schemas 46.1 (portal) | portal `0004`: no accent colour; `GtkSpinner`; description not centred                              | Apps get no accent colour from the Settings portal; older spinner in the wallpaper preview              |
| GTK 4.14 (portal)                                       | libgxdp: `no-portals` in `GDK_DEBUG`, as for GTK 4.16                                               | Nothing                                                                                                 |
| Meson 1.3.2                                             | `scripts/install-debian-build-deps.sh` installs Meson 1.5.2 from PyPI into `/opt/gnoblin-meson`     | Nothing: a build tool only                                                                              |

Every loss of Debian 13, Fedora 41 and Alpine 3.21 applies as well. The deb packages ask for GLib 2.80,
gsettings-desktop-schemas 46.1 and GTK 4.14 (portal backend), and for `libdisplay-info1` when the prefix links
`libdisplay-info.so.1`. The libwayland and PipeWire floors of `pkgsmith/dependencies.star` are 1.22.0 and 1.0.5.

These guards change behaviour that nobody has run in a graphical session:

- Screen cast without explicit sync. `explicit_sync_supported()` returns false, so the stream offers only the buffer
  parameters without a sync timeline. Upstream takes the same path when the consumer or the DRM driver has no explicit
  sync, but on Ubuntu 24.04 it is the only path.
- The colour pipeline gets its primaries, gamma, signal colorimetry and HDR metadata from the copied derivation, not
  from libdisplay-info. A mistake there gives wrong colours or a wrong HDR capability on a display.
- The hardware cursor path reads shm cursor buffers through their `wl_buffer` resource, as Mutter 48 did (the
  libwayland 1.23 guard of `0003`, also on Ubuntu 24.04).
- The Wayland client object lives in a late destroy listener. If libwayland called it at another point than the user
  data destroy function, a resource destructor could read a freed client.
- With the 4 KB connection buffer, a client that floods messages is disconnected.
- The tablet bus type comes from the `id/bustype` attribute of the kernel input device.
- GSettings aborts on a missing key. A search of the compositor, the Gnoblin sources, the portal backend and libgxdp for
  the names of the keys that 46.1 lacks finds only reads that `0015`, portal `0004` or an earlier key check covers. A
  key name built at run time can escape such a search, and nothing has run in a session.

### The apk packages

A prefix built on Fedora links glibc and does not run on Alpine, which uses musl. The apk packages
of the shared set therefore come from a second shared prefix that the shared stage of `release.yml` builds on Alpine
edge (the rolling target with `"shared_source": true`) with the same `scripts/build-prefix.sh`.
`scripts/install-alpine-build-deps.sh` installs its build requirements.

- Alpine edge builds the apk packages of the shared set.
- Each Alpine release branch from `build_from` up builds a musl prefix on its own image and writes
  its own apks with the `other` lane, as an install check only (`testpkgs-<tag>-alpine<N>-<arch>`).
  No GitHub release and no repository takes them. One apk set cannot serve two branches: an apk
  binds the library sonames of the branch that built it (Alpine 3.21 and 3.22 have `libdisplay-info.so.2`,
  3.23 and newer have `.so.3`), and musl has no symbol versions, so a program built against a newer
  libinput fails at start on an older one that has the same soname. So an apk set is built on the
  branch that it is for.
- `scripts/install-alpine-build-deps.sh` installs an optional package, such as `libglycin-dev`, only
  when the index of the branch has it. Alpine 3.23 and older have no Glycin.
- Dependencies name each library as `so:<soname>`, which `apk` resolves itself.
- apk has no epoch, so an apk is ordered by version and release only.
- The apk packages of the shared prefix are release assets and are in the signed repositories.
  `pkgrepo` writes one `APKINDEX.tar.gz` for each architecture, signed with
  `REPO_APK_PRIVATE_KEY`, under `alpine/edge/<arch>/` of the channel.

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
2. **Packages.** From a checkout of the same ref, on each architecture, it builds a prefix on the
   image of every target that has its own prefix, writes the packages with pkgsmith, installs them
   on a clean system of every target, installs stock GNOME beside them and removes Gnoblin. The
   targets come from `scripts/supported-targets.py`.
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

The package suite (`scripts/run-package-tests.sh`, `design/package-testing.md`) installs, runs, upgrades and removes
the packages on a clean system of each release, the graphical VM test boots each release with them, and the release
gate keeps any package whose target failed a stage out of the release. Stock GNOME must be unchanged afterwards, and
no Gnoblin path may remain.

- The GTK checks are about the Gnoblin packages: `gnoblin` must not require GTK or libadwaita, and no
  Gnoblin package may require GTK 3 or gnome-desktop. The check does not look at what else is
  installed, because a distribution's own GTK 4 can pull GTK 3. On Fedora 42 `gtk4` pulls `SDL3`,
  then `libdecor`, which requires `gtk3`.
- Every package manager command must succeed. apk-tools 2 (Alpine 3.21 and 3.22) marks a package as broken
  when a file of it has no embedded checksum, and then exits 1 in every command. The pinned pkgsmith
  (`scripts/pkgsmith.ref`) writes the `APK-TOOLS.checksum.SHA1` record for every member, symbolic links
  included, so the install checks of Alpine 3.21 and 3.22 pass. The static case `S12` fails an apk
  without it. apk-tools 3 (Alpine 3.23 and newer) accepts a file without a checksum. The triggers of
  Alpine's own packages that fail in a container (`dbus`, `gtk-update-icon-cache`) do not change the
  exit code.
- `install_prepare` of the target runs first in the image, for a tool that the image lacks: `bash` on
  Alpine, and `update-alternatives` on the minimal Leap image, which the `gnome-session` scripts need.

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

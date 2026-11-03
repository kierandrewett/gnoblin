# Package testing

This note describes the test stage that runs after the packages are built and before anything is released or
published. It is internal release engineering material. It lists the failure modes that each layer of the suite must
detect, what each layer proves, and what it cannot prove. The failure modes were written before the tests. Each test
in `tests/packages/` refers to the failure mode that it detects (`S1`, `I4` and so on).

The suite is in `tests/packages/`. `scripts/run-package-tests.sh` runs it for one target. In the release, the `test`
job of each target leg (`.github/workflows/target.yml`) runs it for that target and architecture, and the release
gate (`tests/packages/gate.py`, the `gate` job of `release.yml`) decides which packages may be published.
`tests/packages/README.md` has the commands.

## Rules

- The suite tests the packages that the build jobs made. It does not build Gnoblin again. The reproducibility check
  (`S11`) is the one exception: it writes the packages a second time from the same prefix with the same pkgsmith, to
  compare the bytes.
- The targets come from `scripts/supported-targets.py`. The suite and the workflows name no release, image, family or
  architecture. A target is one release (or one rolling system) on one architecture.
- Every target that produced packages runs every layer: `built` targets, `experimental` targets and the rolling
  targets, on every architecture of the build matrix. No target is skipped to save time. The jobs run in parallel.
- A `planned` target has no packages, so it has no job. The gate gives it a verdict, "not built", with its blocker,
  and the coverage table lists it so. It never reads as tested.
- A layer passes only when every one of its cases passes or is skipped with a stated reason. A case that cannot run in
  the environment of the job is skipped with that reason, and the report says so. A skipped case is never shown as a
  pass.
- A layer that did not run is missing. A missing layer counts as a failure.

## Layers

| Layer        | Where it runs                                          | Input                                          |
| ------------ | ------------------------------------------------------ | ---------------------------------------------- |
| `static`     | The build runner, with no install                      | The package files of the target                |
| `install`    | A clean container of the install image of the target   | The package files of the target                |
| `runtime`    | A second clean container of the install image          | The package files of the target                |
| `repository` | A container of the repository tool (`scripts/pkgrepo`) | The package files of the target                |
| Gate         | One job of the run, after every target leg             | The target list, every verdict, every artifact |

### Layer 1: static

The static layer reads each package file with the tools of its format and the Python standard library. It installs
nothing.

Failure modes:

- `S1` A package of the set is missing, or is there twice: gnoblin, gnoblin-x11-frames, xdg-desktop-portal-gnoblin,
  gnoblin-ananicy-rules or gnoblin-geoclue-integration, in the format of the target. A glibc target that installs the
  shared packages also gets the tar packages, and the same five must be there.
- `S2` The metadata is wrong: an unknown name, a version other than the version of the tree (or of the portal source,
  for the portal backend), a release that does not carry the release of the target (`.fc43`, `.leap16.0`,
  `~deb13`) or carries one on a shared package, an architecture other than the architecture of the target (or an
  architecture-independent one for a package that holds compiled code), an empty licence, or no description.
- `S3` A dependency is wrong: gnoblin asks for GTK or libadwaita, a package asks for a development package
  (`pkgconfig(...)`, `-devel`, `-dev`), a package declares a conflict with or replaces a stock GNOME package, or the
  frames client and the portal backend do not ask for exactly the gnoblin of their own version.
- `S4` A file is outside the paths that Gnoblin owns, or replaces a stock GNOME file: a file in `/usr/bin` other than
  the two commands, a Mutter or GNOME Shell file at a system path, a schema or a `.desktop` file outside the private
  prefix other than the public entries. The rules are the ones of `scripts/check-rpm-isolation.py`, applied to every
  format.
- `S5` A file has the wrong owner or mode: not `root:root`, set-user-ID or set-group-ID, writable by group or others,
  or a program without the execute bits.
- `S6` A symbolic link points to a file that the package set does not hold, or out of the package set by `..`.
- `S7` A file holds a path of the build machine: a home folder, `/root/`, `/var/tmp/`, the container paths of the
  prefix build (`/out/prefix`, `/work/`). A later run on a user system would look for files there.
- `S8` A `.desktop` file does not validate (`desktop-file-validate`).
- `S9` The GSettings schemas do not compile, or the compiled schema file is missing.
- `S10` A systemd unit does not pass `systemd-analyze verify`, or a D-Bus service file names a program that the
  package set does not hold.
- `S11` The package files are not reproducible: writing them again from the same prefix, with the same pkgsmith and
  the same arguments, gives other bytes.
- `S12` A checksum or signature is missing or wrong: a payload digest of an rpm, an entry of the `md5sums` of a deb, a
  `sha256digest` of the `.MTREE` of an Arch package, the SHA-1 checksum of an apk member (every member, symbolic links
  included: apk-tools 2 marks a package broken without it), or the `datahash` of an apk. Package signatures are made by
  the repository tool, so the repository layer checks them (`P3`).
- `S13` The ABI is inconsistent: an ELF file needs a shared library that the package does not ask for, the package asks
  for a library that no ELF file needs, or a versioned symbol (`GLIBC_2.43`, `LIBINPUT_1.30`) needs a newer library
  than the version that the package asks for. rpm and apk name each library in their dependencies, so the static layer
  checks both directions there. deb and Arch name packages, so the static layer checks the version floors there and
  the install layer checks the package names in the image (`I5`).

Until `scripts/pkgsmith.ref` moves to a pkgsmith that writes a checksum for each symbolic link of an apk, every apk
fails `S12`, and `I12` fails because `apk audit` cannot verify the links. That is the expected result: the layers
detect the defect that broke the Alpine 3.21 and 3.22 installs. The Alpine targets stay out of the publish set until
the pin moves, and Alpine edge, a built target, fails the release. The suite and every build job take pkgsmith from
the one pin in `scripts/pkgsmith.ref`, so moving the pin fixes all of them at once.

### Layer 2: install

The install layer starts a clean container of the install image of the target and runs `tests/packages/install.sh`. It
contains the checks of the former `scripts/check-package-install.sh`, which it replaces.

Failure modes:

- `I1` gnoblin does not install with the package manager of the release, or installs only with recommended packages,
  stock GNOME or a second Mutter.
- `I2` gnoblin pulls GTK 4 or libadwaita where the format allows it not to (rpm and deb, with recommended packages
  refused), or gnoblin-x11-frames does not behave as the format documents: optional on rpm and deb, a dependency of
  gnoblin on Arch and apk.
- `I3` A public file or the private runtime is not where the documentation says.
- `I4` An ELF file does not resolve in the image with no `LD_LIBRARY_PATH`: a library is not found, a symbol version is
  not found, or (with `ldd -r`) a symbol is undefined.
- `I5` A library that an ELF file needs comes from a package that gnoblin does not ask for (it is there only by chance),
  or the package asks for a library package that no ELF file needs.
- `I6` A program does not start: `gnoblin --version`, `gnoblinctl --version`, `gnoblinctl --help`, `gdctl --help`.
- `I7` The default Lua configuration is not in the binary, or `gnoblinctl init` cannot write the editable tree.
- `I8` The schemas are not compiled in the private prefix, or GSettings cannot read the Mutter schemas from it.
- `I9` The session is not in the list of a display manager: no `gnoblin.desktop` in `/usr/share/wayland-sessions`,
  or its `Exec` or `TryExec` is not an executable file.
- `I10` An optional package does not install beside gnoblin, or a Gnoblin package asks for GTK 3 or gnome-desktop.
- `I11` Stock GNOME does not install beside Gnoblin, or a file of stock GNOME belongs to a Gnoblin package.
- `I12` A reinstall of the same files fails, or leaves a file that the package manager reports as changed.
- `I13` The upgrade from the version that is published at `https://repo.gnoblin.org` for the family and release of the
  target fails, or leaves the old version. When no tree is published for the target, or every published version is
  not older than the new one, the case is skipped with that reason.
- `I14` The removal of Gnoblin leaves a Gnoblin file, or changes or removes a stock GNOME package.
- `I15` A tar package of a glibc target does not extract, does not resolve, or does not start. Its removal by the list
  of its files leaves a file.

### Layer 3: runtime

The runtime layer starts a second clean container of the install image, installs the packages, and installs the test
tools: Mesa (the software renderer), the D-Bus daemon, `wayland-info`, `grim`, `swaybg` and Python. It runs
`tests/packages/runtime.sh` as a user that is not root.

The installed `gnoblin` starts the session as it does at a login: the guardian, the Lua supervisor and the compositor.
A container has no seat and no DRM device, and the packaged Mutter has no development viewer. So the test puts a
wrapper at `/usr/lib/gnoblin/bin/gnoblin` that adds Mutter's `--headless` and `--virtual-monitor 1280x720` when the
guardian starts the compositor. Everything else is the installed program and its defaults.

Failure modes:

- `R1` The session does not start, or exits, within 60 seconds: no Wayland socket, no compositor bridge socket.
- `R2` A Wayland global that the build holds is not advertised. The expected list is the inventory in
  `tests/packages/globals.json`, limited to the interfaces that the installed binaries hold. A release with an older
  libwayland or wayland-protocols compiles some interfaces out, so its list is shorter. An interface that needs a DRM
  device, Xwayland or a frame renderer client is listed with that reason and is not expected in a container.
- `R3` The compositor bridge does not greet, does not answer a ping, or reports no API version.
- `R4` `gnoblinctl` does not reach the session: `ping`, `version`, `status`, `output list`.
- `R5` The default configuration does not load: `gnoblinctl init` then `gnoblinctl config reload` fails. Or a broken
  configuration is accepted, or stops the session.
- `R6` A screenshot through `zwlr_screencopy_manager_v1` (`grim`) fails, has the wrong size, or does not show a
  layer-shell client that fills the output with one colour (`swaybg`).
- `R7` The session does not stop cleanly on `gnoblinctl logout`, a process dumps core, or the logs hold a `CRITICAL`
  or `ERROR` line, a segmentation fault or an address sanitiser report that is not on the allow-list
  `tests/packages/log-allowlist.txt`. Each entry of the allow-list gives the reason that the line is expected in a
  container.

What the runtime layer proves: the installed compositor starts from the packages, serves Wayland clients, composites a
client surface, captures it, answers the bridge and `gnoblinctl`, loads and validates the Lua configuration, and stops
cleanly. What it does not prove: rendering on a GPU, KMS and DRM output, input devices, a graphical login through a
display manager, Xwayland, PipeWire screen casting, and the portal backend at run time. The graphical tests
(`design/graphical-testing.md`) cover some of these on other machines.

### Layer 4: repository

The repository layer builds a signed repository tree from the packages of the target with the repository tool and
throw-away keys, as the repository stage does, and checks it the way a client would.

Failure modes:

- `P1` The repository tool refuses a package: an unknown format, a file name or metadata that it cannot read.
- `P2` The metadata of the tree does not match the files: a size or a SHA-256 of `manifest.json` or of the format
  metadata is wrong, or a package of the target is not in the manifest.
- `P3` A signature does not verify with the test key: the rpm signatures, the APT `InRelease`, the pacman `.sig`
  files, the Alpine index.
- `P4` A package of a target that is not published (the `testpkgs` artifacts: the per-release deb, apk and openSUSE
  Leap rpm) is in an artifact that the release or the repository takes.

### Release gate

The gate is the last job before `publish` and the repository stage. It reads the target list, the verdict of every
target leg (the stage records of the prefix and packages jobs, the suite's `report.json` and the graphical
`result.json`), the stage records of the shared stage, and every published package artifact of the run. The package
suite is one of the four stages it checks: prefix, packages, suite and graphical. `design/release-pipeline.md` has
the whole gate.

Failure modes:

- `G1` A target whose packages were written has no report for one of its architectures. Its suite stage fails
  ("no report.json: the package suite did not finish"), it is not "not run".
- `G2` A report has fewer layers than the suite defines, or a layer did not pass.
- `G3` A package of a published artifact belongs to no target, so no test covered it.
- `G4` A per-release deb or apk, or any file of a `testpkgs` artifact, would be published, in the GitHub release or in
  a repository suite such as `deb/forky`.
- `G5` A package is published although one target or architecture that installs it did not pass every stage: its
  prefix, its packages, every layer of the suite, and the graphical VM test on that architecture.

The gate writes the publish set: the packages of the published artifacts whose every target and architecture passed
every stage. `publish` and the repository stage take only that set, also when the gate failed: a target that passed
is released, and one that failed holds no other back. A failure of a `built` target fails the run (the run is red).
A failure of an `experimental` target does not fail the run. Either way its packages leave the publish set, and the
summary names each exclusion (`excluded: <target> <arch>: <stage>: <detail>`). An empty publish set fails the run and
publishes nothing. A package that is the
same file on two architectures (a `noarch` rpm, an `all` deb, an `any` Arch package) is published only when every
architecture passed. The amd64 result never stands in for arm64.

## Reports and evidence

Each test job writes one folder, uploaded as the artifact `package-tests-<release tag>-<slug>-<arch>`:

| File                | What it holds                                                                     |
| ------------------- | --------------------------------------------------------------------------------- |
| `report.json`       | The target, the layers that the suite defines, the result and the cases of each   |
| `junit.xml`         | The same results in JUnit XML                                                     |
| `summary.md`        | One table for the job summary                                                     |
| `evidence/command`  | The command that repeats the run on a laptop                                      |
| `evidence/env.txt`  | Host, container engine, image digests, tool and pkgsmith versions, package hashes |
| `evidence/<layer>/` | The logs of each layer: the package manager, the compositor, the session          |

The gate writes `coverage.md`, `coverage.json` and the publish set as the artifact `publishable-<release tag>`.

## Negative tests

Each layer has a deliberately broken package that it must reject. `tests/packages/mutate.py` makes the broken copy
from a real package, and `scripts/run-package-tests.sh --mutate NAME` runs the layer against it and passes only when
the layer fails. The mutations:

| Mutation           | Layer        | What is broken                                                  | Failure mode |
| ------------------ | ------------ | --------------------------------------------------------------- | ------------ |
| `foreign-file`     | `static`     | gnoblin also installs `/usr/bin/mutter`                         | `S4`         |
| `build-path`       | `static`     | a file of gnoblin holds `/work/build`                           | `S7`         |
| `drop-copyright`   | `static`     | the gnoblin deb has no `/usr/share/doc/gnoblin/copyright`       | `S2`         |
| `drop-library-dep` | `install`    | the gnoblin deb no longer asks for the package of libinput      | `I5`         |
| `drop-gobject-dep` | `install`    | the gnoblin deb no longer asks for PyGObject, which gdctl needs | `I6`         |
| `rename-global`    | `runtime`    | libmutter names the layer-shell global `zwlr_layer_shell_v0`    | `R2`         |
| `corrupt-payload`  | `repository` | one byte of the payload of the gnoblin rpm is changed           | `P1`         |

The runner passes a negative test only when the named case failed, not merely the layer. A removed Mutter plugin is not
a usable mutation: the Gnoblin compositor registers its own plugin type and never loads `libdefault.so`.

The selftest (`tests/packages/selftest/`) runs the static layer, the report writer and the gate on a small package set
that the pinned pkgsmith writes from a fixture prefix, with no network. It runs in `tests/check-fast.sh` and in the
`Verify installation guide` workflow. It includes the gate cases of a missing architecture, a missing layer and a
failed experimental target.

## Defects the suite found

The first runs found these defects in packages built from the tree. Each was fixed in the same change, and each keeps
a negative test that still fails:

- `S7`: the programs held absolute paths of the build machine: `__FILE__` in assertions (`/work/src/...`) and the
  build-tree fallback of the version file in `gnoblinctl`. `scripts/build-prefix.sh` now maps the checkout to `.`
  with `-ffile-prefix-map`, and `CMakeLists.txt` defines the fallback only outside the system layout.
- `S2`: the debs carried no licence: a deb has no licence field, and pkgsmith wrote no copyright file.
  `gnoblin_package()` in `pkgsmith/common.star` now adds `/usr/share/doc/<package>/copyright` (DEP-5) to every deb.
- `I6`: `gdctl`, which the QEMU and window-effect tests use, is a Python script that needs PyGObject, and no package
  asked for it. `CORE` in `pkgsmith/runtime.star` now does, in every format.
- `I5`: the portal backend asked for libsecret, which it neither links nor uses (a carry-over from the old Arch
  recipe). `PORTAL` no longer names it.

## Local proof, 2026-11-03

Every target of the generated list that has packages was built from this tree on amd64 with the repository's scripts
(the prefix in a clean container of the release, the packages with the pinned pkgsmith 1041e4c), then tested with
`scripts/run-package-tests.sh` and rootless podman. The reports, the evidence and the commands are in
`/var/tmp/PackageTests/reports/`, the negative tests in `/var/tmp/PackageTests/negative/`, and the gate output in
`/var/tmp/PackageTests/coverage/` on the machine that ran them.

| Targets                                                                                                       | Result                                                                                    |
| ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Fedora 41 to 45, Ubuntu 26.04, Debian 13, openSUSE Leap 16.0, openSUSE Tumbleweed, Arch Linux, Debian testing | all four layers pass                                                                      |
| Alpine 3.23, 3.24 and edge                                                                                    | `S12` and `I12` fail: no checksum on the symbolic links of the apks (the pkgsmith pin)    |
| Alpine 3.21 and 3.22                                                                                          | `S12` fails, and apk-tools 2 refuses the packages, so the install and runtime layers fail |

The same apk check passes on an apk that pkgsmith c810012 writes, and fails on one that 1041e4c writes. Every
mutation of the negative tests made its layer fail its case. The gate on these reports excludes the Alpine targets,
fails the run because Alpine edge is a built target, and fails it for every arm64 target, which has no report.

One test job took from 1.5 to 4.5 minutes on this machine: static 3 to 9 seconds (the reproducibility build adds about
20 seconds), install 55 to 185 seconds, runtime 30 to 60 seconds, repository 1 to 2 seconds after the image of the
repository tool exists (about 50 seconds to build it once).

Not run: any arm64 target (no arm64 machine), a GPU, KMS, a graphical login, and the workflows on GitHub. Ubuntu 24.04
was added to the target list during this work and was not built here. The upgrade case `I13` ran against the
published tree only where one exists for the target and holds an older version: for the targets above it was skipped
with the reason in the report (no tree, or no older version than 0.1.13).

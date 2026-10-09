# Scripts

Most of these scripts are not meant to be run by hand. `./build.sh` and CMake run the build
ones, the workflows run the release ones, and the rest are tools. This page says which is
which, so you can tell what a script is for before you run it.

## What to run

| You want to                   | Run                    |
| ----------------------------- | ---------------------- |
| See what a build would change | `./build.sh --dry-run` |
| Build                         | `make` or `./build.sh` |
| Add the login entry           | `make install`         |
| Run the fast checks           | `make check`           |
| Run the CTest suites          | `make test`            |

`./build.sh` is the build. `make` is a short way to run it, and holds no logic of its own.

## Which files are public

No script names the public files. `cmake/public-entries.cmake` reads the finished prefix and lists every file with `gnoblin` in its name under `share/wayland-sessions`, `share/xdg-desktop-portal`,
`share/dbus-1/services`, `lib/systemd/user` and `share/man/man1`. The build writes the list to
`share/gnoblin/public-entries.txt` in the prefix. `make install` and `cmake/system-layout.cmake` both use it. To ship a new login or portal file, install it into the prefix under one of those directories with a name that
contains `gnoblin`. Nothing else needs to change.

## Build

Run by `./build.sh` through CMake and `scripts/build-component.sh`. Run them by hand only to
debug a step.

| Script                                  | What it does                                                                  | Run by                                  |
| --------------------------------------- | ----------------------------------------------------------------------------- | --------------------------------------- |
| `prepare-build-sources.sh`              | Check out the pinned submodules, or unpack a source bundle                    | CMake                                   |
| `apply-patches.sh`                      | Reset a submodule to its tag, copy the overlay, apply `patches/<name>/`       | `build-component.sh`, `make-tarball.sh` |
| `copy-overlay.sh`                       | Copy the Gnoblin-owned overlay files into a submodule                         | `apply-patches.sh`                      |
| `subproject-state.sh`                   | Refuse to reset a submodule that has local changes                            | `apply-patches.sh`                      |
| `ensure-release-subprojects.sh`         | Check that the submodules match the release tags                              | `build-component.sh`                    |
| `checkout-submodules-with-retry.sh`     | Fetch submodules, retrying GNOME GitLab errors                                | workflows                               |
| `retry-command.sh`                      | The retry function used by the script above and by `prepare-build-sources.sh` | those scripts                           |
| `check-build-deps.py`                   | Check that the development libraries the pinned sources need are installed    | CMake                                   |
| `build-component.sh`                    | Build one Meson project (Mutter or the portal backend) into the prefix        | CMake                                   |
| `build-identity.py`                     | Write the build identity that `--version` prints, with the build time         | CMake                                   |
| `embed-config.py`                       | Embed the default Lua configuration tree in a C file                          | CMake                                   |
| `generate-mutter-keybinding-catalog.py` | Export Mutter's keybinding descriptors for the Lua API                        | `build-component.sh`                    |
| `build-adwaita-hyprcursor.py`           | Package the Adwaita cursor vectors for Hyprcursor                             | `install-session.sh`                    |
| `build-frame-renderers.sh`              | Build the optional window frame renderers                                     | by hand, tests                          |
| `install-session.sh`                    | Install the session files, units, schemas and man pages into the prefix       | CMake                                   |
| `cmake/system-layout.cmake`             | Add the public entries a package ships outside the prefix                     | CMake (`--layout system`)               |
| `build-man-pages.py`                    | Write `gnoblin(1)` and `gnoblinctl(1)`                                        | `install-session.sh`                    |
| `gnome-versions.py`                     | Read, check and advance the pinned GNOME version                              | `build.sh`, workflows                   |
| `gnoblin-version.py`                    | Read the Gnoblin release version                                              | scripts, workflows                      |

## Your session

| Script                           | What it does                                                           | Run by                        |
| -------------------------------- | ---------------------------------------------------------------------- | ----------------------------- |
| `register-session.sh`            | Add the login entry, link `gnoblinctl` and the man pages. Needs `sudo` | `build.sh --register-session` |
| `run-gnoblin-devkit.sh`          | Start the compositor in a nested viewer window                         | `build.sh --preview`, tests   |
| `run-clean-devkit.sh`            | Same, with a clean configuration                                       | by hand                       |
| `devkit_dbus.py`                 | Write the private D-Bus configuration a nested run uses                | the devkit scripts            |
| `devkit-document-portal-stub.py` | A stand-in document portal for nested runs                             | `devkit_dbus.py`              |
| `gnoblin-state.sh`               | Write persistent development logs safely                               | `run-gnoblin-devkit.sh`       |
| `gnoblin-test-ibus.sh`           | Start an IBus daemon on the private bus                                | the devkit tests              |

## Releases and packages

Run by the workflows and by `./build.sh package`.

| Script                                      | What it does                                                            | Run by                                   |
| ------------------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------- |
| `build-release-assets.sh`                   | Write the source bundle, Arch recipes and checksums (`--srpm` for RPMs) | `build.sh package`                       |
| `make-tarball.sh`                           | Write the patched Mutter or portal source archive                       | `build-release-assets.sh`                |
| `list-tarball-sources.sh`                   | List the files a source archive needs                                   | `make-tarball.sh`                        |
| `make-gsettings-desktop-schemas-tarball.sh` | Write the pinned GSettings schema archive                               | `build-release-assets.sh`                |
| `build-source-bundle.sh`                    | Join the archives into the source bundle                                | `build-release-assets.sh`                |
| `build-srpm.sh`                             | Build one Fedora source RPM                                             | `build-release-assets.sh`, `release.yml` |
| `stage-rpm-sources.sh`                      | Stage the loose RPM source files of a patched subproject                | the openSUSE build chain                 |
| `sync-package-manifest.py`                  | Check or write the recipes from `packaging/native-packages.json`        | workflows, by hand                       |
| `check-release-tag.sh`                      | Check a release tag against the version in the tree                     | `release.yml`                            |
| `check-rpm-isolation.py`                    | Reject an RPM that could replace GNOME files                            | `opensuse-rpm.yml`                       |
| `check-packaging-targets.py`                | Validate the package target list                                        | a workflow                               |
| `probe-rpm-target.py`                       | Record the library floor of a distribution image                        | a workflow                               |
| `install-arch-build-deps.sh`                | Install what the PKGBUILDs declare                                      | `release.yml`, `verify.yml`              |
| `install-system.sh`                         | Install the published Fedora packages from COPR                         | by hand                                  |
| `publish-copr.sh`                           | Send source RPMs to COPR                                                | `copr.yml`                               |
| `prepare-apt-pages.py`                      | Add the already-published APT archive to the docs site                  | `docs.yml`                               |

## Patches

| Script                           | What it does                                                  |
| -------------------------------- | ------------------------------------------------------------- |
| `manage-patches.py`              | Export and check the Gnoblin patches with one author identity |
| `gen-gnoblin-protocols-patch.sh` | Regenerate the protocol patch (see `src/protocols/README.md`) |

## Checks, tools and documentation

| Script                                               | What it does                                          |
| ---------------------------------------------------- | ----------------------------------------------------- |
| `quality.sh`                                         | Lint and format (`./scripts/quality.sh lint`)         |
| `markdown-style.py`                                  | The Markdown review that the pre-commit hook runs     |
| `format-qt.py`                                       | Format QML and JavaScript that has Qt directives      |
| `docs-vitepress-postbuild.mjs`                       | Post-process the built documentation site             |
| `capture-doc-examples.sh`, `composite-doc-cursor.py` | Capture a clean screenshot of a nested session        |
| `gnoblin-issues`                                     | Sync Beads issues with GitHub                         |
| `qemu-e2e`, `qemu-e2e-provision`, `qemu-e2e-soak`    | The graphical test guest (`tests/qemu-e2e/README.md`) |
| `report-app-e2e-failures.py`                         | Summarise application test failures for an issue      |
| `build-frame-probe.sh`                               | Build the framebuffer reader for window damage tests  |
| `build-mtk-region-copy-probe.sh`                     | Build a probe that counts region copies               |

## Not called by anything

These three have no caller in the repository. They may still be useful by hand. If you do not
use them, they can be removed.

- `run-normal-config-devkit.sh`: a nested session that copies your normal configuration.
- `build-hyprcursor-test.sh`: builds the cursor bridge check.
- `report-mtk-region-copy-probe.sh`: reads the output of the region copy probe.

# Scripts

The Rust build needs no script: `cargo xtask` and `just` do the work (see `justfile` and `docs/source-development.md`). The
scripts here are the tools CI, the parity and conformance harnesses, and the issue and documentation checks call. This page
says what each is for, so you can tell before you run it.

## Build inputs

| Script                           | What it does                                                                                                                | Run by                                                           |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `ci-install-system-libraries.sh` | Install the system packages the Rust workspaces need, on a hosted runner or in a Debian, Fedora, Arch or openSUSE container | `rust.yml`, `matrix.yml`, `wlcs.yml`, `parity-reference.sh`, you |
| `native-probe.sh`                | Probe one target image for the native libraries the workspace needs, and store the report                                   | by hand (`cargo xtask native-probe` documents it)                |
| `oci-rootfs.py`                  | Unpack a container image from its registry into a plain directory, with no container runtime                                | `native-probe.sh`                                                |
| `probe-rpm-target.py`            | Record the library floor of a distribution image                                                                            | `rpm-target-probes.yml`                                          |
| `check-packaging-targets.py`     | Validate the package target list in `packaging/targets.json`                                                                | `packaging-targets.yml`                                          |
| `pkgrepo/`                       | Build, sign and upload the RPM, APT, pacman and Alpine repositories (`pkgrepo/README.md`)                                   | `package-repository.yml`                                         |

## CI matrix

| Script                   | What it does                                                                                                     | Run by                               |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| `ci/free-disk-space.sh`  | Remove the toolchains a hosted runner carries and no job uses; does nothing anywhere else                        | `rust.yml`, `matrix.yml`             |
| `ci/distro-test.sh`      | Run `just` recipes in a container of one distribution, as an ordinary user                                       | `matrix.yml` `distro`, you           |
| `ci/vm/run.sh`           | Boot a distribution's cloud image under KVM and check the installed gnoblin on its DRM backend with real clients | `matrix.yml` `drm-vm`, `gpu-vm`, you |
| `ci/vm/guest-session.sh` | Start gnoblin on the DRM backend through seatd inside the guest, with one client                                 | `ci/vm/run.sh`                       |
| `ci/vm/qmp.py`           | Capture the guest's display and send it keys, pointer motion and clicks through QEMU's QMP socket                | `ci/vm/run.sh`                       |

## Conformance and parity

| Script                                                                                                          | What it does                                                                                 | Run by                                   |
| --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ---------------------------------------- |
| `wlcs.sh`, `wlcs-run.py`                                                                                        | Build Canonical's Wayland Conformance Suite and run the core subset against the compositor   | `wlcs.yml`                               |
| `parity-reference.sh`                                                                                           | Build Mutter 51 and the Mutter-based Gnoblin, and run the parity scenarios against all three | `parity-reference.yml`                   |
| `parity-mutter51`, `parity-mutter-gnoblin`, `parity-gnoblinctl`, `parity-launcher-lib.sh`, `parity-proc-shim.c` | Launch the reference sessions the parity runner drives                                       | `gnoblin-parity`                         |
| `reference-session-record.sh`, `reference-session-sessions.py`                                                  | Record what a stock Mutter 51 session puts on the bus and its Wayland socket                 | by hand                                  |
| `reference-session-matrix.py`                                                                                   | Compare the recorded reference session with the adapters' support matrices                   | `tests/reference-session-matrix.test.py` |
| `contract-record.py`                                                                                            | Record what a live Mutter-based session answers on its control socket                        | by hand                                  |
| `real-host-conformance.sh`                                                                                      | Run the nested backend against a real host compositor, Sway or Weston                        | by hand                                  |
| `xr-smoke.sh`                                                                                                   | Whole-stack XR smoke test with a simulated headset                                           | by hand                                  |

## Shell checks

| Script                                    | What it does                                            | Run by          |
| ----------------------------------------- | ------------------------------------------------------- | --------------- |
| `js-typecheck.sh`, `js-lint.sh`           | Type-check and lint the TypeScript shell sources        | `just js-check` |
| `js-types-check.sh`, `js-types-check.mjs` | Prove the editor files `gnoblinctl types init` writes   | `just js-check` |
| `js-shell-screenshots.sh`                 | Take pictures of the shipped shell on a real compositor | by hand         |

## Quality, issues and documentation

| Script                          | What it does                                                    |
| ------------------------------- | --------------------------------------------------------------- |
| `quality.sh`                    | Lint and format (`./scripts/quality.sh lint`)                   |
| `markdown-style.py`             | The Markdown review that the pre-commit hook runs               |
| `gnoblin-issues`                | Sync Beads issues with GitHub                                   |
| `gnoblin-incident-resources.py` | Record bounded DRM ownership observations during a GPU incident |

`just script-tests` runs `tests/check-fast.sh`: syntax checks of these scripts and the tests of `pkgrepo`,
`probe-rpm-target.py`, `reference-session-matrix.py`, `gnoblin-incident-resources.py` and `contract-record.py`.

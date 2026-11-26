# Testing Gnoblin

Choose a check based on the code you changed. The workspace tests, the conformance suites, the nested session and a real
login each verify a different part of the session. `justfile` lists every recipe CI runs.

## Test the shell libraries

The shells have tests in each language:

- The TypeScript tests are in `crates/gnoblin/tests/js_shell.rs` and `crates/gnoblin-runtime-js/tests/shells.rs`, and run
  with `cargo test -p gnoblin-js -p gnoblin-js-types -p gnoblinctl`.
- The Python tests in `crates/gnoblin-runtime-python/tests/shells.rs` run `.py` shells through the Python runtime, and the
  `crates/gnoblin/tests/modular/` sessions also start `shell.py` examples.
- The Rust tests are in `crates/gnoblin/tests/native_shell.rs` and `crates/gnoblin/tests/modular/native_input.rs`, which
  build a Rust shell library and reload it.

Each test runs its shell against a headless compositor in the test's own process, so they need no seat or display. A passing
run proves the shell's behaviour over the compositor's own sockets; it does not prove a graphical login (see
[hardware verification](real-hardware-verification.md)).

## Available checks

| Command                            | What it checks                                                                                                               | Requirement                                                  |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `just workspace-ci`                | The whole root gate: formatting, build, clippy, tests, the xtask rules, the ABI locks and the SDK generator                  | The libraries `scripts/ci-install-system-libraries.sh` lists |
| `just ci`                          | `workspace-ci`, the extension workspace's gate and the core-only build                                                       | The same, and `cargo-deny`                                   |
| `just test`                        | Every crate's unit and integration tests                                                                                     | The same                                                     |
| `just core-only`                   | The core links no std crate, places windows at the origin and refuses portal requests `no-ui`                                | The same                                                     |
| `just renderer-test RENDERER`      | One renderer's tests and a session drawing with it; RENDERER is `software`, `gles` or `vulkan`, and a missing device fails   | Mesa's llvmpipe and lavapipe, which the install script lists |
| `just modular`                     | The shipped binary with shells of every language, reload, the failure modes and the ABI upgrade tests                        | The same                                                     |
| `just pipewire-test`               | The PipeWire export and the compositor's capture end-to-end tests                                                            | A `pipewire` daemon on `PATH`                                |
| `just abi-check`                   | The C ABI lock files and the golden shells of `tests/modular/golden`                                                         | None                                                         |
| `just dist-check`                  | How the session links, and what it installs, in three temporary prefixes                                                     | A C compiler and `pkg-config`                                |
| `just js-check`                    | Type-checks and lints the TypeScript shell sources and checks formatting                                                     | Node.js                                                      |
| `just script-tests`                | Syntax of the scripts, and the tests of `pkgrepo`, the RPM target probe and the contract recording                           | Python 3                                                     |
| `scripts/wlcs.sh`                  | Canonical's Wayland Conformance Suite against the compositor; `tests/wlcs/expected.toml` lists what must pass                | `scripts/ci-install-system-libraries.sh wlcs`                |
| `scripts/ci/distro-test.sh IMAGE`  | `just test` and `just modular` in a container of one distribution, as an ordinary user                                       | docker or podman                                             |
| `scripts/ci/vm/run.sh DISTRO ...`  | The installed gnoblin on the DRM backend of a distribution's own kernel, with real clients and input, in a virtual machine   | KVM and QEMU; `just vm-dist DIR` builds the tree it copies   |
| `scripts/real-host-conformance.sh` | The nested backend against a real host compositor, Sway or Weston, headless                                                  | Sway or Weston                                               |
| `scripts/xr-smoke.sh`              | The compositor, the stock shell, two clients, the XR extension and a simulated headset together                              | Monado and Vulkan; it installs its own packages              |
| `tests/x11-clients-e2e.sh`         | Real X11 clients (xterm, xeyes, xclock, xmessage) reach the size they ask for under the real session, with a screenshot      | Xwayland 24.1 or newer; xterm, x11-apps, grim, foot          |
| `cargo run -p gnoblin-parity`      | `run tests/parity/scenarios` against the Rust candidate; with the reference sessions, Mutter 51 and the Mutter-based Gnoblin | A Fedora machine; `scripts/parity-reference.sh`              |

CI runs `rust.yml`, `wlcs.yml` and `parity-reference.yml`, and `matrix.yml` across the Fedora, Arch, Debian and openSUSE
releases in [platform support](platform-support.md), the three renderers and the DRM backend in a virtual machine of
each distribution. A passing CI job does not prove behaviour on a real seat.

## Test a nested session

`GNOBLIN_BACKEND=nested gnoblin` runs a whole session in a window of your desktop (see [backends](backends.md)), and
`GNOBLIN_BACKEND=headless` runs it with no window, which is what the tests use. `scripts/js-shell-screenshots.sh` takes
pictures of the shipped shell on a real compositor. These sessions are not a login or a sandbox.

## Test compositor changes

Run `just test` for the whole suite, and the crate's own tests while you work. The
tests that need a render node print `skipping` and pass on a machine without one, so use
[hardware verification](real-hardware-verification.md) to check a fresh login, display configuration, input, lock and
logout on a supported machine.

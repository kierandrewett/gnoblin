# Contributing to Gnoblin

Gnoblin is a Wayland compositor and session layer written in Rust. It owns window management, input, login integration
and hardware services, then exposes a small interface for an external shell such as Bingux. Shell widgets belong in the
Bingux repository.

## Development

Install the [Rust workspace prerequisites](#rust-workspace-prerequisites), then build, test and check with `just`, which
holds every recipe CI runs:

```sh
just build             # cargo build --workspace --all-targets
just test              # cargo test --workspace --all-targets
just ci                # everything CI runs: the root workspace, the extension workspace and the core-only build
just script-tests      # the scripts' own tests; they need no build
cargo xtask dist install --prefix PREFIX   # install a release build (just dist PREFIX builds it first)
```

The [scripts map](scripts/README.md) says what each script does and who runs it. The
[testing guide](docs/testing.md) records which checks need a real seat, hardware or a running private session.

## TypeScript shell code

Run `just js-check` to type-check shell sources and lint tracked JavaScript/TypeScript; it also checks TypeScript formatting.
Use these commands when working on one file:

```sh
npx --yes prettier@3.9.6 --write path/to/file.ts
pre-commit run eslint --files path/to/file.ts
scripts/js-typecheck.sh
```

Start each module with a short file header, document public exports, and write comments to explain why a choice is needed rather
than restating the code. A necessary `any` or type assertion needs a one-line comment naming the reason.

## Rust workspace prerequisites

The Rust workspaces (the root workspace and `extensions/gnoblin-xr`) need the compiler that `rust-toolchain.toml` pins
(rustup installs it on first use), `just` and `cargo-deny`, and some system libraries. A bare Ubuntu 24.04 machine has
none of the libraries, and the build stops on the first missing one (`libudev` is the first). Install them with

```sh
scripts/ci-install-system-libraries.sh            # Ubuntu or Debian (apt-get) and Fedora (dnf); uses sudo when not root
scripts/ci-install-system-libraries.sh wlcs       # also what `scripts/wlcs.sh` builds wlcs with
scripts/ci-install-system-libraries.sh list       # print the package names and install nothing
```

The script is the only list of packages: the jobs in `rust.yml`, `wlcs.yml` and `parity-reference.yml` run it too. The
Ubuntu names, the first column, are what `cargo build` and `cargo test --workspace` need, found by building both
workspaces in a root file system that held only `build-essential`, `pkg-config`, `git`, `curl`, `python3` and `nodejs`:

| Package                                                             | Needed because                                                                                                         |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `libudev-dev`, `libseat-dev`, `libpipewire-0.3-dev`                 | `libudev-sys`, `libseat-sys` and `libspa-sys` run `pkg-config` for them in their build scripts                         |
| `libwayland-dev`                                                    | `wayland-sys` runs `pkg-config` for `wayland-client` and `wayland-server` unless its `dlopen` feature is on            |
| `libdrm-dev`                                                        | `drm-sys` and smithay's DRM backend run `pkg-config` for `libdrm`                                                      |
| `libclang-dev`                                                      | `libspa-sys` runs bindgen, which opens `libclang.so`                                                                   |
| `libssl-dev`                                                        | `libsqlite3-sys` compiles SQLCipher, which includes `openssl/crypto.h`                                                 |
| `libxkbcommon-dev`, `libpixman-1-dev`, `libinput-dev`, `libgbm-dev` | the linker asks for `-lxkbcommon`, `-lpixman-1`, `-linput` and `-lgbm`; `cargo check` passes without them              |
| `dbus-daemon`                                                       | the compat adapters' and the services' tests start a private bus with it and fail when it is missing                   |
| `pipewire`, `pipewire-bin`                                          | `gnoblin-pipewire-export`, `gnoblin-capture` and the cast tests start a private daemon and `pw-dump`, and fail without |
| `xwayland`                                                          | three `gnoblin-xwayland` unit tests fail without an `Xwayland` on `PATH`; with it the real X11 tests run               |
| `libegl1`                                                           | smithay opens `libEGL.so.1` at run time and panics when it is missing; it pulls in Mesa's software EGL                 |
| `libvulkan1`, `mesa-vulkan-drivers`                                 | the Vulkan renderer's tests skip, silently, without a loader and a device; with lavapipe 38 of them run                |

The tests need no X server and no GPU. Tests that need a render node (`gnoblin-wayland`, `tests/dmabuf_gpu.rs`) print
`skipping` and pass on a machine without one, and so does a driver that takes a dmabuf that is not GPU memory (Mesa's
llvmpipe). The extension workspace needs none of these packages to build. The `wlcs` group is needed only by
`scripts/wlcs.sh`. The XR smoke test (`scripts/xr-smoke.sh`) installs its own packages and is not part of CI on a
hosted runner.

## The core library in the build tree

`gnoblin` links `libgnoblin-core.so.1` (`NEEDED`, once) through a generated stub, so `cargo build -p gnoblin` succeeds on its own and does not build the library: Cargo cannot make a binary wait for a `cdylib`. The library is not optional at run time: the compositor role is `gnoblin_main`, one of its five exports, so the whole compositor (Smithay, the renderers, the compat adapters, the shell host) is compiled into `libgnoblin_core_impl.so` and not into `gnoblin`, and a launcher without the library cannot run a session. The build order is therefore the library first, the launcher next, shells last: `cargo xtask stage`, then `cargo build -p gnoblin`, then `cargo build -p <shell>` (a shell links the same stub and needs no library to build). The tests of a native shell (`crates/gnoblin/tests/native_shell*.rs`) build the examples and the fault fixture themselves into a target directory of their own (`target/tmp/native-shells`), which needs only the SDK, and run the staged binary; they fail with the loader's message when the library was not staged. `crates/gnoblin/build.rs` gives the binary a run path of `$ORIGIN:$ORIGIN/../lib:$ORIGIN/../lib64`, so the loader looks for the library beside the binary in `target/<profile>/`, where Cargo writes it as `libgnoblin_core_impl.so`. Something has to name it `libgnoblin-core.so.1` there:

- `cargo build --workspace`, `cargo test --workspace` and any build that includes `gnoblin-core-impl` do it themselves: its `build.rs` links `target/<profile>/libgnoblin-core.so.1` to the library Cargo writes.
- `cargo xtask stage` (`just stage`; `--release` for the release tree) builds `gnoblin-core-impl`, refuses a library `dist-check` would refuse (a missing export, the test driver left in) and makes the SONAME link and the linker link `libgnoblin-core.so`. Run it before `cargo build -p gnoblin` and `cargo test -p gnoblin`, whose integration tests run `target/debug/gnoblin`. Without it the binary stops with the loader's own message, `libgnoblin-core.so.1: cannot open shared object file`; `cargo build -p gnoblin` says so while it builds (a build-script warning that names `cargo xtask stage`) until the tree is staged. A `cargo build --workspace` may print the same note once, when the launcher's build script runs before the core's.
- The stock session also needs `libgnoblin-std.so.1`, beside the core: the compositor role with no shell opens it and std registers the window manager, the portal dialogs and the visual settings (`design/modular-framework.md`, 28). `cargo xtask stage` stages it with the core. A tree without it still runs: the session says so once and runs core-only, with `NullWm` placing windows at the origin and portal requests refused `no-ui`. `GNOBLIN_STD_DIR` names another directory to look in first.

```sh
cargo xtask stage && cargo build -p gnoblin && target/debug/gnoblin --version
```

`GNOBLIN_CORE_LIB_DIR=target/debug` links the real library (its linker name is staged too) instead of the stub. `GNOBLIN_RPATH` replaces the run path when `gnoblin` is built, and an empty value leaves none: a distribution package installs under `/usr`, which the loader searches. `just dist-check` builds the workspace, stages, installs into three temporary trees and runs `cargo xtask dist-check` on the build tree and on each (SONAME and exports of the core, `NEEDED` once, no `gnoblin_*` in a shell, every gnoblin library resolving inside the tree, links, header, `.pc`, manifest, the glibc floor, and `gnoblin --version` run with `LD_LIBRARY_PATH` unset); CI runs it. `design/status/dist.md` has the install layout.

## Build features of the core library

`gnoblin-core-impl` has two build switches, both on by default.

- `drm` is the hardware backend. Without it the build leaves out `gnoblin-backend-drm` and Smithay's udev, libseat and libinput features, so the library links no `libinput`, `libudev` or `libseat`.
- `gles` is the GLES renderer. Without it the build leaves out `gnoblin-render-gles`.
- `drm` implies `gles`, because DRM scan-out draws with GLES only. `--no-default-features` is the smallest build: nested and headless backends with the software renderer, and Vulkan.
- Each switch is forwarded: `gnoblin-core-impl/drm` turns on `gnoblin-compat-host/drm` and `gnoblin-compositor/drm`, and `gles` goes the same way down to `gnoblin-backend/gles`.
- A crate that depends on one of these crates sets `default-features = false` to keep the switch working. Its default would turn the feature back on.
- With `drm` off, `compositor.backend = drm` is refused with `this build has no DRM backend; rebuild with feature drm`, and `auto` does not pick it.
- With `gles` off, `compositor.renderer = gles` is refused naming the setting, and the log names the feature (`this build has no GLES renderer`). The software renderer is unchanged.

## Full suite

```sh
cargo test --workspace --no-fail-fast --locked
```

Run it on Agent37 through `lanes/a37-gate.sh run ID TREE test --workspace --no-fail-fast --locked`, with the output
sent to a log file. Measured on a 4-CPU instance (lane FinalGreen, 2026-11-14): a warm run takes 6 minutes (336 s and
352 s; 426 s after a rebuild, 709 s on a fresh instance's first run); `cargo test -p gnoblin --test js_shell` takes
47 s alone and 85 to 300 s with busy loops of twice `nproc` running beside it. One call of the helper ends at about
15 minutes whatever `--timeout` it is given (the exec endpoint's own limit), so a loop of `js_shell` runs is split into
calls of ten plain runs or five loaded ones. `--no-fail-fast` makes one failure list every other failure instead of
hiding them.

Shell tests wait on observed state with a deadline that names what it waited for and the last state it saw, never on
sleeps or on the time a fade or a transition takes. Two things a wait must not read are the script's word for what the
compositor holds (a View's `input_mode` that the script wrote is not the one the pointer is routed by until the
compositor has it) and a rectangle taken before the window has a size. A claim that something does not happen is made
after an event that the compositor sends later than the thing it denies (a marker View the pointer moves onto), not
after a pause. Also run `cargo test -p gnoblin --test js_shell` ten times plain and five times with busy loops of twice
`nproc` beside it; the real-compositor tests share the machine with fifty-four siblings, and most of their races only
show then.

A failing test is never labelled "unrelated" or "a load flake" without a baseline run of the same test on a pristine
`origin/rust-compositor` tree. Until the baseline is red too, the failure belongs to the change under test. A test that
fails once is not flaky until the failure is understood: run it alone thirty times plain and thirty times loaded, find
whether the product, the test's wait or the machine's speed is at fault, and fix that, with a regression test when the
product was. A longer deadline or a retry is not a fix. A flake the suite shows only in a full parallel run is
reproduced by running several copies of the test binary at once (`js_shell` under eight workers and eight busy loops
showed the shell reading a stale `decoration` about once in sixty runs).

## Core and std: providers

The compositor is split into core and std (`design/modular-framework.md`, sections 9 and 28). Core is mechanism and a
minimal safe default; what the desktop does by convention is a provider, and providers are std's. `libgnoblin-std.so.1`
registers them with the core through the C ABI (`provider_register`, with the vtables of `gnoblin-core-abi`), so the core
carries no std code and no cargo feature chooses between the two:

| Provider                                                                                                                          | Lives in                                                                                        | With no std loaded                                                                           |
| --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Window manager (placement, focus on map and on click, focus rotation, maximise and fullscreen fitting)                            | `crates/gnoblin-std-wm` (`Fallback`), registered by `crates/gnoblin-std-impl/src/providers.rs`  | `NullWm` from `gnoblin-wm-api`: windows at the origin at their natural size, no focus policy |
| Portal dialogs                                                                                                                    | `crates/gnoblin-std-portal` (`PortalUi`), registered the same way                               | `NoUi`: every portal request is refused `no-ui`, nothing is granted                          |
| `window-management`, `accessibility` and `compositor` sections (policy, titlebar gestures, magnifier, colour filter, visual bell) | the configuration observer in `crates/gnoblin-std-impl/src/providers.rs`, calling `setting_set` | not read; the settings keep their defaults                                                   |

`crates/gnoblin-provider-wire` is the one place that knows both ends of each vtable: std exports a Rust provider as a
vtable, and the core wraps a registered vtable as the trait the compositor drives. The lock card, the slot mechanism and
the portal's request queue and reply validation stay in core (D7).

```sh
just core-only        # builds the core, fails if a std crate is in its tree, and runs the no-provider suites
```

`just core-only` is the CI job `core-only`: the compositor and the core library build with no std crate in their
dependency tree, a session with no provider places windows at the origin (`crates/gnoblin-wayland/tests/null_wm.rs`) and
refuses portal requests `no-ui` (`crates/gnoblin-compositor/tests/portal_no_ui.rs`).

When you add code or a test:

- Desktop policy goes in a std crate, never in core. That means a placement rule, a default look, a mapping from a
  configuration section to a setting, or anything a tiling shell would replace. Core offers the mechanism (a setting key,
  a vtable) and std decides what it is set to.
- No crate outside the std tier names a std crate as a normal or build dependency: `cargo xtask deps` fails the edge
  (`std-below-core`). A test whose subject is the stock desktop may name one as a development dependency.
- A core test that needs a window manager holds for `NullWm`, or registers a provider in its own session (see below). Do not
  weaken a core test to pass without std.

A session of the compositor's own tests registers the stock providers the way std does, as the traits the provider
slots take: `tests/support`'s `stock_providers()` in `gnoblin-compositor` puts `Fallback` and `PortalUi` in
`SessionConfig::providers`, and `core_only` clears them. The Wayland engine's test sessions take their window manager from
`crates/gnoblin-wayland/tests/stock/mod.rs`: a test that starts a session builds its `Config` from `stock::config()`, never
`Config::default()`, and `gnoblin-std-wm` is a development dependency, so the library holds no provider.

The whole path through the C ABI, a real `libgnoblin-std.so.1` opened by a real compositor process, is
`crates/gnoblin/tests/std_providers.rs`. It needs the std library staged beside the core (`cargo xtask stage`).

## Boundaries

Do not commit build prefixes, generated tarballs or local session configuration.

Name a crate that builds a binary after the binary, so `xdg-desktop-portal-gnoblin`
is the crate and the binary alike. Library crates take their area prefix, such as
`gnoblin-portal-core`.

## Packaging

`cargo xtask dist` installs the session into a prefix and writes `share/gnoblin/install-manifest` (see
[install layout](docs/install-layout.md)); `just dist-check` checks how it links and what it installs. The release
workflow that builds distribution packages from that install, and runs them through a clean-install and login check on
each target, is tracked in [issue 379](https://github.com/kierandrewett/gnoblin/issues/379).
[Package repositories](scripts/pkgrepo/DESIGN.md) describes the signed repository that `scripts/pkgrepo` builds and
`.github/workflows/package-repository.yml` publishes.

Use small conventional commits. Keep unrelated working-tree changes out of a
commit, and explain the user-visible result in the commit body.

## Lint and format

See [Code quality](docs/code-quality.md) for tool setup, language coverage,
whole-repository checks and formatting selected files.

For documentation prose, use `scripts/markdown-style.md` as the writing guide. Jev
review is required for staged Markdown and agent documentation changes. Create
`.env` in the repository root:

```dotenv
GNOBLIN_DOCS_JEV_PROVIDER=typesafe
GNOBLIN_TYPESAFE_API_KEY=replace-with-your-TypeSafe-key
# Or use OpenRouter instead:
# GNOBLIN_DOCS_JEV_PROVIDER=openrouter
# GNOBLIN_OPENROUTER_API_KEY=replace-with-your-OpenRouter-key
# Optional model override:
# GNOBLIN_DOCS_JEV_MODEL=jev-latest
```

`.env` is ignored by Git. Keep the key in that file and do not commit it.
With the provider configured, run:

```sh
python3 scripts/markdown-style.py review --fail-on-flags docs/page.md
```

The defaults are `jev-latest` on TypeSafe and `~typesafe/jev-latest` on
OpenRouter. Reviews use five concurrent requests by default; set
`--concurrency` to change that. Override the model with `--model` or
`GNOBLIN_DOCS_JEV_MODEL`.

The script includes the full Markdown guide in Jev's state and asks it to
classify each guide section. Jev returns a probability for each rule;
`--threshold` (default `0.9`) controls which results are marked for review. The score moves a few points between runs, so a lower
threshold flags pages for reasons that change from one run to the next.
`--fail-on-flags` exits nonzero if any rule is flagged. The report includes the
exact rule text and requires each flag to be fixed before completing the change.

The pre-commit hook sends staged Markdown source and the style guide to the
chosen provider; referenced image files are not sent. Review output uses color
in interactive terminals and plain text when captured. Use `--color never` or
`--color always` to override automatic color. Process environment variables
override values in `.env`.

See the [TypeSafe API reference](https://api.typesafe.ai/docs) and
[OpenRouter Jev guide](https://openrouter.ai/blog/tutorials/how-to-use-jev/)
for provider details. Agents only run Jev automatically when `.env` opts
in with `GNOBLIN_DOCS_JEV_PROVIDER`.

## Issues and commit hooks

Gnoblin uses Beads for its work queue and GitHub Issues as the shared record.
Install `bd`, authenticate `gh` for the repository, then install the hook once
per clone:

```sh
uv tool run --from pre-commit==4.6.2 pre-commit install
```

The hook syncs Beads and GitHub before each otherwise-valid commit. Commits
require a working GitHub connection and issue-write access. To commit offline,
explicitly skip this hook once with
`SKIP=beads-github-sync git commit ...`, then sync before the next commit. See
`.beads/README.md` for filing and importing issues.

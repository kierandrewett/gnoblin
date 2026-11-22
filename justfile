# Canonical task runner for the Rust workspace.
#
# Everything CI runs is a recipe here, so a local `just ci` and the GitHub job
# cannot drift. Tools these recipes expect on PATH: the toolchain pinned in
# rust-toolchain.toml, `just`, and `cargo-deny`
# (`cargo install cargo-deny --locked`).

set shell := ["bash", "-euo", "pipefail", "-c"]

default: ci

# Compile every crate and every target, including tests and examples.
build:
    cargo build --workspace --all-targets

# Rewrite the tree with rustfmt.
fmt:
    cargo fmt --all

# Fail if any file is not rustfmt-clean. CI runs this instead of `fmt`.
fmt-check:
    cargo fmt --all --check

# Lint every target. Workspace lints already deny clippy::all, so warnings that
# only clippy can promote (from dependencies of a future feature set) also fail.
clippy:
    cargo clippy --workspace --all-targets -- --deny warnings

# Run unit tests, including the dependency-rule checker's own tests.
test:
    cargo test --workspace --all-targets

# The tests that start a private `pipewire` daemon and read the frames a stream carries: the export crate's own
# failure-mode tests and the compositor-side capture end-to-end tests. `just test` runs them too; this recipe is the
# subset the CI `pipewire` job runs on a runner that installs the daemon. It needs the `pipewire` binary on PATH
# (`GNOBLIN_PIPEWIRE_BIN` names another), and a test fails, never skips, when there is none.
pipewire-test:
    cargo test --locked -p gnoblin-pipewire-export
    cargo test --locked -p gnoblin-wayland --test view_capture_e2e

# The core with no std code. The
# stock providers are std's and reach the core through `provider_register`, so the core's own build holds none of them.
# This builds the compositor and the core library, fails if a std crate is in their dependency tree, and runs the suites whose
# subject is a session with no provider: `NullWm` places windows at the origin, and a portal request is refused `no-ui`.
core-only:
    cargo build --locked -p gnoblin-compositor -p gnoblin-core-impl
    ! cargo tree --locked -p gnoblin-compositor -p gnoblin-core-impl --edges normal,build --prefix none | grep -E '^gnoblin-(std|a11y)'
    cargo test --locked -p gnoblin-wayland --test null_wm
    cargo test --locked -p gnoblin-compositor --test portal_no_ui

# Run the JavaScript conformance tests, including the ignored acceptance gates.
spec-gate:
    cargo test -p gnoblin-js --test spec_conformance -- --include-ignored
# Enforce the workspace dependency rules (xtask/src/deps.rs) over `cargo metadata`.
deps:
    cargo xtask deps

# Forbid stock logging: no logging crate named outside gnoblin-log and its sinks, and no
# `println!`-style or `tracing::` macro in library sources. `xtask/logging-allow.toml` lists the exceptions.
logging:
    cargo xtask logging

# Write the generated .proto text and sensitive table to target/gen-proto, for
# reading. The build generates them itself, in OUT_DIR, so this is a convenience:
# nothing it writes is checked in.
gen-proto:
    cargo xtask gen-proto

# Print every compat adapter's declared support matrix. Adapters declare theirs in
# `crates/compat/<crate>/support-matrix.json`; with none declared this prints the
# empty set and succeeds.
compat-report:
    cargo xtask compat-report

# Fail on a struct or enum two crates declare twice and `xtask/dupes-allow.toml`
# does not waive. Part of `just ci`. `just dupes-warn` prints the same report without
# failing, for a lane that is mid-migration.
dupes:
    cargo xtask dupes --deny

# The same report, exiting zero so a lane mid-migration can read it.
dupes-warn:
    cargo xtask dupes --warn

# Fail when the C ABI and its lock files disagree (crates/gnoblin-*-abi/abi.lock), or when a golden shell of example 12
# (tests/modular/golden) is not the binary golden.lock records or has stopped looking like a shell.
abi-check:
    cargo xtask abi-check

# Ask whether the stored golden shells are what this machine's toolchain builds from the current SDK sources. Informational: a
# different toolchain or linker may differ, and only an owner replaces them (tests/modular/golden/regenerate.sh).
golden-check:
    tests/modular/golden/regenerate.sh --check

# The modular framework's suites (examples/README.md). Stage the core and std libraries, then run the shipped
# binary with shells of every language on a headless backend: the `modular` target (examples 2, 3, 4, 5, 6, 7, 9, 11 and 17 for scripts,
# the duplicate-core check, the document the script role commits), `native_shell*` (examples 1 and 8, the failures, the verbs),
# `reload_session` and `reload_scripts` (examples 10, 11 and 18 and the bound on retired libraries), `startup` (examples 13 to 16, and
# `--shell NAME` for example 14), `failed_start`, `std_providers` (example 17), and `abi_upgrade` (example 12: the golden shells on the
# next minor, refused on another major, and the dependency validation with `readelf -d`, `nm -D`, `ldd` and `/proc/PID/maps`). Then the
# crates whose tests drive the real core and std libraries with a fake compositor: lifetimes, stale callbacks, generations, the C ABI.
# Add a suite here when it joins the 18.
modular:
    cargo xtask stage
    cargo test --locked -p gnoblin --test modular --test abi_upgrade --test native_shell --test native_shell_failures --test native_shell_verbs --test native_shell_spelling --test std_providers --test reload_session --test reload_scripts --test startup --test failed_start --test roles
    cargo test --locked -p gnoblin-core-impl -p gnoblin-shell-host -p gnoblin-std-impl
    # `gnoblinctl check` is the launcher's `check` role over the staged libraries: the launcher the line above built.
    cargo test --locked -p gnoblinctl

# Fail on a -sys crate whose system package the CI install list lacks (scripts/ci-install-system-libraries.sh).
native-libs:
    cargo xtask native-libs

# Write a readable copy of every generated SDK file to target/sdk (the build renders them itself; nothing generated is committed); see docs/sdk-generator.md.
gen-sdks:
    cargo xtask gen-sdks

# Prove the SDK generator is sound: deterministic render, Python compiles, the surface locks and the C ABI locks hold. Part of `just workspace-ci`.
gen-sdks-check:
    cargo xtask gen-sdks --check

# Prove the golden fixtures of the native std port are what the TypeScript std does now, over three runs
# (crates/gnoblin-std-impl/tests/ORACLE.md). `cargo xtask oracle record` rewrites them.
oracle-check:
    cargo xtask oracle check --repeat 3

# Fail on a documented `gnoblin` name the implemented surface lacks, unless
# `xtask/docs-parity-allow.toml` lists it.
docs-parity:
    cargo xtask docs-parity

# Minimum supported Rust version: `rust-version` in Cargo.toml, measured against the
# exact pinned dependencies. Needs `rustup toolchain install 1.97.0 --profile minimal`
# locally; the CI `msrv` job installs it. Keep this version equal to `rust-version`.
msrv_toolchain := "1.97.0"

# Check every crate and target of both workspaces with the minimum supported toolchain. The extension workspace
# declares the same `rust-version`.
msrv:
    cargo +{{msrv_toolchain}} check --workspace --all-targets --locked
    cargo +{{msrv_toolchain}} check --manifest-path {{ext_manifest}} --workspace --all-targets --locked

# Build the workspace and the XR extension in release mode, then install the session under PREFIX, with DESTDIR
# prepended for staging. Every installed file is listed in share/gnoblin/install-manifest.
# See xtask/src/dist.rs. Remove with `cargo xtask dist uninstall --prefix PREFIX`.
dist prefix="/usr/local" destdir="":
    cargo build --release --workspace
    cargo build --release --manifest-path extensions/gnoblin-xr/Cargo.toml --target-dir "${CARGO_TARGET_DIR:-target}"
    cargo xtask dist install --prefix "{{prefix}}" --destdir "{{destdir}}"

# Make the build tree runnable: build gnoblin-core-impl and name it libgnoblin-core.so.1 beside target/debug/gnoblin, which
# records that name and finds it through its $ORIGIN run path. `cargo build --workspace` does the same; this is the way to
# run what `cargo build -p gnoblin` built. `just stage --release` for the release tree. See xtask/src/stage.rs.
stage *args:
    cargo xtask stage {{args}}

# Check how the session links and what it installs. The debug build tree first (the core staged, the binary recording
# NEEDED libgnoblin-core.so.1 once and running with no LD_LIBRARY_PATH), then installs into three temporary trees (a private
# prefix with lib, a /usr stage with lib64, a /usr stage with a multiarch library directory) and checks each with the rules
# of `cargo xtask dist-check`: the core's SONAME and exports, no Rust runtime symbols, a shell fixture that needs the
# core only and defines no gnoblin_*, every gnoblin NEEDED resolving inside the tree, links, header, pkg-config, the manifest
# and the glibc floor. Needs a C compiler and pkg-config (a machine that links Rust has the first). See xtask/src/dist_check.rs.
dist-check:
    #!/usr/bin/env bash
    set -euo pipefail
    target="${CARGO_TARGET_DIR:-target}"
    scratch="$(mktemp -d)"
    trap 'rm -rf "$scratch"' EXIT
    cargo build --workspace
    cargo xtask stage --no-build
    cargo xtask dist-check --staged --shell "$target/debug/libshell_min.so" --std-shell "$target/debug/libstd_prelude.so" --run
    # A private prefix: the binary finds the core through its $ORIGIN/../lib run path alone.
    cargo xtask dist install --prefix "$scratch/prefix" --profile debug
    cargo xtask dist-check --prefix "$scratch/prefix" --shell "$target/debug/libshell_min.so" --std-shell "$target/debug/libstd_prelude.so" --run
    env -u LD_LIBRARY_PATH "$scratch/prefix/bin/gnoblin" --version
    cargo xtask dist uninstall --prefix "$scratch/prefix"
    # A distribution stage: /usr with lib64 (still inside the run path), then with a multiarch directory, which only the
    # loader's default search path reaches, so it is checked without running the binary.
    cargo xtask dist install --prefix /usr --destdir "$scratch/lib64" --libdir lib64 --profile debug
    cargo xtask dist-check --prefix /usr --destdir "$scratch/lib64" --libdir lib64 --shell "$target/debug/libshell_min.so" --std-shell "$target/debug/libstd_prelude.so" --run
    multiarch="lib/$(uname -m)-linux-gnu"
    cargo xtask dist install --prefix /usr --destdir "$scratch/multiarch" --libdir "$multiarch" --profile debug
    cargo xtask dist-check --prefix /usr --destdir "$scratch/multiarch" --libdir "$multiarch" --shell "$target/debug/libshell_min.so" --std-shell "$target/debug/libstd_prelude.so"

# Check advisories, licences, duplicate versions and git sources.
deny:
    cargo deny check

# The extension workspace (`extensions/gnoblin-xr`) is its own workspace with its own lockfile and target
# directory, so the root recipes above do not reach it. It uses the toolchain and the rustfmt settings at the
# repository root, and the same `deny.toml`.
ext_manifest := "extensions/gnoblin-xr/Cargo.toml"

# Fail if any file of the extension workspace is not rustfmt-clean.
ext-fmt-check:
    cargo fmt --manifest-path {{ext_manifest}} --all --check

# Lint every target of the extension workspace.
ext-clippy:
    cargo clippy --manifest-path {{ext_manifest}} --workspace --all-targets --locked -- --deny warnings

# Run the extension workspace's tests.
ext-test:
    cargo test --manifest-path {{ext_manifest}} --workspace --all-targets --locked

# Check the extension workspace's advisories, licences, duplicate versions and git sources.
ext-deny:
    cargo deny --manifest-path {{ext_manifest}} check

# The extension workspace's whole gate.
ext-ci: ext-fmt-check ext-clippy ext-test ext-deny

# Syntax checks of the scripts and the tests of the package repository tool, the RPM target probe, the reference session
# matrix, the incident resource reporter and the contract recording (tests/check-fast.sh). They need no build.
script-tests:
    tests/check-fast.sh

# Check the TypeScript shell sources, lint all tracked JS/TS, and format-check every tracked TS declaration/source.
js-check:
    scripts/js-typecheck.sh
    scripts/js-lint.sh
    git ls-files -z '*.ts' '*.d.ts' | xargs -0 npx --yes prettier@3.9.6 --check

# The root workspace's whole gate, in the order that fails fastest.
workspace-ci: js-check fmt-check build clippy test deps logging dupes docs-parity abi-check native-libs gen-sdks-check deny

# Everything CI runs: the root workspace, the extension workspace and the core build with no std providers.
ci: workspace-ci ext-ci core-only

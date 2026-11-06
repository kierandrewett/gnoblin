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

# Enforce the dependency rules in design/rust-compositor.md over `cargo metadata`.
deps:
    cargo xtask deps

# Forbid stock logging: no logging crate named outside gnoblin-log and its sinks, and no
# `println!`-style or `tracing::` macro in library sources. `xtask/logging-allow.toml` lists the exceptions.
logging:
    cargo xtask logging

# Forbid a Rust string literal that names the retired src/data tree: the Lua runtime and its assets
# live under data/. `xtask/layout-allow.toml` lists the exceptions.
layout:
    cargo xtask layout

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
# does not waive. Local only: `just ci` stays the commands that need no network, so
# this is not part of it. `just dupes-warn` prints without failing.
dupes:
    cargo xtask dupes --deny

# The same report, exiting zero so a lane mid-migration can read it.
dupes-warn:
    cargo xtask dupes --warn

# Check advisories, licences, duplicate versions and git sources.
deny:
    cargo deny check

# The whole gate, in the order that fails fastest.
ci: fmt-check build clippy test deps logging layout deny

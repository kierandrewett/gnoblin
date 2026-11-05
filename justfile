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

# Write the .proto files and the sensitive-member table from the object schema.
# Commit the result; `cargo xtask gen-proto --check` fails while they are stale.
gen-proto:
    cargo xtask gen-proto

# Fail when a checked-in .proto file is stale, missing or not generated at all.
gen-proto-check:
    cargo xtask gen-proto --check

# Print every compat adapter's declared support matrix. Adapters declare theirs in
# `crates/compat/<crate>/support-matrix.json`; with none declared this prints the
# empty set and succeeds.
compat-report:
    cargo xtask compat-report

# Report structs and enums two crates declare twice. Report-only: migration lanes
# still produce duplicates, so this prints and exits zero. `just dupes-deny` is
# the failing form.
dupes:
    cargo xtask dupes

# The same report, failing on any hit that `xtask/dupes-allow.toml` does not waive.
dupes-deny:
    cargo xtask dupes --deny

# Check advisories, licences, duplicate versions and git sources.
deny:
    cargo deny check

# The whole gate, in the order that fails fastest.
ci: fmt-check gen-proto-check build clippy test deps deny

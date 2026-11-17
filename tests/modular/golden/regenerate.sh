#!/usr/bin/env bash
# Builds the golden shells of example 12 and records them in golden.lock. AN OWNER RUNS THIS, and only when the golden set has
# to change; nothing in CI does, and a pull request that changes bin/ or golden.lock is reviewed as an ABI change.
#
#   GNOBLIN_GOLDEN_OWNER=yes tests/modular/golden/regenerate.sh           build and write bin/ and golden.lock
#   tests/modular/golden/regenerate.sh --check                            build into a scratch directory and say whether the
#                                                                         stored binaries are what this toolchain makes
#
# Why it exists: example 12 proves a shell binary built for ABI 1.0 keeps running on later libraries. That proof is worth
# something only while the binaries are the ones built against ABI 1.0, so they are stored, hashed, and not rebuilt by the
# test. Regenerate only to start the golden set (the first ABI commit) or when a toolchain change makes the stored files
# unusable. Never to make a failing test pass: a failure means the new core broke an old shell. (After ABI 1.0 ships and a
# later minor is released, a golden of that minor is added beside these with the same recipe, and these are kept.)
#
# Recipe (what makes the files small and the build repeatable on one toolchain):
#   - a workspace of its own (Cargo.toml here), `--release`, opt-level s, fat LTO, one codegen unit, symbols stripped (the
#     dynamic table stays: `nm -D` shows `gnoshell_entry_v1` alone), `panic = unwind` (the SDK refuses abort);
#   - the repository's Cargo.lock seeds the dependency versions (copied in, removed after);
#   - `--remap-path-prefix` removes the checkout and cargo home paths, `--build-id=none` removes the build id, and
#     SOURCE_DATE_EPOCH is 0, so the file does not name the machine that made it.
# Two builds on one machine and toolchain produce the same bytes (`--check` says so); another linker may not, which is why the
# test and `cargo xtask abi-check` verify the stored hash and do not rebuild.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
mode="write"
case "${1:-}" in
    "") ;;
    --check) mode="check" ;;
    *)
        echo "usage: $0 [--check]" >&2
        exit 2
        ;;
esac
if [ "$mode" = write ] && [ "${GNOBLIN_GOLDEN_OWNER:-}" != yes ]; then
    echo "regenerate.sh: the golden shells are replaced by an owner only; set GNOBLIN_GOLDEN_OWNER=yes (see the header)" >&2
    exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch" "$here/Cargo.lock"' EXIT
cp "$root/Cargo.lock" "$here/Cargo.lock"

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="--remap-path-prefix=$root=/gnoblin --remap-path-prefix=$cargo_home=/cargo -C link-arg=-Wl,--build-id=none"
export SOURCE_DATE_EPOCH=0
export CARGO_INCREMENTAL=0
# shellcheck disable=SC2086
cargo build --release --manifest-path "$here/Cargo.toml" --target-dir "$scratch/target" ${GNOBLIN_GOLDEN_CARGO_ARGS:-} >&2

core="libgolden_core_shell.so"
std="libgolden_std_shell.so"

if [ "$mode" = check ]; then
    status=0
    for file in "$core" "$std"; do
        if cmp -s "$scratch/target/release/$file" "$here/bin/$file"; then
            echo "$file: the stored binary is what this toolchain builds"
        else
            echo "$file: differs from the stored binary (another toolchain or linker, or the SDK sources changed since golden.lock's base)"
            status=1
        fi
    done
    exit "$status"
fi

mkdir -p "$here/bin"
cp "$scratch/target/release/$core" "$here/bin/$core"
cp "$scratch/target/release/$std" "$here/bin/$std"
chmod 0644 "$here/bin/$core" "$here/bin/$std"

abi_of() { sed -n 's/^abi \([0-9][0-9]*\.[0-9][0-9]*\)$/\1/p' "$root/$1" | head -n 1; }
line_for() {
    local kind="$1" file="$2"
    printf 'shell %s bin/%s %s %s\n' "$kind" "$file" "$(sha256sum "$here/bin/$file" | cut -d' ' -f1)" "$(stat -c %s "$here/bin/$file")"
}
# The commit of the libraries' sources the SDK crates were compiled from. A build on a box that holds the published base plus a
# delta names the base in GNOBLIN_GOLDEN_BASE; otherwise it is HEAD.
base="${GNOBLIN_GOLDEN_BASE:-$(git -C "$root" rev-parse HEAD)}"
{
    echo "# The golden shells of example 12: binaries built once against the ABI named here and run, unchanged, against every later"
    echo "# core and std library. Written by regenerate.sh; reviewed like source. \`cargo xtask abi-check\` verifies the hashes and the"
    echo "# dynamic facts of the files (NEEDED, exports) on every run, and the modular tests run them in real sessions."
    echo "# A line is \`shell <kind> <path> <sha256> <bytes>\`."
    echo "core-abi $(abi_of crates/gnoblin-core-abi/abi.lock)"
    echo "std-abi $(abi_of crates/gnoblin-std-abi/abi.lock)"
    echo "base $base"
    echo "catalog $(sed -n 's/^pub const CATALOG_HASH: u64 = \(0x[0-9a-f]*\);$/\1/p' "$root/crates/gnoblin-core/src/generated/schema.rs")"
    echo "toolchain $(rustc --version)"
    line_for core-only "$core"
    line_for std "$std"
} >"$here/golden.lock"
echo "wrote golden.lock and bin/ (SDK sources of $base)" >&2

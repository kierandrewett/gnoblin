#!/usr/bin/env bash
# Run the parity scenarios against the Rust candidate, stock Mutter 51 and the legacy Gnoblin, on a Fedora 45 machine.
#
# usage: scripts/parity-reference.sh deps | mutter | gnoblin | run [REPORT]
#
#   deps     install what the Mutter build and the workspace build need: dnf packages (root) and rustup
#   mutter   build Mutter 51.0 from the upstream tag into $GNOBLIN_PARITY_MUTTER_PREFIX (default /opt/mutter51); a
#            prefix that already holds bin/mutter is kept
#   gnoblin  build the legacy C Gnoblin with the devkit (its headless backend) into $GNOBLIN_PARITY_GNOBLIN_PREFIX
#            (default /opt/gnoblin) from the legacy tree in $GNOBLIN_PARITY_GNOBLIN_SOURCE (required: a checkout of `main`
#            at the commit .github/workflows/parity-reference.yml pins, with its submodules); a prefix that already holds
#            bin/gnoblin is kept. This checkout cannot be that tree: it no longer holds the Mutter product
#   run      run tests/parity/scenarios with GNOBLIN_PARITY_MUTTER51 set to scripts/parity-mutter51 and
#            GNOBLIN_PARITY_MUTTER_GNOBLIN to scripts/parity-mutter-gnoblin, requiring that every `mutter51` and
#            `mutter-gnoblin` row ran, and write the report to REPORT (default target/parity-report.txt)
#
# The CI job `.github/workflows/parity-reference.yml` runs deps, mutter, gnoblin and run in order in a fedora:45
# container. On a host that cannot run containers the same steps run inside an extracted Fedora 45 root file system, with
# `run` calling the launchers through GNOBLIN_PARITY_MUTTER_ROOT (tests/parity/README.md, "Run the Mutter reference").
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${GNOBLIN_PARITY_MUTTER_PREFIX:-/opt/mutter51}"
gnoblin_prefix="${GNOBLIN_PARITY_GNOBLIN_PREFIX:-/opt/gnoblin}"
mutter_ref="${GNOBLIN_PARITY_MUTTER_REF:-51.0}"
dnf=(dnf -y --setopt=tsflags=nocaps)

cmd_deps() {
    # Mutter's own build requirements, then the tools to build it and the libraries the workspace links.
    "${dnf[@]}" install dnf-plugins-core git meson ninja-build gcc gcc-c++ pkgconf-pkg-config python3 dbus-daemon \
        mesa-dri-drivers mesa-libEGL gsettings-desktop-schemas
    "${dnf[@]}" builddep mutter
    "${dnf[@]}" install gstreamer1 clang curl
    # The libraries the workspace links, from the one list the other CI jobs install from.
    DNF_OPTIONS="--setopt=tsflags=nocaps" "$root/scripts/ci-install-system-libraries.sh"
    # `rust-toolchain.toml` names the compiler; rustup installs it on first use. A machine that already has a cargo
    # on PATH keeps it.
    if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
    fi
}

cmd_mutter() {
    if [ -x "$prefix/bin/mutter" ]; then
        echo "parity-reference: keeping the Mutter in $prefix"
        return
    fi
    local source="${GNOBLIN_PARITY_MUTTER_SOURCE:-/tmp/mutter-$mutter_ref}"
    [ -d "$source/.git" ] || git clone --depth 1 --branch "$mutter_ref" https://gitlab.gnome.org/GNOME/mutter.git "$source"
    # Headless needs the native backend and nothing of the rest: no X11, no tests, no docs, no profiler.
    meson setup "$source/_build" "$source" --prefix="$prefix" --libdir=lib64 --buildtype=release \
        -Dtests=disabled -Dinstalled_tests=false -Dprofiler=false -Ddocs=false -Dxwayland=false \
        -Dbash_completion=false -Dintrospection=false
    ninja -C "$source/_build"
    ninja -C "$source/_build" install
}

cmd_gnoblin() {
    if [ -x "$gnoblin_prefix/bin/gnoblin" ]; then
        echo "parity-reference: keeping the Gnoblin in $gnoblin_prefix"
        return
    fi
    local source="${GNOBLIN_PARITY_GNOBLIN_SOURCE:?set GNOBLIN_PARITY_GNOBLIN_SOURCE to a checkout of the legacy main branch}"
    # The legacy tree's own list of build requirements, the one its prefix build and its test jobs install from.
    "$source/scripts/install-fedora-build-deps.sh" --with-test-tools
    # The pinned Mutter, the portal backend and imgui are submodules; a checkout that has them already keeps them.
    git -C "$source" submodule update --init --depth 1 --recommend-shallow
    # The portal backend is a GTK program of its own and nothing the parity scenarios touch.
    (cd "$source" && GNOBLIN_DEVKIT=enabled ./build.sh --prefix "$gnoblin_prefix" --jobs "$(nproc)" --without-portal)
}

cmd_run() {
    local report="${1:-$root/target/parity-report.txt}"
    mkdir -p "$(dirname "$report")"
    cd "$root"
    export PATH="$HOME/.cargo/bin:$PATH"
    # `GNOBLIN_PARITY_BIN` names a gnoblin-parity binary built earlier; without it the runner is built and run by cargo.
    local -a runner=(cargo run --locked -q -p gnoblin-parity --)
    [ -z "${GNOBLIN_PARITY_BIN:-}" ] || runner=("$GNOBLIN_PARITY_BIN")
    # The build has no X11, so the launcher must not pass --no-x11.
    GNOBLIN_PARITY_MUTTER51="$root/scripts/parity-mutter51" \
        GNOBLIN_PARITY_MUTTER_PREFIX="$prefix" \
        GNOBLIN_PARITY_MUTTER_ARGS="" \
        GNOBLIN_PARITY_MUTTER_GNOBLIN="$root/scripts/parity-mutter-gnoblin" \
        GNOBLIN_PARITY_GNOBLIN_PREFIX="$gnoblin_prefix" \
        "${runner[@]}" run tests/parity/scenarios --require mutter51 --require mutter-gnoblin | tee "$report"
}

case "${1:-}" in
    deps) cmd_deps ;;
    mutter) cmd_mutter ;;
    gnoblin) cmd_gnoblin ;;
    run)
        shift
        cmd_run "$@"
        ;;
    *)
        sed -n 2,20p "${BASH_SOURCE[0]}"
        exit 2
        ;;
esac

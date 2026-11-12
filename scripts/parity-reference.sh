#!/usr/bin/env bash
# Run the parity scenarios against the Rust candidate and against stock Mutter 51, on a Fedora 45 machine.
#
# usage: scripts/parity-reference.sh deps | mutter | run [REPORT]
#
#   deps     install what the Mutter build and the workspace build need: dnf packages (root) and rustup
#   mutter   build Mutter 51.0 from the upstream tag into $GNOBLIN_PARITY_MUTTER_PREFIX (default /opt/mutter51); a
#            prefix that already holds bin/mutter is kept
#   run      run tests/parity/scenarios with GNOBLIN_PARITY_MUTTER51 set to scripts/parity-mutter51, requiring that
#            every `mutter51` row ran, and write the report to REPORT (default target/parity-report.txt)
#
# The CI job `.github/workflows/parity-reference.yml` runs the three in order in a fedora:45 container. On a host that
# cannot run containers the same steps run inside an extracted Fedora 45 root file system, with `run` calling the
# launcher through GNOBLIN_PARITY_MUTTER_ROOT (tests/parity/README.md, "Run the Mutter reference").
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${GNOBLIN_PARITY_MUTTER_PREFIX:-/opt/mutter51}"
mutter_ref="${GNOBLIN_PARITY_MUTTER_REF:-51.0}"
dnf=(dnf -y --setopt=tsflags=nocaps)

cmd_deps() {
    # Mutter's own build requirements, then the tools to build it and the libraries the workspace links.
    "${dnf[@]}" install dnf-plugins-core git meson ninja-build gcc gcc-c++ pkgconf-pkg-config python3 dbus-daemon \
        mesa-dri-drivers mesa-libEGL gsettings-desktop-schemas
    "${dnf[@]}" builddep mutter
    "${dnf[@]}" install gstreamer1 clang clang-devel pipewire-devel wayland-devel libxkbcommon-devel libinput-devel \
        systemd-devel libseat-devel mesa-libgbm-devel libdrm-devel dbus-devel pam-devel curl
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
        "${runner[@]}" run tests/parity/scenarios --require mutter51 | tee "$report"
}

case "${1:-}" in
    deps) cmd_deps ;;
    mutter) cmd_mutter ;;
    run)
        shift
        cmd_run "$@"
        ;;
    *)
        sed -n 2,14p "${BASH_SOURCE[0]}"
        exit 2
        ;;
esac

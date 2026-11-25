#!/usr/bin/env bash
# Build gnoblin and run its tests inside a container of one distribution, as the `distro` jobs of matrix.yml do.
#
#   scripts/ci/distro-test.sh IMAGE [RECIPE ...]
#
# IMAGE is a container image (for example `registry.fedoraproject.org/fedora:43`). RECIPE names the `just` recipes to
# run, in order; with none it runs `test` (every crate's tests, `cargo test --workspace --all-targets`) and `modular`
# (the shipped binary's sessions over the staged libraries). It runs the same way on a developer machine, with docker or
# podman, so a CI failure on one distribution is reproduced with one command.
#
# The checkout is mounted at /work and the build goes to a cache directory on the host (CI_DISTRO_TARGET, by default
# target/distro/<image name>), so a second run reuses it. In the container, root installs the packages with
# scripts/ci-install-system-libraries.sh (the libraries and the `toolchain` group); then an ordinary user with the
# caller's uid installs the compiler rust-toolchain.toml names, with rustup, and runs the recipes. The tests run as an
# ordinary user, as on a hosted runner: several check permissions that root would pass. Nothing is installed on the
# host.
#
# Inside the container the script runs itself again with --as-root and --as-user; those are not for direct use.
set -euo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

case "${1:-}" in
    --as-root)
        shift
        echo "[distro] $(. /etc/os-release && echo "$PRETTY_NAME")"
        DNF_OPTIONS="--setopt=install_weak_deps=False" /work/scripts/ci-install-system-libraries.sh toolchain
        # The user the build runs as has the caller's uid, so what it writes to the mounted cache stays the caller's.
        uid=${CI_DISTRO_UID:-1000}
        getent passwd "$uid" >/dev/null || useradd --uid "$uid" --create-home --shell /bin/bash ci
        user=$(getent passwd "$uid" | cut -d: -f1)
        chown "$uid" /target /cargo-home
        exec runuser -u "$user" -- env HOME="$(getent passwd "$uid" | cut -d: -f6)" \
            bash /work/scripts/ci/distro-test.sh --as-user "$@"
        ;;
    --as-user)
        shift
        cd /work
        export RUSTUP_HOME=/cargo-home/rustup PATH="$CARGO_HOME/bin:$PATH"
        if ! command -v rustup >/dev/null; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
                sh -s -- -y --no-modify-path --profile minimal --default-toolchain none
        fi
        # rust-toolchain.toml selects the compiler and its components; this installs it.
        rustup show active-toolchain
        # The tests start their own buses and sockets in a runtime directory, which must exist and be private.
        XDG_RUNTIME_DIR="/tmp/runtime-$(id -u)"
        export XDG_RUNTIME_DIR
        install -d -m 0700 "$XDG_RUNTIME_DIR"
        for recipe in "$@"; do
            echo "[distro] just $recipe"
            just "$recipe"
        done
        exit 0
        ;;
esac

image=${1:?usage: scripts/ci/distro-test.sh IMAGE [RECIPE ...]}
shift
recipes=("$@")
[ ${#recipes[@]} -gt 0 ] || recipes=(test modular)

engine=$(command -v docker || command -v podman || true)
[ -n "$engine" ] || {
    echo "[distro] neither docker nor podman is installed" >&2
    exit 1
}

root=$(cd "$(dirname "$0")/../.." && pwd)
name=$(printf '%s' "$image" | tr -c 'A-Za-z0-9._-' '_')
target=${CI_DISTRO_TARGET:-$root/target/distro/$name}
cargo_home=${CI_DISTRO_CARGO_HOME:-$root/target/distro/cargo-home}
mkdir -p "$target" "$cargo_home"

# The build user in the container has the caller's uid. Rootless podman needs --userns=keep-id for that uid to be the
# caller on the host, and --user 0 so the package install still runs as root; docker runs as root and maps uids as-is.
engine_args=()
case "$engine" in
    */podman) engine_args=(--userns=keep-id --user 0:0) ;;
esac

echo "[distro] $image: just ${recipes[*]} (target $target)"
# --security-opt label=disable: on an SELinux host the bind mounts would otherwise be unreadable inside the container.
exec "$engine" run --rm "${engine_args[@]}" \
    --security-opt label=disable \
    -v "$root:/work" \
    -v "$target:/target" \
    -v "$cargo_home:/cargo-home" \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_HOME=/cargo-home \
    -e CARGO_INCREMENTAL=0 \
    -e CARGO_TERM_COLOR=always \
    -e CARGO_PROFILE_DEV_DEBUG=0 \
    -e CI_DISTRO_UID="$(id -u)" \
    -e CI="${CI:-}" \
    "$image" bash /work/scripts/ci/distro-test.sh --as-root "${recipes[@]}"

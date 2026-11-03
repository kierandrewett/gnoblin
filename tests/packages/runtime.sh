#!/usr/bin/env bash
# Layer 3 of the package suite: start the installed compositor and talk to it.
#
# usage: tests/packages/runtime.sh PACKAGE_DIR OUT_DIR
#
# Run as root, with bash, in a clean container of the install image of the target. scripts/run-package-tests.sh starts
# it. It installs gnoblin from PACKAGE_DIR and the test tools from the repositories of the system, then runs
# runtime-session.sh as a user that is not root, in a D-Bus session. OUT_DIR receives results.tsv and the logs.
#
# A container has no seat and no DRM device, and the packaged Mutter has no development viewer. So a wrapper at
# /usr/lib/gnoblin/bin/gnoblin, the path from which the guardian starts the compositor, adds Mutter's --headless and
# --virtual-monitor. The guardian, the Lua supervisor and the compositor are otherwise the installed programs with
# their defaults. design/package-testing.md lists what this proves and what it does not (R1 to R7).
set -euo pipefail

if [ "$#" -ne 2 ] || [ ! -d "$1" ]; then
    echo "usage: $0 PACKAGE_DIR OUT_DIR" >&2
    exit 2
fi
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
package_dir="$(cd -- "$1" && pwd)"
out_dir="$2"
LAYER=runtime
# shellcheck source=tests/packages/lib.sh
. "$here/lib.sh"
mkdir -p "$out_dir"
cases_begin "$out_dir/results.tsv" "$out_dir/cases"
user=gnoblintest

# The test tools: the software renderer (Mesa), the D-Bus daemon, wayland-info, grim (screencopy), swaybg (a
# layer-shell client that fills the output with one colour) and Python for the probe.
case "$kind" in
    rpm) tools=(mesa-dri-drivers mesa-libEGL dbus-daemon wayland-utils grim swaybg python3 shadow-utils) ;;
    suse) tools=(Mesa-dri Mesa-libEGL1 dbus-1-daemon wayland-utils grim swaybg python3 shadow) ;;
    arch) tools=(mesa dbus wayland-utils grim swaybg python) ;;
    apk) tools=(mesa-dri-gallium mesa-egl dbus wayland-utils grim swaybg python3 shadow) ;;
    deb) tools=(libgl1-mesa-dri libegl-mesa0 dbus-daemon dbus-bin wayland-utils grim swaybg python3 passwd) ;;
esac

case_install() {
    pm_refresh
    local core frames
    core="$(find_package gnoblin "$package_dir")"
    frames="$(find_package gnoblin-x11-frames "$package_dir")"
    case "$kind" in
        arch | apk) pm_install_files "$core" "$frames" ;;
        *) pm_install_files "$core" ;;
    esac
    pm_install "${tools[@]}"
    for tool in dbus-run-session wayland-info grim swaybg python3; do
        command -v "$tool"
    done
    useradd --create-home --uid 1500 "$user"
    install -d -o "$user" -m 0700 /run/user/1500
    # The guardian runs as gnoblin.real and starts the compositor from the installed path, where the wrapper adds the
    # headless options. The supervisor is started from /proc/self/exe, which is gnoblin.real.
    mv "$private/bin/gnoblin" "$private/bin/gnoblin.real"
    cat >"$private/bin/gnoblin" <<'WRAPPER'
#!/bin/sh
case " $* " in
*" --gnoblin-runtime-fd"*) exec /usr/lib/gnoblin/bin/gnoblin.real "$@" --headless --virtual-monitor 1280x720 ;;
esac
exec /usr/lib/gnoblin/bin/gnoblin.real "$@"
WRAPPER
    chmod 0755 "$private/bin/gnoblin"
    chown -R "$user" "$out_dir"
}

run_case R0 "install gnoblin and the test tools" fatal case_install
if [ -z "$FATAL_FAILED" ]; then
    # The session cases append to the same result file, as the test user.
    python3 "$here/runtime_probe.py" as-user "$user" env -i PATH=/usr/local/bin:/usr/bin:/bin \
        HOME=/home/$user XDG_RUNTIME_DIR=/run/user/1500 XDG_SESSION_TYPE=wayland XDG_SESSION_ID=package-suite \
        LANG=C.UTF-8 dbus-run-session -- bash "$here/runtime-session.sh" "$out_dir" || true
    # Everything the session wrote, for the evidence.
    cp -r "/home/$user/.local/state/gnoblin" "$out_dir/state" 2>/dev/null || true
else
    for id in R1 R2 R3 R4 R5 R6 R7; do
        printf '%s\t%s\tfail\t0\tnot run: R0 failed\n' "$id" "session case" >>"$out_dir/results.tsv"
    done
fi
echo "[runtime] done"

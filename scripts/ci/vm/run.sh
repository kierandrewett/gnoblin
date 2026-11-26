#!/usr/bin/env bash
# Boot one distribution's cloud image under KVM, run the installed gnoblin on its DRM backend, and check real clients.
#
#   scripts/ci/vm/run.sh DISTRO PREFIX SHELL OUTDIR
#
# DISTRO   fedora-43, arch, debian-13 or opensuse-tumbleweed
# PREFIX   an installed gnoblin tree (`cargo xtask dist install --prefix DIR`); it is copied to /opt/gnoblin in the guest
# SHELL    the shell library gnoblin loads, copied beside it (CI uses examples' libstd_prelude.so)
# OUTDIR   where the logs, captures and summary.md go
#
# CI_VM_MODE picks the display:
#   software (default)  virtio-vga, a 2D virtual GPU: the guest's GBM and GLES are Mesa's llvmpipe and QEMU captures the
#                       scan-out, so it runs on a runner with no GPU. It tests KMS, GBM, EGL, seatd and libinput for real.
#   gpu                 virtio-vga-gl with virgl on the host's render node (CI_VM_RENDER_NODE, default
#                       /dev/dri/renderD128): GLES runs on the host GPU. QEMU cannot capture a GL scan-out, so this mode
#                       checks the logs and the clients and takes no pictures.
#
# Each check is a gnoblin session on the DRM backend with one client. A check fails when gnoblin does not serve, the
# GLES context never comes up, the kernel refuses a frame, or the client is not running after its wait. Input is sent
# as device events through QMP (scripts/ci/vm/qmp.py) to the guest's virtio keyboard, relative mouse and absolute tablet,
# and checked with wev, which prints what a client receives. A check named in EXPECTED_FAILURES below is a known bug: its failure is
# reported and allowed, and its passing fails the run so the entry is removed with the fix.
#
# Images are downloaded once into CI_VM_CACHE (default ~/.cache/gnoblin-ci-vm) and verified against the distribution's
# published checksums. The guest disk is a throwaway overlay in OUTDIR.
set -euo pipefail
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

distro=${1:?usage: run.sh DISTRO PREFIX SHELL OUTDIR}
prefix=$(cd "${2:?PREFIX}" && pwd)
shell_lib=$(cd "$(dirname "${3:?SHELL}")" && pwd)/$(basename "$3")
out=$(mkdir -p "${4:?OUTDIR}" && cd "$4" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
mode=${CI_VM_MODE:-software}
cache=${CI_VM_CACHE:-$HOME/.cache/gnoblin-ci-vm}
mkdir -p "$cache" "$out/run"

# Known bugs, as `check-name|bead`. A check listed here may fail; if it passes, the run fails until the entry goes.
EXPECTED_FAILURES=(
    "xterm-size|gnoblin-99fu"
)

case "$distro" in
    fedora-43)
        image_url=https://download.fedoraproject.org/pub/fedora/linux/releases/43/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-43-1.6.x86_64.qcow2
        sums_url=https://download.fedoraproject.org/pub/fedora/linux/releases/43/Cloud/x86_64/images/Fedora-Cloud-43-1.6-x86_64-CHECKSUM
        seat_group=seat
        install="sudo dnf -y -q --setopt=install_weak_deps=False install seatd libseat mesa-dri-drivers mesa-libgbm mesa-libEGL egl-utils libinput libxkbcommon pipewire-libs libwacom libgudev pixman dbus-daemon foot gtk4-devel-tools gtk3-devel firefox xorg-x11-server-Xwayland xterm xeyes xwininfo wev"
        ;;
    arch)
        image_url=https://geo.mirror.pkgbuild.com/images/latest/Arch-Linux-x86_64-cloudimg.qcow2
        sums_url=$image_url.SHA256
        seat_group=seat
        install="sudo pacman -Syu --noconfirm --needed seatd mesa mesa-utils libinput libxkbcommon pipewire libwacom libgudev pixman dbus foot gtk4-demos gtk3-demos firefox xorg-xwayland xterm xorg-xeyes xorg-xwininfo wev ttf-dejavu"
        ;;
    debian-13)
        # The generic image, not genericcloud: the cloud kernel has no DRM drivers, so it would have no /dev/dri.
        image_url=https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-amd64.qcow2
        sums_url=https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS
        seat_group=video
        install="sudo DEBIAN_FRONTEND=noninteractive apt-get -qq update && sudo DEBIAN_FRONTEND=noninteractive apt-get -y -qq install --no-install-recommends seatd libseat1 libgl1-mesa-dri libegl-mesa0 libgbm1 mesa-utils libinput10 libxkbcommon0 libpipewire-0.3-0t64 libwacom9 libgudev-1.0-0 libpixman-1-0 dbus-daemon foot gtk-4-examples gtk-3-examples firefox-esr xwayland xterm x11-apps x11-utils wev fonts-dejavu-core"
        ;;
    opensuse-tumbleweed)
        image_url=https://download.opensuse.org/tumbleweed/appliances/openSUSE-Tumbleweed-Minimal-VM.x86_64-Cloud.qcow2
        sums_url=$image_url.sha256
        seat_group=seat
        # seatd-systemd carries seatd.service and the sysusers entry for the seat group; --no-recommends skips it.
        # The widget factories are in gtk4-devel-tools and gtk3-devel, not in gtk4-tools and gtk3-tools.
        install="sudo zypper --non-interactive --gpg-auto-import-keys install --no-recommends seatd seatd-systemd libseat1 Mesa-dri Mesa-libEGL1 libgbm1 Mesa-demo-egl libinput10 libxkbcommon0 libpipewire-0_3-0 libwacom-data libgudev-1_0-0 libpixman-1-0 dbus-1-daemon foot gtk4-devel-tools gtk3-devel MozillaFirefox xwayland xterm xeyes xwininfo wev dejavu-fonts"
        ;;
    *)
        echo "[vm] unknown distribution '$distro' (fedora-43, arch, debian-13, opensuse-tumbleweed)" >&2
        exit 2
        ;;
esac

log() { echo "[vm:$distro] $*"; }
results=()
failures=0
# record CHECK pass|fail DETAIL: one summary row, with the expected-failure rule applied.
record() {
    local check=$1 outcome=$2 detail=$3 bead="" entry
    for entry in "${EXPECTED_FAILURES[@]}"; do
        [ "${entry%%|*}" = "$check" ] && bead=${entry#*|}
    done
    if [ -n "$bead" ] && [ "$outcome" = fail ]; then
        results+=("| $check | expected failure ($bead) | $detail |")
    elif [ -n "$bead" ]; then
        results+=("| $check | **passed, remove its expected-failure entry ($bead)** | $detail |")
        failures=$((failures + 1))
    elif [ "$outcome" = pass ]; then
        results+=("| $check | pass | $detail |")
    else
        results+=("| $check | **FAIL** | $detail |")
        failures=$((failures + 1))
    fi
    log "$check: $outcome ${bead:+(expected failure $bead) }$detail"
}

# The image, verified against the published checksum: the line that names the file holds a SHA-256 or SHA-512 digest.
# Arch, Debian and openSUSE publish their image under a name that never changes ("latest"), so the copy a cache keeps
# goes stale when the distribution replaces the image. A cached copy that is not the published one is not an error: it is
# replaced by a download. Only a download that is not the published image fails the run.
image=$cache/$(basename "$image_url")
digest=$(curl -fsSL --retry 3 "$sums_url" | grep -F "$(basename "$image_url")" | grep -oE '\b[0-9a-f]{64}([0-9a-f]{64})?\b' | head -1)
[ -n "$digest" ] || {
    log "no checksum for $(basename "$image_url") in $sums_url"
    exit 1
}
if [ ${#digest} -eq 64 ]; then sum=sha256sum; else sum=sha512sum; fi
is_published() { echo "$digest  $image" | $sum -c --quiet; }
if [ -f "$image" ] && ! is_published; then
    log "the cached $(basename "$image_url") is not the published one: the distribution replaced it"
    rm -f "$image"
fi
if [ ! -f "$image" ]; then
    log "downloading $image_url"
    curl -fsSL --retry 3 -o "$image.part" "$image_url"
    mv "$image.part" "$image"
fi
is_published || {
    rm -f "$image"
    log "checksum mismatch: $(basename "$image_url") is not the image $sums_url publishes; the download was removed"
    exit 1
}

# A throwaway disk, a key for this run, and a NoCloud seed that makes a `ci` user with it.
disk=$out/run/disk.qcow2
qemu-img create -q -f qcow2 -F qcow2 -b "$image" "$disk" 16G
rm -f "$out/run/key" "$out/run/key.pub"
ssh-keygen -q -t ed25519 -N "" -C gnoblin-ci -f "$out/run/key"
mkdir -p "$out/run/seed"
cat >"$out/run/seed/meta-data" <<EOF
instance-id: gnoblin-ci-$distro
local-hostname: gnoblin-ci
EOF
cat >"$out/run/seed/user-data" <<EOF
#cloud-config
users:
  - name: ci
    groups: [wheel, video, render, input]
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    shell: /bin/bash
    lock_passwd: true
    ssh_authorized_keys:
      - $(cat "$out/run/key.pub")
growpart: {mode: auto, devices: ["/"]}
EOF
if command -v cloud-localds >/dev/null; then
    cloud-localds "$out/run/seed.iso" "$out/run/seed/user-data" "$out/run/seed/meta-data"
elif command -v genisoimage >/dev/null; then
    genisoimage -quiet -output "$out/run/seed.iso" -volid cidata -joliet -rock "$out/run/seed/user-data" "$out/run/seed/meta-data"
else
    xorriso -as mkisofs -quiet -output "$out/run/seed.iso" -volid cidata -joliet -rock "$out/run/seed/user-data" "$out/run/seed/meta-data"
fi

# A free port for ssh, and the display the mode needs.
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
qmp=$out/run/qmp.sock
case "$mode" in
    software) display=(-device virtio-vga -display none) ;;
    gpu) display=(-device virtio-vga-gl -display "egl-headless,rendernode=${CI_VM_RENDER_NODE:-/dev/dri/renderD128}") ;;
    *)
        log "CI_VM_MODE must be software or gpu"
        exit 2
        ;;
esac
log "booting ($mode display, ssh on 127.0.0.1:$port)"
qemu-system-x86_64 -name "gnoblin-ci-$distro" -enable-kvm -machine q35 -cpu host \
    -smp "${CI_VM_CPUS:-4}" -m "${CI_VM_MEMORY:-6G}" \
    -drive "file=$disk,if=virtio,cache=unsafe,discard=unmap" \
    -drive "file=$out/run/seed.iso,media=cdrom,readonly=on" \
    -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:$port-:22" -device virtio-net-pci,netdev=n0 \
    "${display[@]}" \
    -device virtio-keyboard-pci -device virtio-mouse-pci -device virtio-tablet-pci \
    -qmp "unix:$qmp,server=on,wait=off" \
    -serial "file:$out/serial.log" &
qemu_pid=$!
trap 'kill "$qemu_pid" 2>/dev/null || true' EXIT

guest() {
    ssh -i "$out/run/key" -p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes ci@127.0.0.1 "$@"
}
for _ in $(seq 1 120); do
    guest true 2>/dev/null && break
    kill -0 "$qemu_pid" 2>/dev/null || {
        log "QEMU exited"
        exit 1
    }
    sleep 5
done
guest true || {
    log "the guest never answered ssh"
    tail -30 "$out/serial.log"
    exit 1
}
guest 'cloud-init status --wait >/dev/null 2>&1 || true; . /etc/os-release; echo "$PRETTY_NAME, kernel $(uname -r)"' | tee "$out/os.txt"

log "installing the seat daemon, Mesa and the test clients"
guest "$install" >"$out/install.log" 2>&1 || {
    log "package install failed"
    tail -20 "$out/install.log"
    exit 1
}
# Any card node will do: on openSUSE Tumbleweed the virtio_gpu device is /dev/dri/card1 and there is no card0.
guest "sudo usermod -aG $seat_group ci && sudo systemctl enable --now seatd && ls /dev/dri/card[0-9]*" >>"$out/install.log" 2>&1 ||
    {
        log "no seat or no DRM device"
        tail -20 "$out/install.log"
        exit 1
    }

log "copying the installed gnoblin to /opt/gnoblin"
tar -C "$prefix" -cf - . | guest 'sudo mkdir -p /opt/gnoblin && sudo tar -C /opt/gnoblin -xf -'
guest 'sudo mkdir -p /opt/gnoblin/share/gnoblin-ci'
guest "sudo tee /opt/gnoblin/share/gnoblin-ci/$(basename "$shell_lib") >/dev/null" <"$shell_lib"
guest 'cat > ~/guest-session.sh && chmod +x ~/guest-session.sh' <"$here/guest-session.sh"
guest 'ldd /opt/gnoblin/bin/gnoblin | grep "not found"' >"$out/ldd-missing.txt" || true
[ ! -s "$out/ldd-missing.txt" ] || {
    log "libraries the binary needs are missing:"
    cat "$out/ldd-missing.txt"
    exit 1
}
shell_path=/opt/gnoblin/share/gnoblin-ci/$(basename "$shell_lib")

# capture NAME: the display as OUTDIR/NAME.png, in software mode, where QEMU can read the scan-out. A capture is the visual
# evidence of a check, so one that fails is recorded as a failing check of its own; the run goes on with the next check.
capture() {
    [ "$mode" = software ] || return 0
    "$here/qmp.py" "$qmp" screendump "$out/$1.png" 2>"$out/$1.capture.err" && [ -s "$out/$1.png" ] && return 0
    record "$1-capture" fail "QEMU could not capture the display (see $1.capture.err)"
}

# check_client NAME PROCESS WAIT COMMAND: one session with one client, its logs and (software mode) a capture.
check_client() {
    local name=$1 process=$2 wait=$3 command=$4 state renderer
    guest "./guest-session.sh $shell_path $(printf %q "$command")" >"$out/$name.session.txt" 2>&1 || true
    sleep "$wait"
    capture "$name"
    guest 'cat ~/session/gnoblin.log' >"$out/$name.gnoblin.log" 2>&1 || true
    guest 'cat ~/session/client.log' >"$out/$name.client.log" 2>&1 || true
    state=$(guest "pgrep -x gnoblin >/dev/null && echo serving || echo stopped; pgrep -x $process >/dev/null && echo running || echo exited" 2>/dev/null | tr '\n' ' ')
    renderer=$(grep -m1 -oE 'GL Renderer: "[^"]*"' "$out/$name.gnoblin.log" | cut -d'"' -f2 || true)
    if ! grep -q 'serving' <<<"$state"; then
        record "$name" fail "gnoblin stopped (see $name.gnoblin.log)"
    elif ! grep -q 'the shell .* serves' "$out/$name.gnoblin.log"; then
        # Without its shell the session runs the compositor's fallback window management, which is not what is tested.
        record "$name" fail "the shell did not start (see $name.gnoblin.log)"
    elif ! grep -q 'the GLES context is ready' "$out/$name.gnoblin.log"; then
        record "$name" fail "no GLES context"
    elif grep -q 'frame was refused' "$out/$name.gnoblin.log"; then
        record "$name" fail "the kernel refused a frame"
    elif ! grep -q 'running' <<<"$state"; then
        record "$name" fail "$process exited (see $name.client.log)"
    else
        record "$name" pass "renderer: ${renderer:-unknown}"
    fi
}

check_client foot foot 6 'foot sh -c "while true; do date +%T; sleep 1; done"'
check_client gtk4 gtk4-widget-fac 12 'gtk4-widget-factory'
check_client gtk3 gtk3-widget-fac 12 'gtk3-widget-factory'
guest 'printf "<!doctype html><title>Gnoblin CI</title><h1>Gnoblin CI</h1><p>A local page, default Firefox settings.</p>" > ~/page.html; rm -rf ~/firefox-profile; mkdir -p ~/firefox-profile'
# firefox-esr first: Debian's /usr/bin/firefox is a script that execs firefox-esr, so the process the check looks for
# would not be named firefox.
firefox=$(guest 'command -v firefox-esr || command -v firefox' | head -1)
check_client firefox "$(basename "$firefox")" 30 "$firefox --no-remote --profile \$HOME/firefox-profile --new-instance file://\$HOME/page.html"
check_client xeyes xeyes 8 'xeyes -geometry 300x200'

# An X11 client from the Xt toolkit asks for its size after it maps; a window left at 1x1 never shows.
check_client xterm xterm 8 'xterm -geometry 90x24'
size=$(guest 'export DISPLAY=:0; w=$(xwininfo -root -tree | grep -i "\"xterm\"" | grep -oE "0x[0-9a-f]+" | head -1); [ -n "$w" ] && xwininfo -id "$w" | grep -E "Width|Height" | tr -s " " | tr "\n" " "' 2>/dev/null || true)
if grep -qE 'Width: 1 .*Height: 1 ' <<<"$size" || [ -z "$size" ]; then
    record xterm-size fail "xterm window ${size:-not found}"
else
    record xterm-size pass "xterm window $size"
fi

# Input (the tablet is added after the mouse, which is what makes QEMU send a click to it): the relative mouse and then the absolute tablet are sent to wev through QMP (scripts/ci/vm/input-check.sh), and each
# check is what wev was given.
GUEST_CMD="ssh -i $out/run/key -p $port -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes ci@127.0.0.1" \
    "$here/input-check.sh" "$qmp" "$shell_path" "$out" >"$out/input-check.txt" 2>&1 || true
capture input
while IFS='|' read -r check outcome detail; do
    record "$check" "$outcome" "$detail"
done <"$out/input-results.txt"
[ -s "$out/input-results.txt" ] || record input-check fail "the input checks produced no result (see input-check.txt)"

guest 'sudo systemctl poweroff' >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
    kill -0 "$qemu_pid" 2>/dev/null || break
    sleep 1
done
rm -f "$disk"

{
    echo "### DRM on $(cat "$out/os.txt") ($mode display)"
    echo
    echo "| Check | Result | Detail |"
    echo "| --- | --- | --- |"
    printf '%s\n' "${results[@]}"
} >"$out/summary.md"
cat "$out/summary.md"
[ -z "${GITHUB_STEP_SUMMARY:-}" ] || cat "$out/summary.md" >>"$GITHUB_STEP_SUMMARY"
[ "$failures" -eq 0 ] || {
    log "$failures check(s) failed"
    exit 1
}
log "all checks passed"

#!/usr/bin/env bash
# The guest side of the graphical VM test (tests/graphical/harness.py). It runs as root in the guest, from
# /var/tmp/gnoblin-graphical, where the harness copies this folder and the package files.
#
# usage: guest.sh install USER SOURCE   install the graphics stack and Gnoblin, and prepare the session on tty1.
#                                       SOURCE is a folder of package files, or "nightly" or "stable" for the
#                                       channel of repo.gnoblin.org (GNOBLIN_REPO_URL changes the server)
#        guest.sh start USER            start the Gnoblin session of USER on tty1
#        guest.sh status                print "running", or "ended STATUS" once the session has ended
#        guest.sh log                   print the output of the session command
#        guest.sh info                  print the guest versions as JSON
#        guest.sh unbind NAME           detach the virtio input device NAME from its driver (a broken-guest check)
#        guest.sh unbind-gpu            detach the virtio GPU from its driver (a broken-guest check)
#        guest.sh threads USER PATTERN  print the threads of USER's compositor (pgrep -f PATTERN): state, CPU time
#                                       twice 2 s apart, kernel wait channel and stack, and a user-space backtrace
#                                       (gdb, installed when missing) after a failed check that needs the session
#
# The session starts the way a display manager starts it: the Exec line of the installed wayland-sessions entry, in a
# logind session of type wayland on seat0 and tty1 (see prepare_session). No display manager is installed, because
# the test is about Gnoblin and not about GDM, and a display manager is one more package that differs from system to
# system.
set -euo pipefail

readonly HERE=/var/tmp/gnoblin-graphical
readonly SESSION_ENTRY=/usr/share/wayland-sessions/gnoblin.desktop

# shellcheck disable=SC1091
. /etc/os-release
separator=-
case "$ID" in
    fedora)
        kind=rpm
        suffix=".fc${VERSION_ID}.*.rpm"
        ;;
    opensuse-tumbleweed | opensuse)
        kind=suse
        suffix=".tumbleweed.*.rpm"
        ;;
    opensuse-leap)
        kind=suse
        suffix=".leap${VERSION_ID}.*.rpm"
        ;;
    arch | archarm)
        kind=arch
        suffix=.pkg.tar.zst
        ;;
    alpine)
        kind=apk
        suffix=.apk
        ;;
    debian | ubuntu)
        kind=deb
        separator=_
        suffix=.deb
        ;;
    *)
        echo "[guest] no package manager known for $ID" >&2
        exit 2
        ;;
esac

# The graphics stack that a desktop system has and the test needs: the Mesa drivers with llvmpipe (software GL for
# EGL on the virtio GPU), a D-Bus user session and Python 3 for the test client. On Alpine also the login and seat
# services that systemd provides elsewhere.
support_packages() {
    case "$kind" in
        rpm) echo mesa-dri-drivers mesa-libEGL python3 ;;
        suse) echo Mesa-dri Mesa-libEGL1 python3 ;;
        arch) echo mesa python ;;
        # util-linux-login: a login with PAM, so pam_elogind opens the logind session (busybox login has no PAM).
        # eudev: the udev database, in which mutter finds the GPU.
        apk) echo mesa-dri-gallium mesa-egl python3 curl elogind polkit-elogind dbus linux-pam util-linux-login eudev ;;
        deb) echo libgl1-mesa-dri libegl-mesa0 python3 dbus-user-session ;;
    esac
}

pm_refresh() {
    case "$kind" in
        rpm) dnf -y makecache >/dev/null ;;
        suse) zypper -n --gpg-auto-import-keys refresh >/dev/null ;;
        arch) pacman -Sy --noconfirm >/dev/null ;;
        apk) apk update >/dev/null ;;
        deb) DEBIAN_FRONTEND=noninteractive apt-get update -qq >/dev/null ;;
    esac
}

pm_install() {
    case "$kind" in
        rpm) dnf -y --setopt=install_weak_deps=False install "$@" ;;
        suse) zypper -n install --no-recommends "$@" ;;
        arch) pacman -S --noconfirm --needed "$@" ;;
        apk) apk add "$@" ;;
        deb) DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@" ;;
    esac
}

pm_install_files() {
    case "$kind" in
        rpm) dnf -y --setopt=install_weak_deps=False install "$@" ;;
        suse) zypper -n --no-gpg-checks install --no-recommends --allow-unsigned-rpm "$@" ;;
        arch) pacman -U --noconfirm "$@" ;;
        apk) apk add --allow-untrusted "$@" ;;
        deb) DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@" ;;
    esac
}

pm_version() {
    case "$kind" in
        rpm | suse) rpm -q --qf '%{VERSION}-%{RELEASE}\n' "$1" 2>/dev/null ;;
        arch) pacman -Q "$1" 2>/dev/null | cut -d' ' -f2 ;;
        apk) apk info -v "$1" 2>/dev/null | sed "s/^$1-//" ;;
        deb) dpkg-query -W -f='${Version}\n' "$1" 2>/dev/null ;;
    esac
}

# The files of one package in the package folder: gnoblin-0.1.13-1.fc42.x86_64.rpm, gnoblin_0.1.13-1~deb13_amd64.deb.
package_files() {
    local folder="$1" name="$2"
    find "$folder" -maxdepth 3 -type f -name "${name}${separator}[0-9]*${suffix}" | sort
}

install_gnoblin() {
    local source="$1"
    if [[ -d "$source" ]]; then
        local files=()
        mapfile -t files < <(package_files "$source" gnoblin)
        ((${#files[@]} == 1)) || {
            echo "[guest] expected one gnoblin${separator}*${suffix} in $source, found ${#files[@]}" >&2
            exit 1
        }
        echo "[guest] installing ${files[*]}"
        pm_install_files "${files[@]}"
        return
    fi
    case "$source" in
        nightly | stable) ;;
        *)
            echo "[guest] package source is not a folder, nightly or stable: $source" >&2
            exit 2
            ;;
    esac
    local channel=()
    [[ "$source" == nightly ]] && channel=(--nightly)
    curl -fsSL "${GNOBLIN_REPO_URL:-https://repo.gnoblin.org}/setup.sh" | sh -s -- --gnoblin "${channel[@]}"
    pm_refresh
    pm_install gnoblin
}

# The session runs the way a display manager runs it: the Exec line of the installed wayland-sessions entry, in a
# logind session of type wayland on seat0 and tty1, opened through the "login" PAM stack. With systemd a service
# does this (PAMName, TTYPath and XDG_SESSION_TYPE=wayland, as a kiosk unit does). OpenRC (Alpine) has no such service:
# an autologin on tty1 starts it from the login shell when the harness asks for it (the start file), so a session
# that ends does not start again.
prepare_session() {
    local user="$1" command
    [[ -r "$SESSION_ENTRY" ]] || {
        echo "[guest] $SESSION_ENTRY is missing" >&2
        exit 1
    }
    command="$(sed -n 's/^Exec=//p' "$SESSION_ENTRY" | head -n 1)"
    [[ -n "$command" ]] || {
        echo "[guest] $SESSION_ENTRY has no Exec line" >&2
        exit 1
    }
    install -d -m 0755 -o "$user" "$HERE/session"
    if command -v systemctl >/dev/null && [[ -d /run/systemd/system ]]; then
        # The console on tty1 is getty, or kmscon (kmsconvt@tty1, the default from Fedora 45). It must stop before the
        # session takes tty1: a console that stops after it hangs up tty1, and the session ends on that SIGHUP, which
        # systemd counts as a clean exit.
        cat >/etc/systemd/system/gnoblin-graphical-session.service <<EOF
[Unit]
Description=Gnoblin session of the graphical VM test
After=systemd-user-sessions.service getty@tty1.service kmsconvt@tty1.service
Conflicts=getty@tty1.service kmsconvt@tty1.service

[Service]
Type=simple
User=$user
PAMName=login
TTYPath=/dev/tty1
TTYReset=yes
TTYVHangup=yes
StandardInput=tty-fail
StandardOutput=journal
StandardError=inherit
UtmpIdentifier=tty1
UtmpMode=user
Environment=XDG_SESSION_TYPE=wayland
# The session command runs from a shell, as a display manager's session helper runs it (gdm-wayland-session): with
# SELinux, the user domain may enter through a shell but not through a file under /usr/lib (lib_t).
ExecStart=/bin/sh -c 'exec $command'
EOF
        systemctl daemon-reload
    else
        # The profile snippet records what the session sees (the logind session, the runtime folder, the seat), so a
        # session that stops at once says why in session.log.
        cat >/etc/profile.d/zz-gnoblin-graphical.sh <<EOF
if [ "\$(id -un)" = "$user" ] && [ "\$(tty)" = /dev/tty1 ] && [ -e $HERE/session/start ]; then
    rm -f $HERE/session/start
    {
        echo "[guest] XDG_SESSION_ID=\${XDG_SESSION_ID:-} XDG_SESSION_TYPE=\${XDG_SESSION_TYPE:-} XDG_SEAT=\${XDG_SEAT:-}"
        echo "[guest] XDG_RUNTIME_DIR=\${XDG_RUNTIME_DIR:-} \$(ls -ld "\${XDG_RUNTIME_DIR:-/nonexistent}" 2>&1)"
        loginctl show-session "\${XDG_SESSION_ID:-}" -p Type -p Seat -p Active -p VTNr 2>&1
    } > $HERE/session/session.log
    # The user's D-Bus session bus, where a systemd user instance would put it, so gnoblinctl over SSH finds it too.
    if [ -z "\${DBUS_SESSION_BUS_ADDRESS:-}" ] && [ -n "\${XDG_RUNTIME_DIR:-}" ]; then
        dbus-daemon --session --address="unix:path=\$XDG_RUNTIME_DIR/bus" --fork --nopidfile >> $HERE/session/session.log 2>&1
        export DBUS_SESSION_BUS_ADDRESS="unix:path=\$XDG_RUNTIME_DIR/bus"
    fi
    XDG_SESSION_TYPE=wayland $command >> $HERE/session/session.log 2>&1
    echo \$? > $HERE/session/exit-status
    exit 0
fi
EOF
        # Busybox login has no PAM, so no logind session is opened and XDG_SESSION_ID stays empty; Gnoblin then
        # refuses to start ("start it from a fresh Wayland login session"). util-linux login runs the login PAM
        # stack, and pam_elogind in it opens a session of type wayland on seat0 (XDG_SESSION_TYPE kept by -p).
        if [[ ! -f /etc/pam.d/login ]]; then
            printf '%s\n' 'auth include base-auth' 'account include base-account' 'password include base-password' \
                'session include base-session' >/etc/pam.d/login
        fi
        grep -q pam_elogind /etc/pam.d/login || echo '-session optional pam_elogind.so' >>/etc/pam.d/login
        rc-update add dbus default >/dev/null && rc-service dbus start >/dev/null
        # Mutter finds GPUs through the udev database, and elogind's seat rules tag them. The cloud image runs mdev,
        # which fills no udev database ("Failed to setup: No GPUs found"), so udev takes over as setup-devd does.
        for service in mdev hwdrivers; do
            rc-service "$service" stop >/dev/null 2>&1 || true
            rc-update del "$service" sysinit >/dev/null 2>&1 || true
        done
        for service in udev udev-trigger udev-settle; do
            rc-update add "$service" sysinit >/dev/null && rc-service "$service" start >/dev/null
        done
        rc-update add elogind default >/dev/null && rc-service elogind start >/dev/null
        local login=/bin/login
        [[ -x /usr/bin/login.util-linux ]] && login=/usr/bin/login.util-linux
        # env -i: login -p keeps the environment, and init's (HOME=/) would make the session write to /.cache and
        # /.config. With a clean one, login sets HOME, SHELL, USER and PATH for the user, and keeps only the type.
        sed -i "s|^tty1::.*|tty1::respawn:/usr/bin/env -i TERM=linux XDG_SESSION_TYPE=wayland $login -p -f $user|" \
            /etc/inittab
    fi
}

start_session() {
    local user="$1"
    rm -f "$HERE/session/exit-status" "$HERE/session/session.log"
    if systemd_guest; then
        systemctl reset-failed gnoblin-graphical-session.service 2>/dev/null || true
        systemctl start gnoblin-graphical-session.service
    else
        install -m 0644 -o "$user" /dev/null "$HERE/session/start"
        kill -HUP 1
        pkill -t tty1 || true
    fi
}

# The state of the session: "running", or "ended STATUS" with the exit status of the session command.
session_status() {
    if systemd_guest; then
        if systemctl is-active --quiet gnoblin-graphical-session.service; then
            echo running
        else
            local result code status
            result="$(systemctl show -p Result --value gnoblin-graphical-session.service)"
            code="$(systemctl show -p ExecMainCode --value gnoblin-graphical-session.service)"
            status="$(systemctl show -p ExecMainStatus --value gnoblin-graphical-session.service)"
            # Result "success" is an exit with status 0. Otherwise ExecMainCode 1 is "exited", and 2 and 3 are
            # "killed" and "dumped" by a signal.
            if [[ "$result" == success ]]; then
                echo "ended 0"
            elif [[ "$code" == 1 ]]; then
                echo "ended $status ($result)"
            elif [[ "$code" == 2 || "$code" == 3 ]]; then
                echo "ended signal $status ($result)"
            else
                echo "ended ($result)"
            fi
        fi
    elif [[ -e "$HERE/session/exit-status" ]]; then
        echo "ended $(cat "$HERE/session/exit-status")"
    else
        echo running
    fi
}

session_log() {
    if systemd_guest; then
        journalctl -b --no-pager -o short-monotonic -u gnoblin-graphical-session.service
    else
        cat "$HERE/session/session.log" 2>/dev/null || true
    fi
}

systemd_guest() {
    command -v systemctl >/dev/null && [[ -d /run/systemd/system ]]
}

info() {
    local kernel gnoblin mesa python
    kernel="$(uname -r)"
    gnoblin="$(pm_version gnoblin || true)"
    case "$kind" in
        rpm) mesa="$(pm_version mesa-dri-drivers || true)" ;;
        suse) mesa="$(pm_version Mesa-dri || true)" ;;
        arch) mesa="$(pm_version mesa || true)" ;;
        apk) mesa="$(pm_version mesa-dri-gallium || true)" ;;
        deb) mesa="$(pm_version libgl1-mesa-dri || true)" ;;
    esac
    python="$(python3 -c 'import platform; print(platform.python_version())' 2>/dev/null || true)"
    python3 - "$PRETTY_NAME" "$kernel" "$gnoblin" "$mesa" "$python" <<'PY'
import json, sys
keys = ("os", "kernel", "gnoblin", "mesa", "python")
print(json.dumps(dict(zip(keys, sys.argv[1:]))))
PY
}

# Detach the virtio input device whose name contains NAME (Tablet, Keyboard) from the virtio_input driver.
unbind_input() {
    local name="$1" device
    for device in /sys/bus/virtio/drivers/virtio_input/virtio*; do
        if grep -qs "$name" "$device"/input/input*/name; then
            basename "$device" >/sys/bus/virtio/drivers/virtio_input/unbind
            echo "[guest] unbound $(basename "$device") ($(cat "$device"/input/input*/name 2>/dev/null || echo "$name"))"
            return
        fi
    done
    echo "[guest] no virtio input device named $name" >&2
    exit 1
}

# Where each thread of the compositor is: a session that does not come up is either busy (CPU time grows between the
# two samples) or waiting (the wait channel and the kernel stack say on what).
compositor_threads() {
    local user="$1" pattern="$2" pid task sample stat
    local -a fields
    pid="$(pgrep -u "$user" -f -- "$pattern" | head -n 1 || true)"
    if [[ -z "$pid" ]]; then
        echo "no compositor process"
        return
    fi
    echo "compositor $pid: $(tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null || true)"
    for sample in first "after 2 s"; do
        echo "-- $sample: thread, name, state, user and system CPU ticks, wait channel"
        for task in "/proc/$pid/task/"*; do
            # The fields after the name: state is the 1st, utime the 12th, stime the 13th (proc(5)). A thread that
            # ended since the glob is skipped.
            stat="$(sed 's/.*) //' "$task/stat" 2>/dev/null)" || continue
            read -r -a fields <<<"$stat"
            ((${#fields[@]} >= 13)) || continue
            echo "${task##*/} $(cat "$task/comm" 2>/dev/null) ${fields[0]} ${fields[11]} ${fields[12]}" \
                "$(cat "$task/wchan" 2>/dev/null)"
        done
        if [[ "$sample" == first ]]; then
            sleep 2
        fi
    done
    for task in "/proc/$pid/task/"*; do
        echo "-- kernel stack of ${task##*/} $(cat "$task/comm" 2>/dev/null)"
        cat "$task/stack" 2>/dev/null || echo "(not readable)"
    done
    # The image has no debugger, so gdb is installed now, only for a check that failed: the passing path installs
    # nothing more. musl has no unwind tables of its own and Alpine's GLib is stripped, so on Alpine musl-dbg and
    # glib-dbg come too (not mesa-dbg, which is large and slow to install on an emulated guest): with them gdb names
    # every frame, where eu-stack stopped at the first musl frame (run 41964995167). Each package goes in on its own, so
    # one that a release lacks does not keep the others out. A failed install leaves the samples above as the evidence.
    # The attach stops every thread, and a futex or epoll wait that it interrupts is restarted or returns, which can
    # wake a thread whose wakeup was lost. So the caller asks the compositor again after this.
    if ! command -v gdb >/dev/null; then
        local tool tools=gdb
        [[ "$kind" != apk ]] || tools="gdb musl-dbg glib-dbg"
        for tool in $tools; do
            echo "-- installing $tool"
            timeout 600 bash -c "$(declare -f pm_install); kind=$kind; pm_install $tool" >/dev/null 2>&1 || true
        done
    fi
    if command -v gdb >/dev/null; then
        timeout 600 gdb -q -batch -p "$pid" -ex "set pagination off" -ex "set debuginfod enabled off" \
            -ex "info threads" -ex "thread apply all bt 40" -ex "info sharedlibrary" 2>&1 | head -n 2000 || true
        echo "-- memory map"
        cat "/proc/$pid/maps" 2>/dev/null || true
    else
        echo "-- no gdb in the guest: no user-space backtrace"
    fi
}

unbind_gpu() {
    local device
    for device in /sys/bus/virtio/drivers/virtio_gpu/virtio*; do
        [[ -e "$device" ]] || continue
        basename "$device" >/sys/bus/virtio/drivers/virtio_gpu/unbind
        echo "[guest] unbound the virtio GPU $(basename "$device")"
        return
    done
    echo "[guest] no virtio GPU is bound" >&2
    exit 1
}

case "${1:-}" in
    install)
        (($# == 3)) || exit 2
        pm_refresh
        # shellcheck disable=SC2046
        pm_install $(support_packages)
        install_gnoblin "$3"
        prepare_session "$2"
        ;;
    start)
        (($# == 2)) || exit 2
        start_session "$2"
        ;;
    status) session_status ;;
    log) session_log ;;
    info) info ;;
    unbind)
        (($# == 2)) || exit 2
        unbind_input "$2"
        ;;
    unbind-gpu) unbind_gpu ;;
    threads)
        (($# == 3)) || exit 2
        compositor_threads "$2" "$3"
        ;;
    *)
        sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
        exit 2
        ;;
esac

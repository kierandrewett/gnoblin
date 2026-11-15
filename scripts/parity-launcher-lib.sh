#!/usr/bin/env bash
# Shared by the launchers of the parity runner (parity-mutter51, parity-mutter-gnoblin): enter a root file system, and
# start the private session bus the runner reaches for input.
#
# The runner calls a launcher as `<launcher> RUNTIME_DIR DISPLAY` with XDG_RUNTIME_DIR set to RUNTIME_DIR. A launcher
# sources this file from its own directory and calls the two functions. When `GNOBLIN_PARITY_MUTTER_ROOT` is set the
# launcher re-runs itself inside that root file system, with this file and itself on standard input, because a root
# file system that is only a Fedora tree does not have the repository: `PARITY_LAUNCHER_LIB` is what makes the second
# run skip the `source` line that the first one needed.

# Re-run the calling launcher inside $GNOBLIN_PARITY_MUTTER_ROOT, when one is set, and never return in that case.
#   parity_enter_root SCRIPT RUNTIME_DIR DISPLAY
# Every GNOBLIN_PARITY_* variable except the root and the runtime base goes along; the default for the mutter
# arguments stays with the launcher that owns it.
parity_enter_root() {
    local script="$1" runtime_dir="$2" display="$3"
    [ -n "${GNOBLIN_PARITY_MUTTER_ROOT:-}" ] || return 0
    local root="${GNOBLIN_PARITY_MUTTER_ROOT%/}" name
    case "$runtime_dir" in
        "$root"/*) ;;
        *)
            echo "$(basename "$script"): $runtime_dir is not inside GNOBLIN_PARITY_MUTTER_ROOT ($root)" >&2
            exit 2
            ;;
    esac
    local -a forward=()
    for name in $(compgen -v GNOBLIN_PARITY_); do
        case "$name" in
            GNOBLIN_PARITY_MUTTER_ROOT | GNOBLIN_PARITY_RUNTIME_BASE) ;;
            *) forward+=("$name=${!name}") ;;
        esac
    done
    # A pipeline's right side runs in a subshell, so `exec` would not leave this process: wait for it and leave with its
    # status. The runner kills this process group when it is done, which ends both sides.
    local status=0
    cat "${BASH_SOURCE[0]}" "$script" |
        chroot "$root" /usr/bin/env -i \
            PATH=/usr/local/bin:/usr/bin:/bin HOME=/root PARITY_LAUNCHER_LIB=1 "${forward[@]}" \
            /bin/bash -s -- "${runtime_dir#"$root"}" "$display" || status=$?
    exit "$status"
}

# Start a private session bus at RUNTIME_DIR/bus and export it as DBUS_SESSION_BUS_ADDRESS. The runner connects to the
# same socket from outside to ask the compositor for an EIS socket, so the bus is a file in the runtime directory and
# not an address only this process tree knows. The bus allows everything: it is private to a throwaway session.
#   parity_start_bus RUNTIME_DIR
parity_start_bus() {
    local runtime_dir="$1" config="$1/bus.conf"
    cat >"$config" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:path=$runtime_dir/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
    # The launcher may be reading its own script from standard input, so the bus must not share it.
    dbus-daemon --config-file="$config" --nofork --nopidfile </dev/null >"$runtime_dir/bus.log" 2>&1 &
    local _
    for _ in $(seq 1 100); do
        [ -S "$runtime_dir/bus" ] && break
        sleep 0.05
    done
    if [ ! -S "$runtime_dir/bus" ]; then
        echo "parity launcher: the private session bus did not start; its log:" >&2
        cat "$runtime_dir/bus.log" >&2
        exit 1
    fi
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime_dir/bus"
}

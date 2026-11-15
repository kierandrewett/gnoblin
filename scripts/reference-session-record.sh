#!/usr/bin/env bash
# Record what a stock Mutter 51 headless session puts on the session bus and on its Wayland socket.
#
# usage: scripts/reference-session-record.sh OUT_DIR            run on the machine that has Mutter 51 (Fedora 45)
#        GNOBLIN_PARITY_MUTTER_ROOT=/path/to/rootfs scripts/reference-session-record.sh OUT_DIR
#                                                               the same, inside a root file system, as root
#
# It starts Mutter the way scripts/parity-mutter51 does (headless, one 1280x720 virtual monitor, llvmpipe, a private
# session bus from `dbus-run-session`), with `busctl monitor` already attached to the bus before Mutter starts, and
# records into OUT_DIR:
#
#   run.txt                 the versions and the command lines, each with its exit code
#   dbus-names.txt          every name on the session bus after start, and which connection owns it
#   dbus/mutter.xml         the full introspection of Mutter's bus connection, which owns every Mutter name, one object
#                           path after the other, as `busctl introspect --xml-interface` printed it
#   dbus-calls.txt          read-only calls to each Mutter interface: the command line, the reply and the exit code
#   dbus/sessions.xml       the introspection of the session and stream objects, while they were alive
#   dbus-sessions.txt       the calls that created, drove and stopped a session of each kind, with replies and errors
#   dbus-monitor.txt        the bus traffic from before Mutter started to after the calls, as `busctl monitor` printed it
#                           (large, and not stored in the repository)
#   dbus-monitor-mutter.txt the method calls and signals Mutter's own connection sent, cut out of dbus-monitor.txt
#   wayland-globals.txt     the globals Mutter advertises, from `wayland-info`, with their versions
#   wayland-trace.txt       the client-side `WAYLAND_DEBUG=1` trace of that `wayland-info` run
#   wayland-gtk4.txt        which globals a GTK 4 client binds (`gtk4-widget-factory`), from its own `WAYLAND_DEBUG` trace
#   wayland-gtk4-trace.txt  that whole trace, including what the client sent to `gtk_shell1`
#
# Nothing is edited after the fact: a file holds what its command printed. A step that fails is recorded with its exit
# code and the output it had, so a missing file is never the way to learn that something did not work.
set -uo pipefail

if [ "${1:-}" = inside ]; then
    shift
    mode=inside
else
    mode=outer
fi

out="${1:?usage: reference-session-record.sh OUT_DIR}"
prefix="${GNOBLIN_PARITY_MUTTER_PREFIX:-/opt/mutter51}"

if [ "$mode" = outer ]; then
    if [ -n "${GNOBLIN_PARITY_MUTTER_ROOT:-}" ]; then
        root="${GNOBLIN_PARITY_MUTTER_ROOT%/}"
        case "$out" in
            "$root"/*) ;;
            *)
                echo "reference-session-record: $out must lie inside $root" >&2
                exit 2
                ;;
        esac
        # The script is copied in: a file inside the root is the only thing `dbus-run-session` can run again.
        install -m 0755 "${BASH_SOURCE[0]}" "$root/tmp/reference-session-record.sh"
        install -m 0755 "$(dirname "${BASH_SOURCE[0]}")/reference-session-sessions.py" "$root/tmp/reference-session-sessions.py"
        exec chroot "$root" /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin HOME=/root \
            "GNOBLIN_PARITY_MUTTER_PREFIX=$prefix" /bin/bash /tmp/reference-session-record.sh inside "${out#"$root"}"
    fi
    exec "${BASH_SOURCE[0]}" inside "$out"
fi

# ---- inside: a machine that has Mutter, a bus tool and a Wayland client ---------------------------------------------
mkdir -p "$out/dbus"
: >"$out/dbus/mutter.xml"
runtime="$(mktemp -d /tmp/refsession.XXXXXX)"
chmod 0700 "$runtime"

# The bus is started here so the monitor and every call share it; `dbus-run-session` runs this script's second half.
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    export XDG_RUNTIME_DIR="$runtime"
    export XDG_CURRENT_DESKTOP=GNOME
    exec dbus-run-session -- /bin/bash "$0" inside "$out"
fi
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$runtime}"

log="$out/run.txt"
: >"$log"
note() { printf '%s\n' "$*" >>"$log"; }

# Runs a command, appends `$ command` and its exit code to run.txt, and prints the command's output to stdout.
step() {
    local code
    "$@"
    code=$?
    note "\$ $* -> exit $code"
    return $code
}

export LD_LIBRARY_PATH="$prefix/lib64:$prefix/lib64/mutter-51:$prefix/lib:$prefix/lib/mutter-51${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export XDG_DATA_DIRS="$prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export LIBGL_ALWAYS_SOFTWARE=1
display=wayland-ref

note "recorded: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
note "host: $(uname -srm)"
note "os: $(. /etc/os-release && echo "$PRETTY_NAME")"
note "mutter: $("$prefix/bin/mutter" --version 2>&1 | head -n 1) (prefix $prefix)"
note "busctl: $(busctl --version 2>&1 | head -n 1)"
note "wayland-info: $(rpm -q wayland-utils 2>&1 | head -n 1)"
note "libwayland: $(rpm -q libwayland-client 2>&1 | head -n 1)"
note "dbus: $(dbus-daemon --version 2>&1 | head -n 1)"

# The monitor goes first, so the names Mutter looks up while it starts are in the trace.
busctl --user monitor --no-pager >"$out/dbus-monitor.txt" 2>&1 &
monitor=$!
sleep 1

# PipeWire, because Mutter's ScreenCast talks to it; recorded like everything else.
pipewire >"$out/pipewire.log" 2>&1 &
pipewire_pid=$!

"$prefix/bin/mutter" --headless --wayland --wayland-display "$display" --virtual-monitor 1280x720 \
    >"$out/mutter.log" 2>&1 &
mutter=$!
note "\$ mutter --headless --wayland --wayland-display $display --virtual-monitor 1280x720 (pid $mutter)"

for _ in $(seq 1 100); do
    [ -S "$XDG_RUNTIME_DIR/$display" ] && break
    sleep 0.2
done
if [ ! -S "$XDG_RUNTIME_DIR/$display" ]; then
    note "mutter never created $XDG_RUNTIME_DIR/$display; see mutter.log"
    kill "$mutter" "$monitor" "$pipewire_pid" 2>/dev/null
    exit 1
fi
sleep 3

# ---- names -----------------------------------------------------------------------------------------------------------
step busctl --user list --no-pager >"$out/dbus-names.txt" 2>&1
mutter_names=$(awk -v pid="$mutter" '$1 !~ /^:/ && $1 != "NAME" && $2 == pid { print $1 }' "$out/dbus-names.txt" | sort)
note "names owned by pid $mutter: $(echo $mutter_names)"

# ---- introspection ---------------------------------------------------------------------------------------------------
# Every name Mutter owns is on one connection, so one walk of that connection's object tree covers all of them; the
# names and the paths they are served at are in dbus-names.txt and in the XML.
connection=$(awk -v pid="$mutter" '$1 ~ /^:/ && $2 == pid { print $1; exit }' "$out/dbus-names.txt")
note "mutter's connection: ${connection:-none}"
if [ -n "$connection" ]; then
    paths=$(busctl --user tree "$connection" --list --no-pager 2>/dev/null)
    for path in $paths; do
        {
            printf '<!-- %s %s -->\n' "$connection" "$path"
            busctl --user introspect "$connection" "$path" --xml-interface --no-pager 2>&1
        } >>"$out/dbus/mutter.xml"
    done
    note "introspected $connection: $(echo $paths | wc -w) object path(s)"
fi

# ---- read-only calls, with their replies as samples ------------------------------------------------------------------
calls="$out/dbus-calls.txt"
: >"$calls"
call() {
    local code
    {
        printf '$ busctl --user %s\n' "$*"
        busctl --user --no-pager "$@" 2>&1
        code=$?
        printf '[exit %s]\n\n' "$code"
    } >>"$calls"
}
call get-property org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig PowerSaveMode
call get-property org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig ApplyMonitorsConfigAllowed
call get-property org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig NightLightSupported
call get-property org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig PanelOrientationManaged
call call org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig GetCurrentState
call call org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig org.gnome.Mutter.DisplayConfig GetResources
call call org.gnome.Mutter.IdleMonitor /org/gnome/Mutter/IdleMonitor/Core org.gnome.Mutter.IdleMonitor GetIdletime
call get-property org.gnome.Mutter.ScreenCast /org/gnome/Mutter/ScreenCast org.gnome.Mutter.ScreenCast Version
call get-property org.gnome.Mutter.RemoteDesktop /org/gnome/Mutter/RemoteDesktop org.gnome.Mutter.RemoteDesktop SupportedDeviceTypes
call get-property org.gnome.Mutter.RemoteDesktop /org/gnome/Mutter/RemoteDesktop org.gnome.Mutter.RemoteDesktop Version
call get-property org.gnome.Mutter.InputCapture /org/gnome/Mutter/InputCapture org.gnome.Mutter.InputCapture SupportedCapabilities
call get-property org.gnome.Mutter.InputCapture /org/gnome/Mutter/InputCapture org.gnome.Mutter.InputCapture Version
call get-property org.gnome.Mutter.ServiceChannel /org/gnome/Mutter/ServiceChannel org.gnome.Mutter.ServiceChannel Version
# Sessions belong to the connection that made them, so their lifecycle needs one connection: a Python client keeps one
# open while it creates, introspects, drives and stops a session of each kind.
step python3 "$(dirname "$0")/reference-session-sessions.py" "$out"
note "calls recorded in dbus-calls.txt"

# ---- Wayland ---------------------------------------------------------------------------------------------------------
export WAYLAND_DISPLAY="$display"
step wayland-info >"$out/wayland-globals.txt" 2>&1
WAYLAND_DEBUG=1 wayland-info >/dev/null 2>"$out/wayland-trace.txt"
note "\$ WAYLAND_DEBUG=1 wayland-info -> exit $?"
# A GTK 4 client is the known user of `gtk_shell1`. Some GTK demos exit while loading an icon (the image loader runs in
# bubblewrap, which this sandbox refuses), before they map a window and send anything to the private protocol, so each
# candidate is tried in turn and every attempt is recorded. The first one whose trace shows a `gtk_shell1` request is kept.
for app in gtk4-widget-factory gtk4-node-editor gtk4-demo gtk4-icon-browser gtk4-print-editor; do
    command -v "$app" >/dev/null 2>&1 || continue
    # No accessibility bus and no dconf: a GTK client waits for them in a session that has neither, and a timeout in
    # that wait would hide the Wayland traffic this step is for.
    GDK_BACKEND=wayland WAYLAND_DEBUG=1 GTK_A11Y=none NO_AT_BRIDGE=1 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
        timeout 15 "$app" >/dev/null 2>"$out/wayland-gtk4.trace"
    code=$?
    sent=$(grep -cE ' -> gtk_(shell1|surface1)#[0-9]+\.' "$out/wayland-gtk4.trace")
    note "\$ GDK_BACKEND=wayland WAYLAND_DEBUG=1 GTK_A11Y=none NO_AT_BRIDGE=1 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo timeout 15 $app -> exit $code ($(wc -l <"$out/wayland-gtk4.trace") trace lines, $sent request(s) to gtk_shell1 or gtk_surface1)"
    last="$app"
    [ "$sent" -gt 0 ] && break
done
if [ -n "${last:-}" ]; then
    note "gtk4 client kept: $last"
    # The binds are the globals the client uses; the registry events before them are only the list it was offered.
    grep -E 'wl_registry#[0-9]+\.bind\(' "$out/wayland-gtk4.trace" | sed -E 's/^\[[0-9:.]+\] *\{[^}]*\} *-> *//' >"$out/wayland-gtk4.txt"
    # The whole trace is kept (a few KB): what GTK says to the private protocols after it binds them is the consumer evidence.
    mv "$out/wayland-gtk4.trace" "$out/wayland-gtk4-trace.txt"
fi

# ---- the stock GNOME portal, Mutter's known client ------------------------------------------------------------------
# xdg-desktop-portal and its GNOME backend are activated by the bus when a portal method is called. Whatever they ask
# Mutter for while they start is in the monitor; a method that needs a user's answer (ScreenCast.Start opens a dialog)
# is not driven here, because nothing in this session can answer it.
if [ -x /usr/libexec/xdg-desktop-portal ]; then
    call call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus UpdateActivationEnvironment 'a{ss}' 4 \
        WAYLAND_DISPLAY "$display" XDG_CURRENT_DESKTOP GNOME XDG_RUNTIME_DIR "$XDG_RUNTIME_DIR" GDK_BACKEND wayland
    call call org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.Settings ReadAll 'as' 1 org.gnome.desktop.interface
    call get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.ScreenCast AvailableSourceTypes
    call get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.ScreenCast AvailableCursorModes
    call get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.RemoteDesktop AvailableDeviceTypes
    call call org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.ScreenCast CreateSession 'a{sv}' 2 \
        handle_token s refreq session_handle_token s refsession
    sleep 4
    note "portal calls recorded in dbus-calls.txt"
fi

# ---- names Mutter looked up or called, from the monitor -------------------------------------------------------------
sleep 1
kill "$monitor" 2>/dev/null
wait "$monitor" 2>/dev/null
# `busctl monitor` prints one blank-line-separated block per message. The full stream (every Hello, AddMatch and the
# tool's own introspection) is not kept; the part that says what Mutter itself uses is: the method calls and signals its
# connection sent, cut out unchanged.
awk -v RS= -v ORS='\n\n' -v conn="$connection" \
    '$0 ~ ("Sender=" conn "[ \t]") && $0 ~ /Type=(method_call|signal)/' "$out/dbus-monitor.txt" >"$out/dbus-monitor-mutter.txt"
note "monitor stopped; $(grep -c '^[^ ]* Type=' "$out/dbus-monitor.txt") message(s) in dbus-monitor.txt"

kill "$mutter" "$pipewire_pid" 2>/dev/null
wait "$mutter" 2>/dev/null
note "mutter stopped"
rm -rf "$runtime"

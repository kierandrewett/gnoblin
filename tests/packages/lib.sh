# shellcheck shell=bash
# Shared by the layers of the package suite that run in a container of the target: install.sh and runtime.sh.
#
# It knows the package manager of each system, from /etc/os-release, and runs the cases of a layer. Each case is a
# function that runs in a subshell with errexit, and writes one line to the result file:
#   id TAB title TAB pass|fail|skip TAB seconds TAB message
# A case skips by writing its reason to "$CASE_SKIP" and exiting 77. After a case marked fatal fails, the remaining
# cases of the layer are failed with the name of that case, because they test a state that does not exist.

# The layers that source this file read stock, private and kind.
# shellcheck disable=SC1091,SC2034
. /etc/os-release
separator=-
case "$ID" in
    fedora)
        kind=rpm
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    opensuse-tumbleweed | opensuse | opensuse-leap)
        kind=suse
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    arch | archarm | cachyos)
        kind=arch
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    alpine)
        kind=apk
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    debian | ubuntu)
        kind=deb
        separator=_
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm3)
        ;;
    *)
        echo "[packages] no package manager known for $ID" >&2
        exit 2
        ;;
esac
export DEBIAN_FRONTEND=noninteractive
private=/usr/lib/gnoblin

# Package manager operations. A bare "! command" never stops a script that runs under set -e, so the negated checks
# are functions.
pm_refresh() {
    case "$kind" in
        rpm) dnf -y install diffutils findutils xz >/dev/null ;;
        suse) zypper -n --gpg-auto-import-keys refresh >/dev/null && zypper -n install diffutils findutils xz >/dev/null ;;
        arch)
            # pacman downloads as an unprivileged user inside a Landlock sandbox. A container runtime may refuse the
            # Landlock calls ("restricting filesystem access failed"), and the container is the sandbox here.
            grep -q '^DisableSandbox' /etc/pacman.conf || sed -i 's/^\[options\]$/[options]\nDisableSandbox/' /etc/pacman.conf
            pacman -Syu --noconfirm --needed diffutils findutils xz >/dev/null
            ;;
        apk) apk_fetch update >/dev/null && apk_fetch add diffutils findutils xz >/dev/null ;;
        deb) apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends diffutils findutils xz-utils >/dev/null ;;
    esac
}
# The errors with which apk-tools 2 and 3 report a fetch from a repository that failed in transit: TLS, DNS, the
# connection, an HTTP timeout or server error. A mirror or runner network can have them for a moment (run 41918451629,
# Alpine edge aarch64: "ERROR: diffutils-3.12-r0: TLS: unspecified error", while the same command passed on every other
# Alpine leg). A missing or broken package, a conflict or an untrusted certificate is not one of them. apk-tools 2
# says "IO ERROR" for a dropped connection, and "Permission denied" for an https index whose connection was closed.
APK_TRANSIENT='TLS: unspecified error|TLS: handshake failed|DNS: transient error|DNS lookup error|temporary error'
APK_TRANSIENT+='|network error|could not connect to server|Connection closed|Connection reset|Connection refused'
APK_TRANSIENT+='|Connection timed out|Operation timed out|Network unreachable|HTTP 408|HTTP 50[0234]|unexpected end of file'
APK_TRANSIENT+='|I/O error|IO ERROR|opening https://[^ ]*: Permission denied'
# apk with a bounded retry of such a fetch error: at most 3 attempts in all, each failed one with its reason on stderr.
# Any other failure fails at once, and a fetch error that persists fails after the third attempt, both with the exit
# status and the messages of apk.
apk_fetch() {
    local attempt status log reason
    log="$(mktemp)"
    for attempt in 1 2 3; do
        status=0
        apk "$@" 2>"$log" || status=$?
        cat "$log" >&2
        reason="$(grep -Eo -m1 "$APK_TRANSIENT" "$log" || true)"
        if [ "$status" -eq 0 ] || [ -z "$reason" ]; then
            break
        fi
        if [ "$attempt" -lt 3 ]; then
            echo "[packages] apk $1: attempt $attempt of 3 failed in a fetch ($reason), again in $((attempt * 10)) s" >&2
            sleep $((attempt * 10))
        else
            echo "[packages] apk $1: attempt 3 of 3 failed in a fetch ($reason), giving up" >&2
        fi
    done
    rm -f "$log"
    return "$status"
}
# Say why an apk command failed: apk-tools 2 keeps the packages that it marks as broken in its database, with an "f:"
# line (f: files without a checksum or not extracted, s: a failed script).
apk_failed() {
    local rc=$?
    echo "apk exited $rc" >&2
    awk -F: '/^P:/ { name = $2 } /^f:/ { print "apk marks " name " broken (" $2 ")" } /^$/ { name = "" }' \
        /lib/apk/db/installed >&2
    return 1
}
pm_install_files() {
    case "$kind" in
        rpm) dnf -y --setopt=install_weak_deps=False install "$@" ;;
        suse) zypper -n --no-gpg-checks install --no-recommends --allow-unsigned-rpm "$@" ;;
        arch) pacman -U --noconfirm "$@" ;;
        apk) apk_fetch add --allow-untrusted "$@" || apk_failed ;;
        deb) apt-get install -y --no-install-recommends "$@" ;;
    esac
}
pm_reinstall_files() {
    case "$kind" in
        rpm) dnf -y --setopt=install_weak_deps=False reinstall "$@" ;;
        suse) zypper -n --no-gpg-checks install --force --no-recommends --allow-unsigned-rpm "$@" ;;
        arch) pacman -U --noconfirm "$@" ;;
        apk) apk_fetch add --allow-untrusted "$@" || apk_failed ;;
        deb) apt-get install -y --no-install-recommends --reinstall "$@" ;;
    esac
}
pm_install() {
    case "$kind" in
        rpm) dnf -y install "$@" ;;
        suse) zypper -n install --no-recommends "$@" ;;
        arch) pacman -S --noconfirm --needed "$@" ;;
        apk) apk_fetch add "$@" || apk_failed ;;
        deb) apt-get install -y --no-install-recommends "$@" ;;
    esac
}
pm_remove() {
    case "$kind" in
        rpm) dnf -y remove "$@" ;;
        suse) zypper -n remove "$@" ;;
        arch) pacman -R --noconfirm "$@" ;;
        apk) apk del "$@" || apk_failed ;;
        deb) apt-get purge -y "$@" ;;
    esac
}
# What a package requires, one entry for each line.
pm_requires() {
    case "$kind" in
        rpm | suse) rpm -q --requires "$1" ;;
        arch) pacman -Qi "$1" | sed -n '/^Depends On/,/^Optional Deps/p' | sed '$d; s/^Depends On *: //' | tr -s ' ' '\n' ;;
        apk) apk info -R "$1" | sed '1d' ;;
        deb) dpkg-query -W -f='${Pre-Depends}, ${Depends}\n' "$1" | tr ',|' '\n\n' ;;
    esac
}
pm_is_installed() {
    case "$kind" in
        rpm | suse) rpm -q "$1" >/dev/null 2>&1 ;;
        arch) pacman -Q "$1" >/dev/null 2>&1 ;;
        apk) apk info -e "$1" >/dev/null 2>&1 ;;
        deb) dpkg-query -W -f='${db:Status-Status}\n' "$1" 2>/dev/null | grep -qx installed ;;
    esac
}
pm_owner() {
    case "$kind" in
        rpm | suse) rpm -qf --qf '%{NAME}\n' "$1" ;;
        arch) pacman -Qqo "$1" ;;
        apk) apk info -W "$1" | sed 's/.* is owned by //; s/-[0-9][^-]*-r[0-9]*$//' ;;
        deb) dpkg -S "$1" | sed 's/:.*//; s/:[a-z0-9]*$//' ;;
    esac
}
pm_versions() {
    case "$kind" in
        rpm | suse) rpm -q "$@" ;;
        arch) pacman -Q "$@" ;;
        apk) apk info -v "$@" 2>/dev/null | sort ;;
        deb) dpkg-query -W "$@" ;;
    esac
}
# The installed version of a package, and the version in a package file, in the same spelling.
pm_installed_version() {
    case "$kind" in
        rpm | suse) rpm -q --qf '%{EPOCHNUM}:%{VERSION}-%{RELEASE}\n' "$1" ;;
        arch) pacman -Q "$1" | cut -d' ' -f2 ;;
        apk) apk info -v "$1" 2>/dev/null | head -n1 | sed "s/^$1-//" ;;
        deb) dpkg-query -W -f='${Version}\n' "$1" ;;
    esac
}
pm_file_version() {
    case "$kind" in
        rpm | suse) rpm -qp --nosignature --qf '%{EPOCHNUM}:%{VERSION}-%{RELEASE}\n' "$1" 2>/dev/null ;;
        arch) pacman -Qp "$1" | cut -d' ' -f2 ;;
        apk) tar -xzOf "$1" .PKGINFO 2>/dev/null | sed -n 's/^pkgver = //p' ;;
        deb) dpkg-deb -f "$1" Version ;;
    esac
}
pm_file_name() {
    case "$kind" in
        rpm | suse) rpm -qp --nosignature --qf '%{NAME}\n' "$1" 2>/dev/null ;;
        arch) pacman -Qp "$1" | cut -d' ' -f1 ;;
        apk) tar -xzOf "$1" .PKGINFO 2>/dev/null | sed -n 's/^pkgname = //p' ;;
        deb) dpkg-deb -f "$1" Package ;;
    esac
}
# 0 when version $1 is older than version $2, with the comparison of the package manager.
pm_older() {
    case "$kind" in
        rpm | suse) [ "$(rpm --eval "%{lua: print(rpm.vercmp('$1', '$2'))}")" = -1 ] ;;
        arch) [ "$(vercmp "$1" "$2")" -lt 0 ] ;;
        apk) [ "$(apk version -t "$1" "$2")" = '<' ] ;;
        deb) dpkg --compare-versions "$1" lt "$2" ;;
    esac
}
# The files that the package manager reports as changed since the install.
pm_verify() {
    case "$kind" in
        rpm | suse) rpm -V "$@" ;;
        arch)
            # The image leaves out the paths of its NoExtract rule (manual pages, documentation), so pacman reports
            # them as missing. Those reports are about the image, not the package.
            local changed skip
            skip="$(sed -n 's/^NoExtract *= *//p' /etc/pacman.conf | tr ' ' '\n' | sed -n 's/\*.*//p' | sed 's|^|/|' | paste -sd'|' -)"
            changed="$(pacman -Qkk "$@" 2>&1 | grep -E 'warning|error' | grep -vE "${skip:-^$}" | grep -vE ': /usr/share/(man|doc|locale|info)( |/|$)' || true)"
            [ -z "$changed" ] || {
                echo "$changed"
                return 1
            }
            ;;
        apk)
            # apk audit marks a changed file with U and a changed extended attribute with x. A container may change the
            # attributes, so only content counts.
            local changed
            changed="$(apk audit --system 2>/dev/null | grep -E '^U .*gnoblin' || true)"
            [ -z "$changed" ] || {
                echo "$changed"
                return 1
            }
            ;;
        deb)
            local changed
            changed="$(dpkg --verify "$@" 2>&1 || true)"
            [ -z "$changed" ] || {
                echo "$changed"
                return 1
            }
            ;;
    esac
}
pm_installed_gnoblin() {
    case "$kind" in
        rpm | suse) rpm -qa --qf '%{NAME}\n' 'gnoblin*' 'xdg-desktop-portal-gnoblin*' ;;
        arch) pacman -Qq | grep -E '^(gnoblin|xdg-desktop-portal-gnoblin)' || true ;;
        apk) apk info | grep -E '^(gnoblin|xdg-desktop-portal-gnoblin)' || true ;;
        deb)
            dpkg-query -W -f='${db:Status-Status} ${Package}\n' 'gnoblin*' 'xdg-desktop-portal-gnoblin*' 2>/dev/null |
                awk '$1 == "installed" { print $2 }' || true
            ;;
    esac
}
not_installed() {
    if pm_is_installed "$1"; then
        echo "$1 is installed but must not be" >&2
        return 1
    fi
}
path_absent() {
    if [ -e "$1" ] || [ -L "$1" ]; then
        echo "$1 exists but must not" >&2
        return 1
    fi
}
# requires_none PACKAGE PATTERN: no requirement of PACKAGE matches the extended regular expression PATTERN.
requires_none() {
    local found
    found="$(pm_requires "$1" | grep -E "$2" || true)"
    if [ -n "$found" ]; then
        echo "$1 requires $(paste -sd, <<<"$found"), which it must not" >&2
        return 1
    fi
}
owner_is() {
    local owner
    owner="$(pm_owner "$1")"
    if [ "$owner" != "$2" ]; then
        echo "$1 is owned by '$owner', expected '$2'" >&2
        return 1
    fi
}
# The package file of one name in a folder that the host filled with the packages of this target.
find_package() {
    local matches
    mapfile -t matches < <(find "$2" -maxdepth 1 -type f -name "${1}${separator}[0-9]*" ! -name '*.tar.xz' | sort)
    if [ "${#matches[@]}" -ne 1 ]; then
        echo "expected one $1 package in $2, found ${#matches[@]}" >&2
        return 1
    fi
    echo "${matches[0]}"
}

# The case runner.
RESULTS=""
CASE_LOG_DIR=""
FATAL_FAILED=""
cases_begin() {
    RESULTS="$1"
    CASE_LOG_DIR="$2"
    mkdir -p "$CASE_LOG_DIR"
    : >"$RESULTS"
}
# run_case ID TITLE fatal|continue FUNCTION
run_case() {
    local id="$1" title="$2" mode="$3" function="$4" start rc message log
    log="$CASE_LOG_DIR/$id.log"
    CASE_SKIP="$CASE_LOG_DIR/$id.skip"
    export CASE_SKIP
    rm -f -- "$CASE_SKIP"
    echo "[$LAYER] $id $title"
    if [ -n "$FATAL_FAILED" ]; then
        printf '%s\t%s\tfail\t0\tnot run: %s failed\n' "$id" "$title" "$FATAL_FAILED" >>"$RESULTS"
        return 0
    fi
    start="$(date +%s%N)"
    set +e
    (
        set -eEuo pipefail
        trap 'echo "failed at line $LINENO: $BASH_COMMAND" >&2' ERR
        "$function"
    ) >"$log" 2>&1
    rc=$?
    set -e
    local seconds elapsed
    elapsed=$((($(date +%s%N) - start) / 100000000))
    seconds="$((elapsed / 10)).$((elapsed % 10))"
    if [ "$rc" = 0 ]; then
        printf '%s\t%s\tpass\t%s\t\n' "$id" "$title" "$seconds" >>"$RESULTS"
    elif [ "$rc" = 77 ] && [ -s "$CASE_SKIP" ]; then
        message="$(tr '\n\t' '  ' <"$CASE_SKIP")"
        printf '%s\t%s\tskip\t%s\t%s\n' "$id" "$title" "$seconds" "$message" >>"$RESULTS"
        echo "[$LAYER] $id skipped: $message"
    else
        message="$(tr '\r' '\n' <"$log" | grep -v '^[[:space:]]*$' | tail -n 25 | sed 's/\t/ /g; s/$/\\n/' | tr -d '\n')"
        printf '%s\t%s\tfail\t%s\t%s\n' "$id" "$title" "$seconds" "$message" >>"$RESULTS"
        echo "[$LAYER] $id FAILED (exit $rc), the end of $log:"
        tail -n 25 "$log" | sed 's/^/    /'
        if [ "$mode" = fatal ]; then
            FATAL_FAILED="$id"
        fi
    fi
}
skip_case() {
    echo "$*" >"$CASE_SKIP"
    exit 77
}

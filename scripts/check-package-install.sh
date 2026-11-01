#!/usr/bin/env bash
# Check that the Gnoblin packages install, run, coexist with stock GNOME and remove cleanly.
#
# usage: check-package-install.sh PACKAGE_DIR
#
# Run as root, with bash, in a clean container of the system to check: Fedora, openSUSE, Arch Linux, Debian, Ubuntu or
# Alpine. PACKAGE_DIR holds the packages that pkgsmith wrote for that system's format (.rpm, .deb, .pkg.tar.zst or
# .apk): gnoblin, xdg-desktop-portal-gnoblin, gnoblin-geoclue-integration and gnoblin-ananicy-rules. Other formats in the
# directory are ignored, and so are the rpm files of the other distributions: on Fedora only the files of the release
# that the container runs are used (.fc43 on Fedora 43), and on openSUSE only .tumbleweed. The script installs from the
# files, so a package that is not published yet is checked the way a user installs it.
#
# What it checks, in order:
#   1. gnoblin installs without the portal backend, stock GNOME or a second Mutter.
#   2. The public files and the private runtime are where the documentation says, and every ELF file resolves its
#      libraries with no help from LD_LIBRARY_PATH.
#   3. The portal backend, the GeoClue integration and the ananicy rules install beside it.
#   4. Stock GNOME installs beside Gnoblin, and removing Gnoblin leaves stock GNOME and every Gnoblin path as it was.
set -euo pipefail
export PATH="$PATH"

if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
    echo "usage: $0 PACKAGE_DIR" >&2
    exit 2
fi
package_dir="$(cd -- "$1" && pwd)"
trap 'echo "[install-check] failed at line $LINENO: $BASH_COMMAND" >&2' ERR

# shellcheck disable=SC1091
. /etc/os-release
separator=-
case "$ID" in
    fedora)
        kind=rpm
        suffix=".fc${VERSION_ID}.*.rpm"
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    opensuse-tumbleweed | opensuse-leap | opensuse)
        kind=suse
        suffix=".tumbleweed.*.rpm"
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    arch | archarm | cachyos)
        kind=arch
        suffix=.pkg.tar.zst
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    alpine)
        kind=apk
        suffix=.apk
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)
        ;;
    debian | ubuntu)
        kind=deb
        separator=_
        suffix=.deb
        stock=(gnome-shell mutter gnome-session gnome-settings-daemon gdm3)
        ;;
    *)
        echo "[install-check] no package manager known for $ID" >&2
        exit 2
        ;;
esac

# Package manager operations. A bare "! command" never stops a script that runs under set -e, so the negated checks
# are functions.
pm_refresh() {
    case "$kind" in
        rpm) dnf -y install diffutils findutils >/dev/null ;;
        suse) zypper -n --gpg-auto-import-keys refresh >/dev/null && zypper -n install diffutils findutils >/dev/null ;;
        arch)
            # pacman downloads as an unprivileged user inside a Landlock sandbox. A container runtime may refuse the
            # Landlock calls ("restricting filesystem access failed"), and the container is the sandbox here.
            grep -q '^DisableSandbox' /etc/pacman.conf || sed -i 's/^\[options\]$/[options]\nDisableSandbox/' /etc/pacman.conf
            pacman -Syu --noconfirm --needed diffutils findutils >/dev/null
            ;;
        apk) apk update >/dev/null && apk add diffutils findutils >/dev/null ;;
        deb) apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends diffutils findutils >/dev/null ;;
    esac
}
pm_install_files() {
    case "$kind" in
        rpm) dnf -y --setopt=install_weak_deps=False install "$@" ;;
        suse) zypper -n --no-gpg-checks install --no-recommends --allow-unsigned-rpm "$@" ;;
        arch) pacman -U --noconfirm "$@" ;;
        apk) apk add --allow-untrusted "$@" ;;
        deb) apt-get install -y --no-install-recommends "$@" ;;
    esac
}
pm_install() {
    case "$kind" in
        rpm) dnf -y install "$@" ;;
        suse) zypper -n install --no-recommends "$@" ;;
        arch) pacman -S --noconfirm --needed "$@" ;;
        apk) apk add "$@" ;;
        deb) apt-get install -y --no-install-recommends "$@" ;;
    esac
}
pm_remove() {
    case "$kind" in
        rpm) dnf -y remove "$@" ;;
        suse) zypper -n remove "$@" ;;
        arch) pacman -R --noconfirm "$@" ;;
        apk) apk del "$@" ;;
        deb) apt-get purge -y "$@" ;;
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
        apk) apk info -v "$@" ;;
        deb) dpkg-query -W "$@" ;;
    esac
}
pm_installed_gnoblin() {
    case "$kind" in
        rpm | suse) rpm -qa --qf '%{NAME}\n' 'gnoblin*' 'xdg-desktop-portal-gnoblin*' ;;
        arch) pacman -Qq | grep -E '^(gnoblin|xdg-desktop-portal-gnoblin)' || true ;;
        apk) apk info | grep -E '^(gnoblin|xdg-desktop-portal-gnoblin)' || true ;;
        deb) dpkg-query -W -f='${Package}\n' 'gnoblin*' 'xdg-desktop-portal-gnoblin*' 2>/dev/null || true ;;
    esac
}

not_installed() {
    if pm_is_installed "$1"; then
        echo "[install-check] $1 is installed but must not be" >&2
        exit 1
    fi
}
path_absent() {
    if [ -e "$1" ] || [ -L "$1" ]; then
        echo "[install-check] $1 exists but must not" >&2
        exit 1
    fi
}
owner_is() {
    local owner
    owner="$(pm_owner "$1")"
    if [ "$owner" != "$2" ]; then
        echo "[install-check] $1 is owned by '$owner', expected '$2'" >&2
        exit 1
    fi
}

# The packages of this system's format, found by name so that a second architecture in the directory does not matter.
find_package() {
    local matches
    mapfile -t matches < <(find "$package_dir" -maxdepth 1 -type f -name "${1}${separator}[0-9]*${suffix}" | sort)
    if [ "${#matches[@]}" -ne 1 ]; then
        echo "[install-check] expected one $1 package in $package_dir, found ${#matches[@]}" >&2
        exit 1
    fi
    echo "${matches[0]}"
}
echo "[install-check] refresh the package manager"
pm_refresh
core="$(find_package gnoblin)"
portal="$(find_package xdg-desktop-portal-gnoblin)"
geoclue="$(find_package gnoblin-geoclue-integration)"
ananicy="$(find_package gnoblin-ananicy-rules)"

portal_conf=/usr/share/xdg-desktop-portal/gnoblin-portals.conf
portal_file=/usr/share/xdg-desktop-portal/portals/gnoblin.portal
private=/usr/lib/gnoblin

echo "[install-check] install gnoblin without stock GNOME"
pm_install_files "$core"
pm_is_installed gnoblin || {
    echo "[install-check] gnoblin is not installed" >&2
    exit 1
}
not_installed xdg-desktop-portal-gnoblin
test -f "$portal_conf"
grep -Fxq 'default=gnoblin;*;' "$portal_conf"
owner_is "$portal_conf" gnoblin
test -x "$private/bin/gnoblin"
test -x /usr/bin/gnoblinctl
test -f /usr/share/wayland-sessions/gnoblin.desktop
grep -q "^Exec=$private/bin/gnoblin\$" /usr/share/wayland-sessions/gnoblin.desktop
test -f /usr/lib/systemd/user/gnoblin-session.target
test -f /usr/lib/systemd/user/gnoblin-idle.service
test -f /usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy
test -f "$private/share/glib-2.0/schemas/gschemas.compiled"
for package in "${stock[@]}"; do
    not_installed "$package"
done
case "$kind" in
    rpm | suse)
        # The old payload packages are replaced, and the capability that other packages ask for stays.
        rpm -q --provides gnoblin | grep -Eq '^gnoblin-session = '
        rpm -q --obsoletes gnoblin | grep -Eq '^gnoblin-mutter-devel < '
        ;;
    deb)
        dpkg-query -W -f='${Provides}\n' gnoblin | grep -q 'gnoblin-session'
        ;;
esac

echo "[install-check] run the installed programs"
/usr/bin/gnoblinctl --version >/dev/null
"$private/bin/gnoblin" --version | grep -q .

echo "[install-check] check that every private library resolves with no library path set"
unset LD_LIBRARY_PATH
# A plugin has undefined symbols that the compositor provides, so musl reports it alone as unresolved.
checked=0
while IFS= read -r -d '' binary; do
    if [[ $(head -c 4 "$binary") == $'\x7fELF' ]]; then
        checked=$((checked + 1))
        linkage="$(ldd "$binary" 2>&1 || true)"
        if [[ $linkage == *'not found'* ]]; then
            echo "[install-check] unresolved library in $binary" >&2
            echo "$linkage" >&2
            exit 1
        fi
    fi
done < <(find "$private" -type f -perm /111 ! -path '*/plugins/*' -print0)
# A check that looked at nothing proves nothing, for example when the private runtime moved to another path.
if [ "$checked" -lt 5 ]; then
    echo "[install-check] only $checked ELF files found under $private" >&2
    exit 1
fi
echo "[install-check] $checked ELF files resolve"

echo "[install-check] add the optional portal backend, the GeoClue integration and the ananicy rules"
pm_install_files "$portal" "$geoclue" "$ananicy"
pm_is_installed xdg-desktop-portal-gnoblin
pm_is_installed gnoblin-geoclue-integration
pm_is_installed gnoblin-ananicy-rules
owner_is "$portal_conf" gnoblin
owner_is "$portal_file" xdg-desktop-portal-gnoblin
grep -q 'org.freedesktop.impl.portal.FileChooser;' "$portal_file"
grep -q 'org.freedesktop.impl.portal.Inhibit;' "$portal_file"
owner_is /etc/geoclue/conf.d/50-gnoblin.conf gnoblin-geoclue-integration
owner_is /etc/ananicy.d/99-gnoblin.rules gnoblin-ananicy-rules
owner_is /etc/ananicy.d/99-gnoblin.types gnoblin-ananicy-rules
test -x "$private/libexec/xdg-desktop-portal-gnoblin"
test -f /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
test -f /usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service
if [ "$kind" != arch ]; then
    # Arch's gtk4 depends on xdg-desktop-portal-gtk itself, so only the other systems can be held to this.
    not_installed xdg-desktop-portal-gtk
fi
if [ "$kind" = rpm ] || [ "$kind" = suse ]; then
    for package in gnome-desktop3 gnome-desktop4 gtk3; do
        not_installed "$package"
    done
fi
linkage="$(ldd "$private/libexec/xdg-desktop-portal-gnoblin" 2>&1 || true)"
if [[ $linkage == *'not found'* ]]; then
    echo "[install-check] unresolved library in the portal backend" >&2
    echo "$linkage" >&2
    exit 1
fi

echo "[install-check] install stock GNOME beside Gnoblin"
pm_install "${stock[@]}"
pm_versions "${stock[@]}" >/tmp/stock-gnome-packages
{
    pm_owner /usr/bin/gnome-shell
    pm_owner /usr/bin/mutter
} >/tmp/stock-gnome-owners
test -x /usr/bin/gnome-shell
test -x /usr/bin/mutter

echo "[install-check] remove Gnoblin and check that stock GNOME is unchanged"
mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
pm_remove "${gnoblin_packages[@]}"
for name in "${gnoblin_packages[@]}"; do
    not_installed "$name"
done
for path in /usr/share/wayland-sessions/gnoblin.desktop "$portal_conf" "$portal_file" \
    /usr/lib/systemd/user/gnoblin-session.target /usr/lib/systemd/user/gnoblin-idle.service \
    /usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service /etc/geoclue/conf.d/50-gnoblin.conf \
    /etc/ananicy.d/99-gnoblin.rules /etc/ananicy.d/99-gnoblin.types /usr/bin/gnoblin /usr/bin/gnoblinctl "$private"; do
    path_absent "$path"
done
pm_versions "${stock[@]}" >/tmp/gnome-packages-after-removal
diff -u /tmp/stock-gnome-packages /tmp/gnome-packages-after-removal
{
    pm_owner /usr/bin/gnome-shell
    pm_owner /usr/bin/mutter
} >/tmp/gnome-owners-after-removal
diff -u /tmp/stock-gnome-owners /tmp/gnome-owners-after-removal
test -x /usr/bin/gnome-shell
test -x /usr/bin/mutter
echo "[install-check] passed"

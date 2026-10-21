#!/usr/bin/env bash
# Check that Gnoblin installs, coexists with stock GNOME and removes cleanly on Fedora.
#
# usage: check-fedora-install.sh [--rpm-dir DIR | [--copr OWNER/PROJECT] [--version VERSION]]
#
# Run as root in a clean Fedora container. Without arguments the script installs `gnoblin` from the enabled
# repositories. --copr enables a COPR project first, and --version installs that exact version, for the COPR check
# after a release. --rpm-dir installs the binary RPMs found in DIR, for a release build that has not been published.
#
# One script holds the gate for every Fedora route: the release build, the COPR check after publication and the
# manual COPR check in verify.yml. Before this script each route kept its own copy, and the copies drifted.
set -euo pipefail
export PATH="$PATH"

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
rpm_dir=""
version=""
copr=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --rpm-dir)
            rpm_dir="$(realpath "${2:?--rpm-dir needs a directory}")"
            shift 2
            ;;
        --version)
            version="${2:?--version needs a version}"
            [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
                echo "Not a release version: $version" >&2
                exit 2
            }
            shift 2
            ;;
        --copr)
            copr="${2:?--copr needs OWNER/PROJECT}"
            shift 2
            ;;
        *)
            echo "usage: $0 [--rpm-dir DIR | [--copr OWNER/PROJECT] [--version VERSION]]" >&2
            exit 2
            ;;
    esac
done
if [ -n "$rpm_dir" ] && { [ -n "$version" ] || [ -n "$copr" ]; }; then
    echo "Use --rpm-dir alone, or --copr with --version." >&2
    exit 2
fi
# A bare "! command" never stops a script that runs under set -e, so these helpers do the negated checks.
trap 'echo "[fedora-install] failed at line $LINENO: $BASH_COMMAND" >&2' ERR
not_installed() {
    if rpm -q "$1" >/dev/null 2>&1; then
        echo "[fedora-install] $1 is installed but must not be" >&2
        exit 1
    fi
}
path_absent() {
    if [ -e "$1" ]; then
        echo "[fedora-install] $1 exists but must not" >&2
        exit 1
    fi
}

portal_conf=/usr/share/xdg-desktop-portal/gnoblin-portals.conf
portal_file=/usr/share/xdg-desktop-portal/portals/gnoblin.portal
stock_packages=(gnome-shell mutter gnome-session gnome-settings-daemon gdm)

echo "[fedora-install] install Gnoblin without stock GNOME"
if [ -n "$rpm_dir" ]; then
    mapfile -t core_rpms < <(find "$rpm_dir" -maxdepth 1 -type f -name 'gnoblin-[0-9]*.rpm' ! -name '*.src.rpm')
    mapfile -t portal_rpms < <(find "$rpm_dir" -maxdepth 1 -type f -name 'xdg-desktop-portal-gnoblin-[0-9]*.rpm' ! -name '*.src.rpm')
    test "${#core_rpms[@]}" -eq 1
    test "${#portal_rpms[@]}" -eq 1
    python3 "$root/scripts/check-rpm-isolation.py" "${core_rpms[0]}" "${portal_rpms[0]}"
    dnf -y --setopt=install_weak_deps=False install "${core_rpms[0]}"
else
    dnf -y install dnf-plugins-core
    if [ -n "$copr" ]; then
        dnf -y copr enable "$copr"
    fi
    dnf -y --setopt=install_weak_deps=False install --refresh "gnoblin${version:+-$version}"
fi
rpm -q gnoblin
if [ -n "$version" ]; then
    test "$(rpm -q --qf '%{VERSION}' gnoblin)" = "$version"
fi
not_installed xdg-desktop-portal-gnoblin
test -f "$portal_conf"
grep -Fxq 'default=gnoblin;*;' "$portal_conf"
test "$(rpm -qf --qf '%{NAME}' "$portal_conf")" = gnoblin
# rpm --whatprovides takes a capability name, not a versioned expression, so check the version separately.
test "$(rpm -q --whatprovides gnoblin-session --qf '%{NAME}')" = gnoblin
rpm -q --provides gnoblin | grep -Fxq "gnoblin-session = $(rpm -q --qf '%{VERSION}' gnoblin)"
test -x /usr/lib/gnoblin/bin/gnoblin
test -x /usr/bin/gnoblinctl
test -f /usr/share/wayland-sessions/gnoblin.desktop
grep -q '^Exec=/usr/lib/gnoblin/bin/gnoblin$' /usr/share/wayland-sessions/gnoblin.desktop
test -f /usr/lib/systemd/user/gnoblin-session.target
test -f /usr/lib/systemd/user/gnoblin-idle.service
for package in "${stock_packages[@]}"; do
    not_installed "$package"
done

echo "[fedora-install] add the optional portal backend"
if [ -n "$rpm_dir" ]; then
    dnf -y --setopt=install_weak_deps=False install "${portal_rpms[0]}"
else
    dnf -y install xdg-desktop-portal-gnoblin
fi
rpm -q xdg-desktop-portal-gnoblin
test "$(rpm -qf --qf '%{NAME}' "$portal_conf")" = gnoblin
test -f "$portal_file"
grep -q 'org.freedesktop.impl.portal.FileChooser;' "$portal_file"
grep -q 'org.freedesktop.impl.portal.Inhibit;' "$portal_file"
not_installed xdg-desktop-portal-gtk
for package in gnome-desktop3 gnome-desktop4 gtk3; do
    not_installed "$package"
done

echo "[fedora-install] check that every private library resolves"
# The stock Fedora container image has no diff, and the final comparisons need it.
dnf -y install diffutils >/dev/null
mapfile -t mutter_libdirs < <(find /usr/lib/gnoblin -maxdepth 2 -type d -name 'mutter-*')
library_path="/usr/lib/gnoblin/lib64"
for dir in "${mutter_libdirs[@]}"; do
    library_path="$library_path:$dir"
done
checked=0
while IFS= read -r -d '' binary; do
    if [[ $(head -c 4 "$binary") == $'\x7fELF' ]]; then
        checked=$((checked + 1))
        linkage="$(LD_LIBRARY_PATH="$library_path" ldd "$binary" 2>&1 || true)"
        if [[ $linkage == *'not found'* ]]; then
            echo "Unresolved library in $binary" >&2
            echo "$linkage" >&2
            exit 1
        fi
    fi
done < <(find /usr/lib/gnoblin -type f -perm /111 -print0)
# A check that looked at nothing proves nothing, for example when the private runtime moved to another path.
if [ "$checked" -lt 5 ]; then
    echo "[fedora-install] only $checked ELF files found under /usr/lib/gnoblin" >&2
    exit 1
fi
echo "[fedora-install] $checked ELF files resolve"

echo "[fedora-install] install stock GNOME beside Gnoblin"
dnf -y install "${stock_packages[@]}"
rpm -q "${stock_packages[@]}" >/tmp/stock-gnome-packages
rpm -qf /usr/bin/gnome-shell /usr/bin/mutter >/tmp/stock-gnome-owners
test -x /usr/bin/gnome-shell
test -x /usr/bin/mutter

echo "[fedora-install] remove Gnoblin and check that stock GNOME is unchanged"
dnf -y remove gnoblin
mapfile -t gnoblin_packages < <(rpm -qa 'gnoblin*' 'xdg-desktop-portal-gnoblin*')
if [ "${#gnoblin_packages[@]}" -gt 0 ]; then
    dnf -y remove "${gnoblin_packages[@]}"
fi
path_absent /usr/share/wayland-sessions/gnoblin.desktop
path_absent "$portal_conf"
path_absent /usr/lib/systemd/user/gnoblin-session.target
path_absent /usr/lib/systemd/user/gnoblin-idle.service
rpm -q "${stock_packages[@]}" >/tmp/gnome-packages-after-removal
diff -u /tmp/stock-gnome-packages /tmp/gnome-packages-after-removal
rpm -qf /usr/bin/gnome-shell /usr/bin/mutter >/tmp/gnome-owners-after-removal
diff -u /tmp/stock-gnome-owners /tmp/gnome-owners-after-removal
test -x /usr/bin/gnome-shell
test -x /usr/bin/mutter
echo "[fedora-install] passed"

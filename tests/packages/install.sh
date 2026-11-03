#!/usr/bin/env bash
# Layer 2 of the package suite: install, check, upgrade and remove the Gnoblin packages in a clean system.
#
# usage: tests/packages/install.sh PACKAGE_DIR OUT_DIR [PUBLISHED_DIR]
#
# Run as root, with bash, in a clean container of the install image of the target. scripts/run-package-tests.sh starts
# it. PACKAGE_DIR holds the packages of the target and nothing else (the host selects them): gnoblin,
# gnoblin-x11-frames, xdg-desktop-portal-gnoblin, gnoblin-geoclue-integration and gnoblin-ananicy-rules in the format of
# the system, and the tar packages for a glibc target of the shared set. PUBLISHED_DIR holds the packages of the
# version that https://repo.gnoblin.org publishes for the target, one folder for each channel, or a file SKIP that says
# why there are none. OUT_DIR receives results.tsv and one log for each case.
#
# The cases and the failure modes they detect (design/package-testing.md): I1 to I15. The script replaces
# scripts/check-package-install.sh and keeps its checks.
#
# The GTK checks look at what the Gnoblin packages require, not at what is installed: a distribution's own GTK 4 may pull
# GTK 3 (on Fedora 42 gtk4 pulls SDL3, which pulls libdecor, which requires gtk3), and that is not a Gnoblin dependency.
#
# Every package manager command must succeed. apk-tools 2 (Alpine 3.22 and older) marks a package as broken, and exits 1 in
# that and every later command, when a file of the package has no embedded checksum. The check does not accept that.
set -euo pipefail

if [ "$#" -lt 2 ] || [ ! -d "$1" ]; then
    echo "usage: $0 PACKAGE_DIR OUT_DIR [PUBLISHED_DIR]" >&2
    exit 2
fi
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
package_dir="$(cd -- "$1" && pwd)"
out_dir="$2"
published_dir="${3:-}"
LAYER=install
# shellcheck source=tests/packages/lib.sh
. "$here/lib.sh"
cases_begin "$out_dir/results.tsv" "$out_dir/cases"

portal_conf=/usr/share/xdg-desktop-portal/gnoblin-portals.conf
portal_file=/usr/share/xdg-desktop-portal/portals/gnoblin.portal
session_entry=/usr/share/wayland-sessions/gnoblin.desktop
case "$kind" in
    rpm | arch) gtk_packages=(gtk4 libadwaita) ;;
    suse | deb) gtk_packages=(libgtk-4-1 libadwaita-1-0) ;;
    apk) gtk_packages=(gtk4.0 libadwaita) ;;
esac
glibc=true
[ "$kind" != apk ] || glibc=false

case_refresh() {
    pm_refresh
    for name in gnoblin gnoblin-x11-frames xdg-desktop-portal-gnoblin gnoblin-geoclue-integration gnoblin-ananicy-rules; do
        find_package "$name" "$package_dir" >/dev/null
    done
}
core() { find_package gnoblin "$package_dir"; }
frames() { find_package gnoblin-x11-frames "$package_dir"; }
optional_packages() {
    find_package xdg-desktop-portal-gnoblin "$package_dir"
    find_package gnoblin-geoclue-integration "$package_dir"
    find_package gnoblin-ananicy-rules "$package_dir"
}

# I1: gnoblin installs on its own: no portal backend, no stock GNOME, no second Mutter. rpm and deb recommend the
# frames package, and the install refuses recommended packages. While gnoblin depends on the frames package on Arch and
# apk (FRAMES_HARD_DEPENDENCY_ON_NO_WEAK_DEP_FORMATS in pkgsmith/runtime.star), the two files go in together, because the
# package is not in any repository.
case_install_core() {
    case "$kind" in
        arch | apk) pm_install_files "$(core)" "$(frames)" ;;
        *) pm_install_files "$(core)" ;;
    esac
    pm_is_installed gnoblin
    not_installed xdg-desktop-portal-gnoblin
    for package in "${stock[@]}"; do
        not_installed "$package"
    done
}

# I2: gnoblin asks for no GTK and no libadwaita in any format; the frames client is optional on rpm and deb and a
# dependency on Arch and apk.
case_gtk() {
    requires_none gnoblin 'gtk|adwaita'
    case "$kind" in
        rpm | suse | deb)
            not_installed gnoblin-x11-frames
            test ! -e "$private/libexec/mutter-x11-frames"
            for package in "${gtk_packages[@]}"; do
                not_installed "$package"
            done
            ;;
        *) pm_is_installed gnoblin-x11-frames ;;
    esac
    case "$kind" in
        rpm | suse)
            # The old payload packages are replaced, and the capability that other packages ask for stays.
            rpm -q --provides gnoblin | grep -Eq '^gnoblin-session = '
            rpm -q --obsoletes gnoblin | grep -Eq '^gnoblin-mutter-devel < '
            ;;
        deb) dpkg-query -W -f='${Provides}\n' gnoblin | grep -q 'gnoblin-session' ;;
    esac
}

# I3: the public files and the private runtime are where the documentation says.
case_files() {
    test -f "$portal_conf"
    grep -Fxq 'default=gnoblin;*;' "$portal_conf"
    owner_is "$portal_conf" gnoblin
    test -x "$private/bin/gnoblin"
    test -x /usr/bin/gnoblin
    test -x /usr/bin/gnoblinctl
    test -f /usr/lib/systemd/user/gnoblin-session.target
    test -f /usr/lib/systemd/user/gnoblin-idle.service
    test -f /usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy
    owner_is /usr/bin/gnoblinctl gnoblin
}

# I6: the programs start.
case_programs() {
    /usr/bin/gnoblinctl --version
    /usr/bin/gnoblinctl --help >/dev/null
    "$private/bin/gnoblin" --version | grep -q .
    /usr/bin/gnoblin --version --json | grep -q '"version"'
    "$private/bin/gdctl" --help >/dev/null
}

# I7: the default configuration is in the binary, and gnoblinctl writes the editable tree from it.
case_config() {
    local home
    home="$(mktemp -d)"
    /usr/bin/gnoblinctl config default >"$home/default.lua"
    test -s "$home/default.lua"
    HOME="$home" XDG_CONFIG_HOME="$home/config" /usr/bin/gnoblinctl init
    test -s "$home/config/gnoblin/init.lua"
    rm -rf -- "$home"
}

# I8: the schemas are compiled in the private prefix and GSettings reads the Mutter schemas from there.
case_schemas() {
    local schemas="$private/share/glib-2.0/schemas"
    test -s "$schemas/gschemas.compiled"
    if command -v gsettings >/dev/null; then
        GSETTINGS_SCHEMA_DIR="$schemas" GSETTINGS_BACKEND=memory gsettings list-schemas | grep -qx org.gnome.mutter
    else
        echo "gsettings is not installed; checked the compiled file only"
    fi
}

# I9: a display manager lists the session: the entry is in wayland-sessions and starts an executable file.
case_session_entry() {
    test -f "$session_entry"
    grep -q '^Name=' "$session_entry"
    grep -q '^Type=Application' "$session_entry"
    local program
    program="$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$session_entry")"
    test "$program" = "$private/bin/gnoblin"
    test -x "$program"
    program="$(sed -n 's/^TryExec=\([^ ]*\).*/\1/p' "$session_entry")"
    [ -z "$program" ] || test -x "$program"
}

# I10: the frames client, the portal backend, the GeoClue integration and the ananicy rules install beside gnoblin, and
# no Gnoblin package asks for GTK 3 or gnome-desktop.
case_optional() {
    case "$kind" in
        rpm | suse | deb) pm_install_files "$(frames)" ;;
    esac
    pm_is_installed gnoblin-x11-frames
    owner_is "$private/libexec/mutter-x11-frames" gnoblin-x11-frames
    test -x "$private/libexec/mutter-x11-frames"
    pm_is_installed "${gtk_packages[0]}"
    mapfile -t optional < <(optional_packages)
    pm_install_files "${optional[@]}"
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
    mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
    for package in "${gnoblin_packages[@]}"; do
        requires_none "$package" 'gtk3|gtk\+3|gtk-3|gnome-desktop'
    done
}

# I4: every ELF file of every Gnoblin package resolves with no LD_LIBRARY_PATH: libraries, symbol versions and (on
# glibc, with ldd -r) symbols. A Mutter plugin takes its symbols from the compositor that loads it and does not name
# libmutter, so ldd reports them as undefined: the plugins are checked for their libraries only.
case_resolve() {
    unset LD_LIBRARY_PATH
    local checked=0 binary linkage failed=0
    while IFS= read -r -d '' binary; do
        [[ $(head -c 4 "$binary") == $'\x7fELF' ]] || continue
        if "$glibc" && [[ $binary != */plugins/* ]]; then
            linkage="$(ldd -r "$binary" 2>&1 || true)"
        elif "$glibc"; then
            linkage="$(ldd "$binary" 2>&1 || true)"
        else
            # musl relocates while it lists, so a plugin reports the symbols of the compositor as missing.
            [[ $binary == */plugins/* ]] && continue
            linkage="$(ldd "$binary" 2>&1 || true)"
        fi
        checked=$((checked + 1))
        if grep -Eq 'not found|undefined symbol|Error relocating|Error loading' <<<"$linkage"; then
            echo "$binary does not resolve:"
            grep -E 'not found|undefined symbol|Error relocating|Error loading' <<<"$linkage" | head -n 20
            failed=1
        fi
    done < <(find "$private" -type f -print0)
    echo "$checked ELF files checked"
    # A check that looked at nothing proves nothing, for example when the private runtime moved to another path.
    [ "$checked" -ge 5 ] || {
        echo "only $checked ELF files found under $private"
        return 1
    }
    [ "$failed" = 0 ]
}

# I5: every library that an ELF file needs comes from a package that the Gnoblin package asks for, and every library
# package that it asks for is needed.
case_abi() {
    python3 "$here/abi_owner.py" --kind "$kind" gnoblin gnoblin-x11-frames xdg-desktop-portal-gnoblin
}

# I11: stock GNOME installs beside Gnoblin, and none of its files belongs to a Gnoblin package.
case_stock() {
    pm_install "${stock[@]}"
    pm_versions "${stock[@]}" >/tmp/stock-gnome-packages
    {
        pm_owner /usr/bin/gnome-shell
        pm_owner /usr/bin/mutter
    } >/tmp/stock-gnome-owners
    test -x /usr/bin/gnome-shell
    test -x /usr/bin/mutter
    ! grep -E '^(gnoblin|xdg-desktop-portal-gnoblin)' /tmp/stock-gnome-owners
}

# I12: the same files install again, and the package manager reports no changed file afterwards. apk has no reinstall
# of a local file, so there the same files are removed and added again.
case_reinstall() {
    local files=("$(core)" "$(frames)")
    mapfile -t -O 2 files < <(optional_packages)
    if [ "$kind" = apk ]; then
        mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
        apk del "${gnoblin_packages[@]}" || apk_failed
    fi
    pm_reinstall_files "${files[@]}"
    mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
    pm_verify "${gnoblin_packages[@]}"
}

remove_all_gnoblin() {
    mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
    pm_remove "${gnoblin_packages[@]}"
    for name in "${gnoblin_packages[@]}"; do
        not_installed "$name"
    done
}
gnoblin_paths=(
    /usr/share/wayland-sessions/gnoblin.desktop "$portal_conf" "$portal_file"
    /usr/lib/systemd/user/gnoblin-session.target /usr/lib/systemd/user/gnoblin-idle.service
    /usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service /etc/geoclue/conf.d/50-gnoblin.conf
    /etc/ananicy.d/99-gnoblin.rules /etc/ananicy.d/99-gnoblin.types /usr/bin/gnoblin /usr/bin/gnoblinctl
    /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
    /usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy "$private"
)

# I14: the removal leaves no Gnoblin file, and stock GNOME as it was.
case_remove() {
    remove_all_gnoblin
    for path in "${gnoblin_paths[@]}"; do
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
}

# I13: the version that repo.gnoblin.org publishes for the target installs, and the new files upgrade it.
case_upgrade() {
    if [ -z "$published_dir" ] || [ ! -d "$published_dir" ]; then
        skip_case "no published packages were given"
    fi
    if [ -f "$published_dir/SKIP" ]; then
        skip_case "$(cat "$published_dir/SKIP")"
    fi
    local new_version channel chosen="" old_files=()
    new_version="$(pm_file_version "$(core)")"
    for channel in stable nightly; do
        local old
        old="$(find "$published_dir/$channel" -maxdepth 1 -type f -name "gnoblin${separator}[0-9]*" 2>/dev/null | head -n1)"
        [ -n "$old" ] || continue
        if pm_older "$(pm_file_version "$old")" "$new_version"; then
            chosen="$channel"
            break
        fi
        echo "the $channel version $(pm_file_version "$old") is not older than $new_version"
    done
    [ -n "$chosen" ] || skip_case "no published version is older than $new_version (stable and nightly checked)"
    mapfile -t old_files < <(find "$published_dir/$chosen" -maxdepth 1 -type f | sort)
    echo "upgrade from $chosen: $(basename -a "${old_files[@]}" | paste -sd' ')"
    pm_install_files "${old_files[@]}"
    local new_files=("$(core)" "$(frames)")
    mapfile -t -O 2 new_files < <(optional_packages)
    pm_install_files "${new_files[@]}"
    local file name
    for file in "${new_files[@]}"; do
        name="$(pm_file_name "$file")"
        test "$(pm_installed_version "$name")" = "$(pm_file_version "$file")" || {
            echo "$name is $(pm_installed_version "$name") after the upgrade, expected $(pm_file_version "$file")"
            return 1
        }
    done
    /usr/bin/gnoblinctl --version
    mapfile -t gnoblin_packages < <(pm_installed_gnoblin)
    pm_verify "${gnoblin_packages[@]}"
    remove_all_gnoblin
    for path in "${gnoblin_paths[@]}"; do
        path_absent "$path"
    done
}

# I15: a tar package extracts, resolves and starts, and its removal by the list of its files leaves nothing.
case_tar() {
    local tar_packages=("$package_dir"/*.tar.xz)
    [ -e "${tar_packages[0]}" ] || tar_packages=()
    if [ "${#tar_packages[@]}" -eq 0 ]; then
        skip_case "this target installs no tar packages: only the glibc targets of the shared packages do"
    fi
    local package list
    list="$(mktemp)"
    for package in "${tar_packages[@]}"; do
        tar -tJf "$package" >>"$list"
        tar -C / -xJf "$package"
    done
    "$private/bin/gnoblin" --version | grep -q .
    /usr/bin/gnoblinctl --version
    local binary linkage failed=0 checked=0
    while IFS= read -r -d '' binary; do
        [[ $(head -c 4 "$binary") == $'\x7fELF' ]] || continue
        checked=$((checked + 1))
        if [[ $binary == */plugins/* ]]; then
            linkage="$(ldd "$binary" 2>&1 || true)"
        else
            linkage="$(ldd -r "$binary" 2>&1 || true)"
        fi
        if grep -Eq 'not found|undefined symbol' <<<"$linkage"; then
            echo "$binary does not resolve:"
            grep -E 'not found|undefined symbol' <<<"$linkage" | head -n 20
            failed=1
        fi
    done < <(find "$private" -type f -print0)
    if [ "$checked" -lt 5 ] || [ "$failed" != 0 ]; then
        echo "$checked ELF files checked, unresolved: $failed"
        return 1
    fi
    # Remove the files, then the folders that are empty, deepest first, as the tar listing names them.
    grep -v '/$' "$list" | sed 's|^\./||; s|^|/|' | while IFS= read -r path; do rm -f -- "$path"; done
    grep '/$' "$list" | sed 's|^\./||; s|^|/|' | sort -r | while IFS= read -r path; do rmdir -- "$path" 2>/dev/null || true; done
    for path in "${gnoblin_paths[@]}"; do
        path_absent "$path"
    done
}

run_case I0 "refresh the package manager, find the packages" fatal case_refresh
run_case I1 "gnoblin installs alone" fatal case_install_core
run_case I2 "no GTK in gnoblin, frames client as documented" continue case_gtk
run_case I3 "public files and private runtime in place" continue case_files
run_case I6 "programs start" continue case_programs
run_case I7 "default configuration and gnoblinctl init" continue case_config
run_case I8 "GSettings schemas compiled and readable" continue case_schemas
run_case I9 "session entry for display managers" continue case_session_entry
run_case I10 "optional packages install, no GTK 3" fatal case_optional
run_case I4 "every ELF file resolves" continue case_resolve
run_case I5 "libraries come from declared dependencies" continue case_abi
run_case I11 "stock GNOME installs beside Gnoblin" fatal case_stock
run_case I12 "reinstall" continue case_reinstall
run_case I14 "removal leaves stock GNOME and no Gnoblin file" fatal case_remove
run_case I13 "upgrade from the published version" continue case_upgrade
run_case I15 "tar packages" continue case_tar
echo "[install] done"

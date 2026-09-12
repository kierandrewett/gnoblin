#!/usr/bin/env bash
# Exercise RPM payload installation/removal against disposable GNOME fixtures.
# Scriptlets, dependency resolution and login need a real test host.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ "$#" -lt 3 ]; then
    echo "Usage: $0 <gnoblin-mutter.rpm> <gnoblin-shell.rpm> <gnoblin-session.rpm> [gnoblin-mutter-devel.rpm]" >&2
    exit 2
fi
python3 "$ROOT/scripts/check-rpm-isolation.py" "$@"
work="$(mktemp -d /tmp/gnoblin-coexistence.XXXXXX)"
trap 'rm -rf "$work"' EXIT
packages=()
for package in "$@"; do
    packages+=("$(realpath "$package")")
done
fakeroot_command="$(command -v fakeroot-sysv || command -v fakeroot)"
TEST_WORK="$work" "$fakeroot_command" -- bash -euo pipefail -c '
    stock=(/usr/bin/gnome-shell /usr/lib64/libmutter-17.so.0
           /usr/lib/systemd/user/org.gnome.Shell@wayland.service
           /usr/share/glib-2.0/schemas/org.gnome.shell.gschema.xml)
    for file in "${stock[@]}"; do
        install -Dm644 "$file" "$TEST_WORK/root$file"
    done
    cd "$TEST_WORK/root"
    sha256sum "${stock[@]/#/\.}" > "$TEST_WORK/stock.sha256"
    rpm --dbpath "$TEST_WORK/db" --initdb
    rpm --dbpath "$TEST_WORK/db" --nodeps --noscripts --notriggers \
        --badreloc --relocate "/usr=$TEST_WORK/root/usr" -i "$@"
    sha256sum -c "$TEST_WORK/stock.sha256"
    test -x ./usr/lib/gnoblin/bin/gnome-shell
    test -L ./usr/bin/gnoblinctl
    test -f ./usr/share/wayland-sessions/gnoblin.desktop
    grep -q "Exec=/usr/lib/gnoblin/bin/gnoblin-session" ./usr/share/wayland-sessions/gnoblin.desktop
    grep -q "ExecStart=/usr/lib/gnoblin/bin/gnoblin-shell-service" ./usr/lib/systemd/user/org.gnoblin.Shell@wayland.service
    glib-compile-schemas --strict ./usr/lib/gnoblin/share/glib-2.0/schemas
    mapfile -t installed < <(rpm --dbpath "$TEST_WORK/db" -qa --qf "%{NAME}\n")
    rpm --dbpath "$TEST_WORK/db" --nodeps --noscripts --notriggers -e "${installed[@]}"
    sha256sum -c "$TEST_WORK/stock.sha256"
    test ! -e ./usr/share/wayland-sessions/gnoblin.desktop
    test ! -e ./usr/lib/gnoblin/bin/gnome-shell
    echo "PASS: RPM payload install/remove preserved GNOME fixtures"
' test "${packages[@]}"

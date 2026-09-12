#!/usr/bin/env bash
# Install only side-by-side Gnoblin RPMs; never replace or remove GNOME packages.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${GNOBLIN_RPM_DIR:-$HOME/rpmbuild/RPMS}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
DRY_RUN=0
VERB=install
DNF_OPTIONS=()
for arg in "$@"; do
    case "$arg" in
        --yes | -y) DNF_OPTIONS+=(-y) ;;
        --dry-run | -n) DRY_RUN=1 ;;
        --reinstall) VERB=reinstall ;;
        *)
            echo "Usage: $0 [--yes] [--dry-run] [--reinstall]" >&2
            exit 2
            ;;
    esac
done

command -v dnf >/dev/null || {
    echo "This installer requires Fedora and DNF." >&2
    exit 1
}
MUTTER_VERSION="$(sed -n 's/^Version:[[:space:]]*//p' "$ROOT/packaging/rpm/mutter.spec")"
SHELL_VERSION="$(sed -n 's/^Version:[[:space:]]*//p' "$ROOT/packaging/rpm/gnome-shell.spec")"
packages=("gnoblin-mutter:$MUTTER_VERSION" "gnoblin-shell:$SHELL_VERSION" "gnoblin-session:$SHELL_VERSION")
if rpm -q gnoblin-mutter-devel >/dev/null 2>&1; then
    packages+=("gnoblin-mutter-devel:$MUTTER_VERSION")
fi
rpms=()
for package in "${packages[@]}"; do
    name="${package%%:*}"
    version="${package#*:}"
    # Match the exact spec release, not an arbitrary older build in the directory.
    project=gnome-shell
    [[ "$name" == gnoblin-mutter* ]] && project=mutter
    release="$(rpmspec -q --srpm --qf '%{RELEASE}' "$ROOT/packaging/rpm/$project.spec")"
    mapfile -t matches < <(find "$RPM_DIR" -type f -name "$name-$version-$release.*.rpm" | sort)
    if [ "${#matches[@]}" -ne 1 ]; then
        echo "Expected one $name-$version-$release RPM under $RPM_DIR; found ${#matches[@]}." >&2
        echo "Build the private packages first; see docs/installation.md." >&2
        exit 1
    fi
    rpms+=("${matches[0]}")
done

python3 "$ROOT/scripts/check-rpm-isolation.py" "${rpms[@]}"
printf 'Install alongside GNOME: %s\n' "${rpms[@]}"
if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run: no package or session changes made."
    exit 0
fi

# Existing local overrides would hide the packaged session. Stop before making
# any changes if they are user-written files rather than registration symlinks.
units=(org.gnoblin.Shell.target org.gnoblin.Shell@wayland.service gnome-session@gnoblin.target.d/gnoblin.conf)
for unit in "${units[@]}"; do
    if [ -e "$UNIT_DIR/$unit" ] && [ ! -L "$UNIT_DIR/$unit" ]; then
        echo "Move the custom Gnoblin override aside first: $UNIT_DIR/$unit" >&2
        exit 1
    fi
done
sudo_args=()
[ "$(id -u)" -eq 0 ] || sudo_args=(sudo)
"${sudo_args[@]}" dnf "$VERB" "${DNF_OPTIONS[@]}" "${rpms[@]}"

# Only discard old Gnoblin registration links after a successful transaction.
for unit in "${units[@]}"; do
    if [ -L "$UNIT_DIR/$unit" ]; then
        unlink "$UNIT_DIR/$unit"
    fi
done
systemctl --user daemon-reload
rpm -q gnoblin-mutter gnoblin-shell gnoblin-session
printf '%s\n' 'Installed. Select Gnoblin at login; GNOME remains available.' \
    'Remove with: sudo dnf remove gnoblin-session gnoblin-shell gnoblin-mutter'

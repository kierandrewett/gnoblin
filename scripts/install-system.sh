#!/usr/bin/env bash
# Install Gnoblin from the official COPR; never replace or remove GNOME packages.
# Local RPM installation is retained only for package maintainers and must be
# explicitly requested with --local-rpms.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${GNOBLIN_RPM_DIR:-$HOME/rpmbuild/RPMS}"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
COPR_OWNER="${GNOBLIN_COPR_OWNER:-kierandrewett}"
COPR_PROJECT="${GNOBLIN_COPR_PROJECT:-gnoblin}"
SOURCE=copr
DRY_RUN=0
VERB=install
DNF_OPTIONS=()
for arg in "$@"; do
    case "$arg" in
        --yes | -y) DNF_OPTIONS+=(-y) ;;
        --dry-run | -n) DRY_RUN=1 ;;
        --reinstall) VERB=reinstall ;;
        --local-rpms) SOURCE=local ;;
        --copr) SOURCE=copr ;;
        *)
            echo "Usage: $0 [--yes] [--dry-run] [--reinstall] [--copr|--local-rpms]" >&2
            exit 2
            ;;
    esac
done

command -v dnf >/dev/null || {
    echo "This installer requires Fedora and DNF." >&2
    exit 1
}

# Existing local overrides would hide the packaged session. Stop before making
# any changes if they are user-written files rather than registration symlinks.
units=(org.gnoblin.Shell.target org.gnoblin.Shell@wayland.service gnome-session@gnoblin.target.d/gnoblin.conf)
for unit in "${units[@]}"; do
    if [ -e "$UNIT_DIR/$unit" ] && [ ! -L "$UNIT_DIR/$unit" ]; then
        if [ "$unit" = gnome-session@gnoblin.target.d/gnoblin.conf ] \
            && cmp -s "$ROOT/src/data/session/systemd-user/gnome-session@gnoblin.target.d.conf" "$UNIT_DIR/$unit"; then
            echo "Recognized managed Gnoblin user drop-in: $UNIT_DIR/$unit"
        else
            echo "Move the custom Gnoblin override aside first: $UNIT_DIR/$unit" >&2
            exit 1
        fi
    fi
done

if [ "$SOURCE" = copr ]; then
    copr="$COPR_OWNER/$COPR_PROJECT"
    fedora="$(rpm -E %fedora)"
    arch="$(rpm -E %_arch)"
    repo_url="https://download.copr.fedorainfracloud.org/results/$COPR_OWNER/$COPR_PROJECT/fedora-${fedora}-${arch}/"
    printf 'Official source: COPR %s (%s)\n' "$copr" "$repo_url"
    if [ "$DRY_RUN" -eq 1 ]; then
        dnf --assumeno --disablerepo='*' \
            --repofrompath="gnoblin-copr,$repo_url" \
            --setopt=gnoblin-copr.gpgcheck=1 --refresh \
            repoquery --available --qf '%{name}-%{evr}.%{arch}' \
            gnoblin-mutter gnoblin-shell gnoblin-session
        echo "Dry run: no repository, package, or session changes made."
        exit 0
    fi
    sudo_args=()
    [ "$(id -u)" -eq 0 ] || sudo_args=(sudo)
    "${sudo_args[@]}" dnf copr enable -y "$copr"
    mapfile -t copr_packages < <(
        dnf repoquery --available --latest-limit=1 --arch="$arch" \
            --qf $'%{name}-%{evr}.%{arch}\n' \
            gnoblin-mutter gnoblin-shell gnoblin-session | sort -u
    )
    if [ "${#copr_packages[@]}" -ne 3 ]; then
        echo "COPR did not expose the complete host package set; found:" >&2
        printf '  %s\n' "${copr_packages[@]}" >&2
        exit 1
    fi
    printf 'Installing COPR packages:\n'; printf '  %s\n' "${copr_packages[@]}"
    "${sudo_args[@]}" dnf "$VERB" "${DNF_OPTIONS[@]}" --refresh "${copr_packages[@]}"
else
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
        project=gnome-shell
        [[ "$name" == gnoblin-mutter* ]] && project=mutter
        release="$(rpmspec -q --srpm --qf '%{RELEASE}' "$ROOT/packaging/rpm/$project.spec")"
        mapfile -t matches < <(find "$RPM_DIR" -type f -name "$name-$version-$release.*.rpm" | sort)
        if [ "${#matches[@]}" -ne 1 ]; then
            echo "Expected one $name-$version-$release RPM under $RPM_DIR; found ${#matches[@]}." >&2
            echo "Use the official COPR path unless you are packaging locally: just install-session" >&2
            exit 1
        fi
        rpms+=("${matches[0]}")
    done
    python3 "$ROOT/scripts/check-rpm-isolation.py" "${rpms[@]}"
    printf 'Local RPM source: %s\n' "${rpms[@]}"
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "Dry run: no package or session changes made."
        exit 0
    fi
    sudo_args=()
    [ "$(id -u)" -eq 0 ] || sudo_args=(sudo)
    "${sudo_args[@]}" dnf "$VERB" "${DNF_OPTIONS[@]}" "${rpms[@]}"
fi

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

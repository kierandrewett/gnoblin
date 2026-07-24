#!/usr/bin/env bash
# Install the built gnoblin RPMs onto THIS host, so "Gnoblin" shows up at the
# login manager and boots the packaged build.
#
# This is the production path, and it is NOT the same thing as
# scripts/install-session.sh (which lays session data into a build prefix, and
# is what `just dev-session` and the RPM %install both call). Here nothing
# points at a dev prefix: the gnoblin-session package ships
# Exec=/usr/bin/gnoblin-session, its units to /usr/lib/systemd/user, and its
# .desktop to /usr/share/wayland-sessions.
#
# It exists because three things bite in sequence, and hitting them one at a
# time from a login screen is miserable:
#
#   1. Dev-prefix unit symlinks in ~/.config/systemd/user SHADOW the packaged
#      units -- that search path outranks /usr/lib/systemd/user, so after a
#      clean package install gnome-session would still resolve
#      org.gnoblin.Shell to whatever ./install had, or fail outright once that
#      prefix is stale or gone. `just dev-session-register` is what puts them
#      there, so anyone who tried the dev path first is carrying them.
#   2. The RPM release is `1.gnoblin`, which rpm sorts OLDER than a
#      `1.<anything-after-g>` build of the same version (e.g. 1.kdr). dnf
#      calls that a downgrade and declines without --allow-downgrade.
#   3. mutter/gnome-shell each Requires: their own -common at the exact same
#      version-release, so the arch and noarch RPMs have to go in one
#      transaction or dependency resolution fails.
#
# Usage: install-system.sh [--yes] [--dry-run]
#   --yes      pass -y to dnf (no transaction prompt)
#   --dry-run  resolve and print the transaction, change nothing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${GNOBLIN_RPM_DIR:-$HOME/rpmbuild/RPMS}"
UNIT_DIR="$HOME/.config/systemd/user"
ASSUME_YES=0
DRY_RUN=0

for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=1 ;;
    --dry-run|-n) DRY_RUN=1 ;;
    *) echo "unknown option: $arg -- usage: install-system.sh [--yes] [--dry-run]" >&2; exit 2 ;;
  esac
done

command -v dnf >/dev/null 2>&1 || { echo "dnf not found -- this path is Fedora-only (see packaging/{deb,arch}/README.md)" >&2; exit 1; }

spec_version() { sed -n 's/^Version:[[:space:]]*//p' "$ROOT/packaging/rpm/$1.spec" | head -1; }
MUTTER_V="$(spec_version mutter)"
SHELL_V="$(spec_version gnome-shell)"
[ -n "$MUTTER_V" ] && [ -n "$SHELL_V" ] || { echo "could not read Version: from packaging/rpm/*.spec" >&2; exit 1; }

# Match on the .gnoblin release specifically. A host that has built its own
# mutter/gnome-shell under another release tag will have those RPMs sitting in
# the same directory, and picking one of those up would be a silent disaster.
find_rpm() {
  local name="$1" version="$2"
  local found
  found="$(find "$RPM_DIR" -name "$name-$version-*.gnoblin*.rpm" -print 2>/dev/null | sort | tail -1)"
  [ -n "$found" ] || return 1
  printf '%s\n' "$found"
}

RPMS=()
MISSING=()
for pkg in "mutter:$MUTTER_V" "mutter-common:$MUTTER_V" "gnome-shell:$SHELL_V" "gnome-shell-common:$SHELL_V" "gnoblin-session:$SHELL_V"; do
  name="${pkg%%:*}"
  version="${pkg##*:}"
  if rpm_path="$(find_rpm "$name" "$version")"; then
    RPMS+=("$rpm_path")
  else
    MISSING+=("$name-$version-*.gnoblin*.rpm")
  fi
done

if [ "${#MISSING[@]}" -gt 0 ]; then
  echo "missing RPMs under $RPM_DIR:" >&2
  printf '     %s\n' "${MISSING[@]}" >&2
  echo >&2
  echo "build them first:  just rpm-all" >&2
  exit 1
fi

echo ">> installing gnoblin $SHELL_V (mutter $MUTTER_V) onto this host:"
printf '     %s\n' "${RPMS[@]##*/}"

# Warn if the tree has moved on since the packages were built. Cheap check, and
# it catches the common "I fixed that, why is it still broken" case.
NEWEST_RPM="$(printf '%s\n' "${RPMS[@]}" | xargs ls -t 2>/dev/null | head -1)"
if [ -n "$NEWEST_RPM" ]; then
  STALE="$(find "$ROOT/patches" "$ROOT/src" -type f -newer "$NEWEST_RPM" -print -quit 2>/dev/null || true)"
  if [ -n "$STALE" ]; then
    echo
    echo ">> WARNING: patches/ or src/ has changed since these RPMs were built"
    echo "   (e.g. ${STALE#"$ROOT"/}). Re-run 'just rpm-all' if you want those changes."
  fi
fi

# Shadowing dev units, see note 1 in the header. Only ever remove symlinks --
# a real file there is something the user wrote, and is theirs to deal with.
for unit in org.gnoblin.Shell.target "org.gnoblin.Shell@wayland.service"; do
  path="$UNIT_DIR/$unit"
  if [ -L "$path" ]; then
    [ "$DRY_RUN" -eq 1 ] || rm "$path"
    echo ">> removed shadowing dev unit: $path"
  elif [ -e "$path" ]; then
    echo ">> WARNING: $path exists and is not a symlink -- leaving it alone."
    echo "   It will shadow the packaged unit. Move it aside if the session misbehaves."
  fi
done

DNF_ARGS=(install --allow-downgrade)
[ "$ASSUME_YES" -eq 1 ] && DNF_ARGS+=(-y)
[ "$DRY_RUN" -eq 1 ] && DNF_ARGS+=(--assumeno)

SUDO=()
[ "$(id -u)" -eq 0 ] || SUDO=(sudo)

echo
"${SUDO[@]}" dnf "${DNF_ARGS[@]}" "${RPMS[@]}"

if [ "$DRY_RUN" -eq 1 ]; then
  echo
  echo ">> dry run, nothing changed."
  exit 0
fi

# Pick up the packaged units now that the shadowing ones are gone.
systemctl --user daemon-reload || true

echo
echo ">> verifying:"
FAIL=0
for pkg in mutter mutter-common gnome-shell gnome-shell-common gnoblin-session; do
  if installed="$(rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}' "$pkg" 2>/dev/null)" && [[ "$installed" == *gnoblin* ]]; then
    echo "     $installed"
  else
    echo "     MISSING or not a gnoblin build: $pkg (${installed:-not installed})" >&2
    FAIL=1
  fi
done

DESKTOP=/usr/share/wayland-sessions/gnoblin.desktop
if [ -f "$DESKTOP" ]; then
  echo "     $DESKTOP -> $(sed -n 's/^Exec=//p' "$DESKTOP" | head -1)"
else
  echo "     MISSING: $DESKTOP" >&2
  FAIL=1
fi

if [ "$FAIL" -ne 0 ]; then
  echo >&2
  echo "install did not land cleanly -- see above." >&2
  exit 1
fi

cat <<EOF

>> Done. Log out and pick "Gnoblin" from the session menu at the login screen.

   It boots bare: no top bar, no dash, no overview. Bring your own chrome.
   Sanity checks and the full first-login checklist are in
   docs/real-hardware-verification.md.

>> Undo:
     sudo dnf remove gnoblin-session
     sudo dnf downgrade mutter gnome-shell    # or: sudo dnf history undo last
EOF

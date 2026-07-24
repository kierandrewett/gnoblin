#!/usr/bin/env bash
# Install the built gnoblin RPMs onto THIS host, so "Gnoblin" shows up at the
# login manager and boots the packaged build.
#
# This is the production path, and it is NOT the same thing as
# scripts/install-session.sh (which lays session data into a build prefix, and
# is what `just dev-session` and the RPM %install both call). Here nothing
# points at a dev prefix: gnoblin-session ships Exec=/opt/gnoblin/bin/gnoblin-session,
# its units to /usr/lib/systemd/user, and its .desktop to
# /usr/share/wayland-sessions.
#
# The packages are gnoblin-mutter / gnoblin-gnome-shell, installed under
# /opt/gnoblin. They sit ALONGSIDE the distro's own mutter and gnome-shell --
# nothing is replaced, downgraded, or conflicted with, and a normal GNOME
# session is unaffected. Only the gnoblin session looks in the prefix, via
# gnoblin-env.sh.
#
# Two stages, because gnoblin-gnome-shell links the patched libmutter and so
# BuildRequires gnoblin-mutter-devel: mutter has to be built AND installed
# before the shell can even be built. The script installs what it can, then
# tells you the exact next command. Re-running it continues where it stopped.
#
# Usage: install-system.sh [--yes] [--dry-run] [--reinstall]
#   --yes        pass -y to dnf (no transaction prompt)
#   --dry-run    resolve and print the transaction, change nothing
#   --reinstall  dnf reinstall instead of install, for iterating on a rebuild
#                that kept the same version-release
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${GNOBLIN_RPM_DIR:-$HOME/rpmbuild/RPMS}"
UNIT_DIR="$HOME/.config/systemd/user"
PREFIX=/opt/gnoblin
ASSUME_YES=0
DRY_RUN=0
REINSTALL=0

for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=1 ;;
    --dry-run|-n) DRY_RUN=1 ;;
    --reinstall) REINSTALL=1 ;;
    *) echo "unknown option: $arg -- usage: install-system.sh [--yes] [--dry-run] [--reinstall]" >&2; exit 2 ;;
  esac
done

command -v dnf >/dev/null 2>&1 || { echo "dnf not found -- this path is Fedora-only (see packaging/{deb,arch}/README.md)" >&2; exit 1; }

spec_version() { sed -n 's/^Version:[[:space:]]*//p' "$ROOT/packaging/rpm/gnoblin-$1.spec" | head -1; }
MUTTER_V="$(spec_version mutter)"
SHELL_V="$(spec_version gnome-shell)"
[ -n "$MUTTER_V" ] && [ -n "$SHELL_V" ] || { echo "could not read Version: from packaging/rpm/gnoblin-*.spec" >&2; exit 1; }

find_rpm() {
  local name="$1" version="$2" found
  found="$(find "$RPM_DIR" -name "$name-$version-*.gnoblin*.rpm" -print 2>/dev/null | sort | tail -1)"
  [ -n "$found" ] || return 1
  printf '%s\n' "$found"
}

SUDO=()
[ "$(id -u)" -eq 0 ] || SUDO=(sudo)

# Resolve a stage's RPMs into RPMS/MISSING. Subpackages beyond the required set
# are pulled in only when already installed, so an existing gnoblin-mutter-tests
# is kept in step rather than being left pinned to a stale build.
resolve_stage() {
  local version="$1"; shift
  local name rpm_path built
  RPMS=()
  MISSING=()
  for name in "$@"; do
    if rpm_path="$(find_rpm "$name" "$version")"; then
      RPMS+=("$rpm_path")
    else
      MISSING+=("$name-$version-*.gnoblin*.rpm")
    fi
  done
  for built in "$RPM_DIR"/*/*.gnoblin*.rpm; do
    [ -e "$built" ] || continue
    name="$(rpm -qp --qf '%{NAME}' "$built" 2>/dev/null)" || continue
    case "$name" in *-debuginfo|*-debugsource) continue ;; esac
    [ "$(rpm -qp --qf '%{VERSION}' "$built" 2>/dev/null)" = "$version" ] || continue
    printf '%s\n' "${RPMS[@]:-}" | grep -qF "/$name-$version-" && continue
    rpm -q "$name" >/dev/null 2>&1 || continue
    RPMS+=("$built")
    echo ">> also updating installed subpackage: $name"
  done
}

run_dnf() {
  local verb=install
  [ "$REINSTALL" -eq 1 ] && verb=reinstall
  local args=("$verb")
  [ "$verb" = install ] && args+=(--allow-downgrade)
  [ "$ASSUME_YES" -eq 1 ] && args+=(-y)
  [ "$DRY_RUN" -eq 1 ] && args+=(--assumeno)
  echo
  "${SUDO[@]}" dnf "${args[@]}" "$@"
}

# Stage 1: the compositor. gnoblin-mutter-devel is not a runtime dependency,
# but gnoblin-gnome-shell cannot be BUILT without it, and building the shell is
# the very next thing anyone running this script needs to do.
echo ">> stage 1/2: gnoblin-mutter $MUTTER_V"
resolve_stage "$MUTTER_V" gnoblin-mutter gnoblin-mutter-common gnoblin-mutter-devel
if [ "${#MISSING[@]}" -gt 0 ]; then
  echo "missing RPMs under $RPM_DIR:" >&2
  printf '     %s\n' "${MISSING[@]}" >&2
  echo >&2
  echo "build them first:  just rpm mutter" >&2
  exit 1
fi
printf '     %s\n' "${RPMS[@]##*/}"
run_dnf "${RPMS[@]}"

# Stage 2: the shell and the session. Only buildable once stage 1 is installed.
echo
echo ">> stage 2/2: gnoblin-gnome-shell $SHELL_V"
resolve_stage "$SHELL_V" gnoblin-gnome-shell gnoblin-gnome-shell-common gnoblin-session
if [ "${#MISSING[@]}" -gt 0 ]; then
  echo "missing RPMs under $RPM_DIR:" >&2
  printf '     %s\n' "${MISSING[@]}" >&2
  cat >&2 <<EOF

gnoblin-mutter is installed now, which is what gnoblin-gnome-shell needs to
build against. Build the shell and re-run this:

     just rpm gnome-shell
     just install-session
EOF
  exit 1
fi
printf '     %s\n' "${RPMS[@]##*/}"

# Dev-prefix unit symlinks SHADOW the packaged units: ~/.config/systemd/user
# outranks /usr/lib/systemd/user, so leaving them means gnome-session resolves
# org.gnoblin.Shell to whatever ./install had. `just dev-session-register` is
# what puts them there. Only ever remove symlinks -- a real file is something
# the user wrote, and is theirs.
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

run_dnf "${RPMS[@]}"

if [ "$DRY_RUN" -eq 1 ]; then
  echo
  echo ">> dry run, nothing changed."
  exit 0
fi

systemctl --user daemon-reload || true

echo
echo ">> verifying:"
FAIL=0
for pkg in gnoblin-mutter gnoblin-mutter-common gnoblin-gnome-shell gnoblin-gnome-shell-common gnoblin-session; do
  if installed="$(rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}' "$pkg" 2>/dev/null)"; then
    echo "     $installed"
  else
    echo "     MISSING: $pkg" >&2
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

# The whole point of the split: the distro's packages must still be there.
for pkg in mutter gnome-shell; do
  if installed="$(rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}' "$pkg" 2>/dev/null)"; then
    echo "     system $installed (untouched)"
  else
    echo "     note: no system $pkg installed -- gnoblin did not remove it, it was already absent"
  fi
done

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
     sudo dnf remove gnoblin-session gnoblin-gnome-shell gnoblin-mutter
EOF

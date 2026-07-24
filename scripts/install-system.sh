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

# A post-split gnoblin package installs ONLY into the prefix, plus the two
# paths that genuinely cannot live there -- the login entry, because login
# managers scan a fixed directory, and the systemd user units -- and rpm's own
# per-build-id debuginfo symlinks.
#
# Pre-split builds are still sitting in the same RPM directory under the SAME
# NAMES: gnoblin-session-49.6-1.gnoblin.fc43 installs /usr/bin/gnoblin-session,
# /usr/share/gnome-shell/modes/gnoblin.json and friends, and Requires
# `gnome-shell = 49.6-1.gnoblin.fc43` -- the replace-style package. Resolving
# one would drag the old gnome-shell in and overwrite the distro's. The name
# cannot tell them apart. The layout can.
# Deliberately pure bash, no grep. This check is what stands between a stale
# RPM and an overwritten distro package, and `grep` is not reliably GNU grep --
# on this machine an interactive shell function routes it to ugrep, whose
# exit status for -q combined with -v differs. A case statement has no such
# ambiguity.
STALE_SEEN=0
rpm_layout_ok() {
  local path
  while IFS= read -r path; do
    case "$path" in
      "$PREFIX"|"$PREFIX"/*) ;;
      /usr/share/wayland-sessions|/usr/share/wayland-sessions/*) ;;
      /usr/lib/systemd/user|/usr/lib/systemd/user/*) ;;
      /usr/lib/.build-id|/usr/lib/.build-id/*) ;;
      *) return 1 ;;
    esac
  done < <(rpm -qlp "$1" 2>/dev/null)
  return 0
}

find_rpm() {
  local name="$1" version="$2" cand
  while IFS= read -r cand; do
    [ -n "$cand" ] || continue
    if rpm_layout_ok "$cand"; then
      printf '%s\n' "$cand"
      return 0
    fi
    echo ">> ignoring pre-split RPM (installs outside $PREFIX): ${cand##*/}" >&2
    STALE_SEEN=1
  done < <(find "$RPM_DIR" -name "$name-$version-*.gnoblin*.rpm" -print 2>/dev/null | sort -r)
  return 1
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
    # ONLY gnoblin-* packages. Older `.gnoblin`-release builds of the plain
    # `mutter`/`gnome-shell` names -- from before the split, when these specs
    # replaced the distro packages -- may still be sitting in the same
    # directory, and they match the same glob. Installing one would replace the
    # system package, which is the exact thing this packaging exists to avoid.
    case "$name" in gnoblin-*) ;; *) continue ;; esac
    case "$name" in *-debuginfo|*-debugsource) continue ;; esac
    rpm_layout_ok "$built" || continue
    [ "$(rpm -qp --qf '%{VERSION}' "$built" 2>/dev/null)" = "$version" ] || continue
    already=0
    for have in "${RPMS[@]:-}"; do
      case "${have##*/}" in "$name-$version-"*) already=1 ;; esac
    done
    [ "$already" -eq 1 ] && continue
    rpm -q "$name" >/dev/null 2>&1 || continue
    RPMS+=("$built")
    echo ">> also updating installed subpackage: $name"
  done
}

# True when every RPM in the set is already installed at exactly this NEVRA.
# Lets a re-run skip straight to the stage that still needs doing instead of
# prompting for a password to reinstall what is already there.
stage_satisfied() {
  local f file_nevra installed
  for f in "$@"; do
    file_nevra="$(rpm -qp --qf '%{NAME}-%{VERSION}-%{RELEASE}' "$f" 2>/dev/null)" || return 1
    installed="$(rpm -q --qf '%{NAME}-%{VERSION}-%{RELEASE}' "${file_nevra%-*-*}" 2>/dev/null)" || return 1
    [ "$installed" = "$file_nevra" ] || return 1
  done
  return 0
}

run_dnf() {
  if [ "$REINSTALL" -eq 0 ] && stage_satisfied "$@"; then
    echo "     already installed at this version -- nothing to do"
    echo "     (use 'just install-session reinstall' to re-apply a rebuild)"
    return 0
  fi
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

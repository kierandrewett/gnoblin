#!/usr/bin/env bash
# Install the built gnoblin RPMs onto THIS host, so "Gnoblin" shows up at the
# login manager and boots the packaged build.
#
# This is the production path, and it is NOT the same thing as
# scripts/install-session.sh (which lays session data into a build prefix, and
# is what `just dev-session` and the RPM %install both call).
#
# The packages install to STANDARD PATHS (/usr), which means the gnoblin
# mutter and gnome-shell REPLACE the distro's -- two packages cannot both own
# /usr/bin/gnome-shell. That is the deliberate trade: standard paths, one set
# of binaries, no prefix juggling and no lookup-path games. dnf shows the
# replacement before you confirm, and `dnf downgrade` puts the distro build
# back.
#
# Four things bite here, and hitting them one at a time from a login screen is
# miserable:
#
#   1. Dev-prefix unit symlinks in ~/.config/systemd/user SHADOW the packaged
#      units -- that search path outranks /usr/lib/systemd/user, so after a
#      clean package install gnome-session resolves org.gnoblin.Shell to
#      whatever ./install had, or fails outright once that prefix is stale or
#      gone. `just dev-session-register` is what puts them there.
#   2. The RPM release is `1.gnoblin`, which rpm sorts OLDER than a
#      `1.<anything-after-g>` build of the same version (1.kdr, say). dnf calls
#      that a downgrade and declines without --allow-downgrade.
#   3. Subpackages like mutter-devel carry an exact
#      `Requires: mutter = %{version}-%{release}`, so swapping the base package
#      out from under an installed one fails the whole transaction with
#      "none of the providers can be installed". Any already-installed
#      subpackage has to be replaced in the same go.
#   4. An earlier iteration of this packaging installed under /opt/gnoblin as
#      gnoblin-mutter / gnoblin-gnome-shell. Those are a dead end now and are
#      removed first, so the two layouts never coexist.
#
# Usage: install-system.sh [--yes] [--dry-run] [--reinstall]
#   --yes        pass -y to dnf (no transaction prompt)
#   --dry-run    resolve and print, change nothing
#   --reinstall  dnf reinstall instead of install, for iterating on a rebuild
#                that kept the same version-release
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${GNOBLIN_RPM_DIR:-$HOME/rpmbuild/RPMS}"
UNIT_DIR="$HOME/.config/systemd/user"
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

spec_version() { sed -n 's/^Version:[[:space:]]*//p' "$ROOT/packaging/rpm/$1.spec" | head -1; }
MUTTER_V="$(spec_version mutter)"
SHELL_V="$(spec_version gnome-shell)"
[ -n "$MUTTER_V" ] && [ -n "$SHELL_V" ] || { echo "could not read Version: from packaging/rpm/*.spec" >&2; exit 1; }

SUDO=()
[ "$(id -u)" -eq 0 ] || SUDO=(sudo)

# An /opt-era package is identifiable by where it installs, not by its name --
# gnoblin-session exists in both layouts. Layout is the reliable discriminator.
# Deliberately pure bash: `grep` is not reliably GNU grep here (an interactive
# shell function routes it to ugrep, whose -q/-v exit status differs), and this
# check decides whether files get removed.
installs_under_opt() {
  local path
  while IFS= read -r path; do
    case "$path" in /opt/gnoblin|/opt/gnoblin/*) return 0 ;; esac
  done < <(rpm -ql "$1" 2>/dev/null)
  return 1
}

STALE_OPT=()
for pkg in gnoblin-mutter gnoblin-mutter-common gnoblin-mutter-devel gnoblin-mutter-tests \
           gnoblin-gnome-shell gnoblin-gnome-shell-common gnoblin-session; do
  rpm -q "$pkg" >/dev/null 2>&1 || continue
  installs_under_opt "$pkg" || continue
  STALE_OPT+=("$pkg")
done

if [ "${#STALE_OPT[@]}" -gt 0 ]; then
  echo ">> removing the /opt-era gnoblin packages first:"
  printf '     %s\n' "${STALE_OPT[@]}"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "     (dry run -- not removed)"
  else
    args=(remove)
    [ "$ASSUME_YES" -eq 1 ] && args+=(-y)
    "${SUDO[@]}" dnf "${args[@]}" "${STALE_OPT[@]}"
  fi
  echo
fi

find_rpm() {
  local name="$1" version="$2" found
  found="$(find "$RPM_DIR" -name "$name-$version-*.gnoblin*.rpm" -print 2>/dev/null | sort | tail -1)"
  [ -n "$found" ] || return 1
  printf '%s\n' "$found"
}

RPMS=()
NAMES=()
MISSING=()
for pkg in "mutter:$MUTTER_V" "mutter-common:$MUTTER_V" "gnome-shell:$SHELL_V" "gnome-shell-common:$SHELL_V" "gnoblin-session:$SHELL_V"; do
  name="${pkg%%:*}"
  version="${pkg##*:}"
  if rpm_path="$(find_rpm "$name" "$version")"; then
    RPMS+=("$rpm_path")
    NAMES+=("$name")
  else
    MISSING+=("$name-$version-*.gnoblin*.rpm")
  fi
done

have_name() {
  local needle="$1" n
  for n in "${NAMES[@]}"; do [ "$n" = "$needle" ] && return 0; done
  return 1
}

# Point 3: keep already-installed subpackages in step with their base package.
# Only ones ALREADY installed -- this is not the place to start handing someone
# mutter-tests they never asked for.
for built in "$RPM_DIR"/*/*.gnoblin*.rpm; do
  [ -e "$built" ] || continue
  name="$(rpm -qp --qf '%{NAME}' "$built" 2>/dev/null)" || continue
  case "$name" in gnoblin-*) continue ;; esac
  case "$name" in *-debuginfo|*-debugsource) continue ;; esac
  have_name "$name" && continue
  rpm -q "$name" >/dev/null 2>&1 || continue
  RPMS+=("$built")
  NAMES+=("$name")
  echo ">> also replacing installed subpackage: $name"
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

NEWEST_RPM="$(printf '%s\n' "${RPMS[@]}" | xargs ls -t 2>/dev/null | head -1)"
if [ -n "$NEWEST_RPM" ]; then
  STALE="$(find "$ROOT/patches" "$ROOT/src" -type f -newer "$NEWEST_RPM" -print -quit 2>/dev/null || true)"
  if [ -n "$STALE" ]; then
    echo
    echo ">> WARNING: patches/ or src/ has changed since these RPMs were built"
    echo "   (e.g. ${STALE#"$ROOT"/}). Re-run 'just rpm-all' if you want those changes."
  fi
fi

# Point 1. Only ever remove symlinks -- a real file there is something the user
# wrote, and is theirs to deal with.
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

VERB=install
[ "$REINSTALL" -eq 1 ] && VERB=reinstall
DNF_ARGS=("$VERB")
[ "$VERB" = install ] && DNF_ARGS+=(--allow-downgrade)   # point 2
[ "$ASSUME_YES" -eq 1 ] && DNF_ARGS+=(-y)
[ "$DRY_RUN" -eq 1 ] && DNF_ARGS+=(--assumeno)

echo
"${SUDO[@]}" dnf "${DNF_ARGS[@]}" "${RPMS[@]}"

if [ "$DRY_RUN" -eq 1 ]; then
  echo
  echo ">> dry run, nothing changed."
  exit 0
fi

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

# The session cannot start without these three in system paths: the login entry
# GDM scans, and the session definition gnome-session-binary reads while running
# under the systemd user manager (which never sees a session wrapper's env).
for f in /usr/share/wayland-sessions/gnoblin.desktop \
         /usr/share/gnome-session/sessions/gnoblin.session \
         /usr/share/gnome-shell/modes/gnoblin.json; do
  if [ -f "$f" ]; then
    echo "     $f"
  else
    echo "     MISSING: $f" >&2
    FAIL=1
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
     sudo dnf remove gnoblin-session
     sudo dnf downgrade mutter gnome-shell    # or: sudo dnf history undo last
EOF

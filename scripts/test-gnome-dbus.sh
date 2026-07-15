#!/usr/bin/env bash
# Boot patched gnome-shell (gnoblin mode) headless and exercise the org.gnoblin.*
# control protocol end-to-end over D-Bus:
#   - org.gnoblin.Shell.Ping        -> "pong"
#   - org.gnoblin.Shell.GetVersion  -> "*-gnoblin"
#   - org.gnoblin.Shell.Reload      -> triggers a soft in-process reload (log check)
#
# gnome-shell AND the gdbus calls run inside one dbus-run-session, so they share
# the same isolated session bus (no host bus leakage, no address plumbing).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
source "$ROOT/scripts/gnoblin-state.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR
LAST_LOG="$GNOBLIN_STATE_DIR/dbus-last.log"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
SHELL_BIN="$PREFIX/bin/gnome-shell"
MONITOR="${MONITOR:-1280x800}"

[ -x "$SHELL_BIN" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

source "$ROOT/src/tools/gnoblin-env.sh"
gnoblin_env_apply "$PREFIX"
export GDK_BACKEND=wayland

DK="$(mktemp -d /tmp/gnoblin-dbus.XXXXXX)"
mkdir -p "$DK"/{data,config,cache,home}
export HOME="$DK/home" XDG_DATA_HOME="$DK/data" XDG_CONFIG_HOME="$DK/config" XDG_CACHE_HOME="$DK/cache"
# Seed one valid record for each portal scope. The backend normally writes these
# after an approved session reaches its ready state.
SCREEN_GRANT_ID="$(printf '%064x' 1).grant"
REMOTE_GRANT_ID="$(printf '%064x' 2).grant"
export SCREEN_GRANT_ID REMOTE_GRANT_ID
mkdir -p "$DK/data/gnoblin/portal-grants"/{screen-cast,remote-desktop}
cat > "$DK/data/gnoblin/portal-grants/screen-cast/$SCREEN_GRANT_ID" <<'EOF'
[Grant]
version=1
portal=screen-cast
identity=app-id:org.example.Cast
device-types=0
clipboard-enabled=false
streams=[(uint32 0, uint32 1, <'monitor-A'>)]
EOF
cat > "$DK/data/gnoblin/portal-grants/remote-desktop/$REMOTE_GRANT_ID" <<'EOF'
[Grant]
version=1
portal=remote-desktop
identity=host-exe:/usr/bin/example-remote
device-types=3
clipboard-enabled=true
streams=[(uint32 0, uint32 1, <'monitor-B'>)]
EOF
export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 GSETTINGS_BACKEND=dconf GTK_A11Y=none NO_AT_BRIDGE=1
export DISP="gnoblin-dbus-$$" SHELL_LOG="$DK/shell.log"

cleanup() {
  for proc in /proc/[0-9]*; do
    e="$({ tr '\0' '\n' < "$proc/environ"; } 2>/dev/null || true)"
    case "$e" in *"WAYLAND_DISPLAY=$DISP"*) kill -KILL "${proc##*/}" 2>/dev/null || true ;; esac
  done
  [ -f "$SHELL_LOG" ] && gnoblin_publish_log "$SHELL_LOG" dbus-last.log 2>/dev/null || true
  rm -rf "$DK"
}
trap cleanup EXIT INT TERM HUP

CONF="$(python3 "$ROOT/scripts/devkit_dbus.py" "$DK" "$ROOT")" || exit 1

# Everything below shares the one dbus-run-session bus.
dbus-run-session --config-file="$CONF" -- bash -euo pipefail -c '
  source "$ROOT/scripts/gnoblin-test-lib.sh"
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin \
    --virtual-monitor "'"$MONITOR"'" --wayland-display "$DISP" >"$SHELL_LOG" 2>&1 &
  SHELL_PID=$!

  # Wait for the shell to own org.gnoblin.Shell (implies started + component up).
  if ! gdbus wait --session --timeout 30 org.gnoblin.Shell; then
    echo "FAIL: org.gnoblin.Shell never appeared"; tail -20 "$SHELL_LOG"; kill $SHELL_PID 2>/dev/null; exit 1
  fi

  rc=0
  call() { gdbus call --session --dest org.gnoblin.Shell \
             --object-path /org/gnoblin/Shell --method "org.gnoblin.Shell.$1" 2>&1; }

  callp() { gdbus call --session --dest org.gnoblin.Shell \
              --object-path /org/gnoblin/Shell --method "org.gnoblin.Shell.$@" 2>&1; }

  feature_is() {
    case "$(callp GetFeature "$1")" in
      *"$2"*) return 0 ;;
      *) return 1 ;;
    esac
  }

  feature_signal_count_is() {
    [ "$(grep -c FeatureChanged "$SIGNAL_LOG" || true)" -eq "$1" ]
  }

  ping="$(call Ping)";        echo "Ping       -> $ping"
  ver="$(call GetVersion)";   echo "GetVersion -> $ver"
  reload="$(call Reload)";    echo "Reload     -> $reload"

  case "$ping"   in *pong*)     echo "  ok: Ping";;        *) echo "  FAIL: Ping"; rc=1;; esac
  case "$ver"    in *-gnoblin*) echo "  ok: GetVersion";;  *) echo "  FAIL: GetVersion"; rc=1;; esac
  # Reload is void; its reply must arrive after asynchronous work completes.
  if grep -qE "gnoblin: soft-reload .* complete" "$SHELL_LOG"; then
    echo "  ok: Reload waited for soft-reload completion"
  else
    echo "  FAIL: Reload replied before completion"; rc=1
  fi
  if callp ReloadExtension missing@gnoblin >/dev/null; then
    echo "  FAIL: unknown extension reload reported success"; rc=1
  else
    echo "  ok: failed extension reload returned a D-Bus error"
  fi

  # --- feature toggles ---
  feats="$(call ListFeatures)"; echo "ListFeatures -> $feats"
  case "$feats" in *osd*screenshot*|*screenshot*osd*) echo "  ok: ListFeatures (osd + screenshot)";; *) echo "  FAIL: ListFeatures"; rc=1;; esac

  g0="$(callp GetFeature osd)";                 echo "GetFeature osd (default) -> $g0"
  case "$g0" in *true*)  echo "  ok: osd default enabled";; *) echo "  FAIL: osd default"; rc=1;; esac

  callp SetFeature osd false >/dev/null
  g1="$(callp GetFeature osd)";                 echo "GetFeature osd (after off) -> $g1"
  case "$g1" in *false*) echo "  ok: SetFeature osd off";; *) echo "  FAIL: SetFeature off"; rc=1;; esac

  callp SetFeature osd true >/dev/null
  g2="$(callp GetFeature osd)";                 echo "GetFeature osd (after on) -> $g2"
  case "$g2" in *true*)  echo "  ok: SetFeature osd on";; *) echo "  FAIL: SetFeature on"; rc=1;; esac

  # Changes made outside org.gnoblin.Shell must follow the same live apply and
  # FeatureChanged path, without duplicating the signal on each transition.
  SIGNAL_LOG="$XDG_CACHE_HOME/feature-signals.log"
  gdbus monitor --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell >"$SIGNAL_LOG" 2>&1 &
  SIGNAL_PID=$!
  gnoblin_wait_for_log "$SIGNAL_LOG" "Monitoring signals" 5
  gsettings set org.gnoblin.shell disabled-features "['\''screenshot'\'']"
  if gnoblin_wait_until 10 feature_is screenshot false; then
    echo "  ok: direct GSettings disable applied live"
  else
    echo "  FAIL: direct GSettings disable not applied"; rc=1
  fi
  gsettings set org.gnoblin.shell disabled-features "[]"
  if gnoblin_wait_until 10 feature_is screenshot true &&
     gnoblin_wait_until 10 feature_signal_count_is 2; then
    echo "  ok: direct GSettings transitions emitted exactly once"
  else
    echo "  FAIL: direct GSettings transition signals"; cat "$SIGNAL_LOG"; rc=1
  fi
  kill "$SIGNAL_PID" 2>/dev/null || true

  gu="$(callp GetFeature bogus)";               echo "GetFeature bogus -> $gu"
  case "$gu" in *false*) echo "  ok: unknown feature -> false";; *) echo "  FAIL: unknown feature"; rc=1;; esac

  # per-OSD toggles (master osd + per-type)
  case "$feats" in *osd-volume*) echo "  ok: per-OSD features listed (osd-volume)";; *) echo "  FAIL: no per-OSD features"; rc=1;; esac
  callp SetFeature osd-volume false >/dev/null
  gv="$(callp GetFeature osd-volume)";          echo "GetFeature osd-volume (after off) -> $gv"
  case "$gv" in *false*) echo "  ok: SetFeature osd-volume off";; *) echo "  FAIL: per-OSD set"; rc=1;; esac
  callp SetFeature osd-volume true >/dev/null

  # typed, portal-scoped grants: list both kinds, reject traversal, revoke one
  grants="$(callp ListPortalGrants)"; echo "ListPortalGrants -> $grants"
  case "$grants" in
    *screen-cast*app-id:org.example.Cast*) echo "  ok: screen-cast grant listed";;
    *) echo "  FAIL: screen-cast grant missing"; rc=1;;
  esac
  case "$grants" in
    *remote-desktop*host-exe:/usr/bin/example-remote*uint32\ 3*true*true*) echo "  ok: remote-desktop capabilities listed";;
    *) echo "  FAIL: remote-desktop grant missing"; rc=1;;
  esac
  if callp RevokePortalGrant screen-cast ../outside.grant >/dev/null; then
    echo "  FAIL: invalid grant id accepted"; rc=1
  else
    echo "  ok: invalid grant id rejected"
  fi
  callp RevokePortalGrant screen-cast "$SCREEN_GRANT_ID" >/dev/null
  grants2="$(callp ListPortalGrants)"; echo "ListPortalGrants (after revoke) -> $grants2"
  case "$grants2" in
    *org.example.Cast*) echo "  FAIL: screen-cast grant not revoked"; rc=1;;
    *example-remote*) echo "  ok: scoped revoke retained remote-desktop grant";;
    *) echo "  FAIL: scoped revoke removed the wrong grant"; rc=1;;
  esac

  kill $SHELL_PID 2>/dev/null || true
  exit $rc
'
rc=$?
[ "$rc" = 0 ] && echo ">> RESULT: PASS (org.gnoblin.* round-trip)" || echo ">> RESULT: FAIL (rc=$rc). log -> $LAST_LOG"
exit "$rc"

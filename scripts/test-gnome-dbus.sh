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
  gsettings set org.gnome.desktop.input-sources sources "[('\''xkb'\'', '\''us'\''), ('\''xkb'\'', '\''gb'\'')]"
  # The private test session enables Eval only to emit MetaDisplay::overlay-key.
  # Headless Mutter has no synthetic Super input path. The production shell
  # remains in safe mode.
  "'"$SHELL_BIN"'" --headless --wayland --no-x11 --mode=gnoblin --unsafe-mode \
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

  super_release_signal_is_valid() {
    grep -qE "SuperReleased.*uint32 1.*uint64 [1-9][0-9]*" "$SUPER_SIGNAL_LOG"
  }

  osd_signal_count() {
    grep -c OsdRequested "$OSD_SIGNAL_LOG" || true
  }

  osd_signal_records_are_valid_after() {
    local previous_count="$1"
    local count=0
    local found=0
    local line
    local normalised
    while IFS= read -r line; do
      case "$line" in
        *OsdRequested*)
          count=$((count + 1))
          if [ "$count" -le "$previous_count" ]; then
            continue
          fi
          found=1
          normalised="${line/, int32 0,/, 0,}"
          case "$normalised" in
            *"OsdRequested (uint32 2, 0, '\''audio-volume-high-symbolic'\'', '\''Volume'\'', 0.5, 1.0, ${OSD_OUTPUT_NAMES})"*) ;;
            *) return 1 ;;
          esac
          ;;
      esac
    done < "$OSD_SIGNAL_LOG"
    [ "$found" -eq 1 ]
  }

  show_test_osd() {
    gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
      --method org.gnome.Shell.Eval "Main.osdWindowManager.showAll(new Gio.ThemedIcon({name: '\''audio-volume-high-symbolic'\''}), '\''Volume'\'', 0.5, 1);" >/dev/null
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

  # --- input source + privacy state ---
  sources="$(call ListInputSources)"; echo "ListInputSources -> $sources"
  case "$sources" in
    *xkb*us*gb*) echo "  ok: input sources listed";;
    *) echo "  FAIL: configured input sources missing"; rc=1;;
  esac
  input_source_is() {
    case "$(call GetCurrentInputSource)" in
      *"$1"*) return 0 ;;
      *) return 1 ;;
    esac
  }
  if callp SetInputSource xkb missing >/dev/null; then
    echo "  FAIL: unknown input source accepted"; rc=1
  else
    echo "  ok: unknown input source rejected"
  fi
  INPUT_SOURCE_SIGNAL_LOG="$XDG_CACHE_HOME/input-source-signals.log"
  gdbus monitor --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell >"$INPUT_SOURCE_SIGNAL_LOG" 2>&1 &
  INPUT_SOURCE_SIGNAL_PID=$!
  gnoblin_wait_for_log "$INPUT_SOURCE_SIGNAL_LOG" "Monitoring signals" 5
  callp SetInputSource xkb gb >/dev/null
  if gnoblin_wait_until 10 input_source_is gb &&
     gnoblin_wait_until 10 grep -q "InputSourceChanged.*xkb.*gb" "$INPUT_SOURCE_SIGNAL_LOG"; then
    echo "  ok: input source switched and signalled"
  else
    echo "  FAIL: input source switch"; cat "$INPUT_SOURCE_SIGNAL_LOG"; rc=1
  fi
  kill "$INPUT_SOURCE_SIGNAL_PID" 2>/dev/null || true

  privacy="$(call GetPrivacyState)"; echo "GetPrivacyState -> $privacy"
  case "$privacy" in
    *true*|*false*) echo "  ok: privacy state returned";;
    *) echo "  FAIL: privacy state"; rc=1;;
  esac

  # --- Super-release signal ---
  # GNOME Shell Eval lets this isolated test emit the exact
  # MetaDisplay::overlay-key signal without changing the production key path.
  SUPER_SIGNAL_LOG="$XDG_CACHE_HOME/super-release-signals.log"
  gdbus monitor --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell >"$SUPER_SIGNAL_LOG" 2>&1 &
  SUPER_SIGNAL_PID=$!
  gnoblin_wait_for_log "$SUPER_SIGNAL_LOG" "Monitoring signals" 5
  gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
    --method org.gnome.Shell.Eval "global.display.emit('\''overlay-key'\'');" >/dev/null
  if gnoblin_wait_until 5 super_release_signal_is_valid; then
    echo "  ok: SuperReleased emitted version 1 with a monotonic timestamp"
  else
    echo "  FAIL: SuperReleased signal"; cat "$SUPER_SIGNAL_LOG"; rc=1
  fi
  kill "$SUPER_SIGNAL_PID" 2>/dev/null || true

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

  # Eval is private to this unsafe test shell. It drives the exact
  # OsdWindowManager chokepoint without adding a production test command.
  OSD_SIGNAL_LOG="$XDG_CACHE_HOME/osd-signals.log"
  gdbus monitor --session --dest org.gnoblin.Shell \
    --object-path /org/gnoblin/Shell >"$OSD_SIGNAL_LOG" 2>&1 &
  OSD_SIGNAL_PID=$!
  gnoblin_wait_for_log "$OSD_SIGNAL_LOG" "Monitoring signals" 5

  # Match the shell monitor-index-to-connector lookup instead of assuming
  # the backend virtual connector name.
  monitor_eval="$(gdbus call --session --dest org.gnome.Shell \
    --object-path /org/gnome/Shell --method org.gnome.Shell.Eval \
    "global.backend.get_monitor_manager().get_logical_monitors().find(m => m.get_number() === 0).get_monitors().filter(m => m.is_active()).map(m => m.get_connector())")"
  OSD_OUTPUT_NAMES_JSON="$(printf "%s\n" "$monitor_eval" |
    sed -n "s/^(true, '\''\(.*\)'\'')$/\1/p")"
  OSD_OUTPUT_NAMES=
  if [ -n "$OSD_OUTPUT_NAMES_JSON" ] && [ "$OSD_OUTPUT_NAMES_JSON" != "[]" ]; then
    OSD_OUTPUT_NAMES="$(printf "%s\n" "$OSD_OUTPUT_NAMES_JSON" |
      python3 -c "import json,sys; names=json.load(sys.stdin); print(chr(91) + (chr(44) + chr(32)).join(chr(39) + name + chr(39) for name in names) + chr(93), end=\"\")")"
  else
    echo "  FAIL: no active output names for logical monitor 0"
    rc=1
  fi
  if [ -n "$OSD_OUTPUT_NAMES" ]; then
    echo "  active OSD outputs -> $OSD_OUTPUT_NAMES"
  fi

  show_test_osd
  sleep 1
  if grep -q OsdRequested "$OSD_SIGNAL_LOG"; then
    echo "  FAIL: native OSD was forwarded"; cat "$OSD_SIGNAL_LOG"; rc=1
  else
    echo "  ok: native OSD was not forwarded"
  fi

  callp SetFeature osd false >/dev/null
  previous_count="$(osd_signal_count)"
  show_test_osd
  if gnoblin_wait_until 5 osd_signal_records_are_valid_after "$previous_count"; then
    echo "  ok: suppressed OSD emitted one complete protocol v2 record"
  else
    echo "  FAIL: suppressed OSD handoff"; cat "$OSD_SIGNAL_LOG"; rc=1
  fi

  # The master gate stays enabled while the per-type gate suppresses volume.
  callp SetFeature osd true >/dev/null
  master_osd="$(callp GetFeature osd)"
  case "$master_osd" in
    *true*) echo "  ok: master osd enabled for osd-volume gate";;
    *) echo "  FAIL: master osd not enabled for osd-volume gate"; rc=1;;
  esac
  callp SetFeature osd-volume false >/dev/null
  volume_gate="$(callp GetFeature osd-volume)"
  case "$volume_gate" in
    *false*) echo "  ok: osd-volume disabled for forwarding check";;
    *) echo "  FAIL: osd-volume gate did not disable"; rc=1;;
  esac
  previous_count="$(osd_signal_count)"
  show_test_osd
  if gnoblin_wait_until 5 osd_signal_records_are_valid_after "$previous_count"; then
    echo "  ok: disabled osd-volume forwarded a new complete OSD record"
  else
    echo "  FAIL: osd-volume OSD handoff"; cat "$OSD_SIGNAL_LOG"; rc=1
  fi
  callp SetFeature osd-volume true >/dev/null
  kill "$OSD_SIGNAL_PID" 2>/dev/null || true

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

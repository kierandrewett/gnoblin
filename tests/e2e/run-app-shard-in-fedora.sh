#!/usr/bin/env bash
set -euo pipefail
trap 'chmod -R a+rX "$ARTIFACT_DIR" 2>/dev/null || true' EXIT

dnf -y install git flatpak gtk3 gtk4 gnome-shell wayland-devel wayland-protocols-devel \
    gcc pkgconf-pkg-config xorg-x11-server-Xwayland dbus-daemon python3-gobject \
    xdg-desktop-portal xdg-desktop-portal-gnome xdg-desktop-portal-gtk dconf hyprcursor util-linux
trace_env=()
if [[ "${TRACE_CRASH:-false}" == true ]]; then
    dnf -y install gdb
    trace_env+=(GNOBLIN_TEST_GDB_LOG_CRITICALS=1)
fi

tar -xf "$GITHUB_WORKSPACE/e2e-ci-artifacts/gnoblin-install-prefix.tar" \
    --no-same-owner -C "$GITHUB_WORKSPACE"
test -x "$GITHUB_WORKSPACE/install/bin/gnome-shell"
test -f "$GITHUB_WORKSPACE/install/share/gnome-shell/gnome-shell-dbus-interfaces.gresource"

useradd --create-home e2e
runuser -u e2e -- bwrap --unshare-all --ro-bind / / --proc /proc --dev /dev true
./tests/start-system-bus.sh

e2e_uid="$(id -u e2e)"
e2e_failure_policy="${GNOBLIN_E2E_FAILURE_POLICY:-strict}"
e2e_required_app_ids="${GNOBLIN_E2E_REQUIRED_APP_IDS:-}"
e2e_app_ids="${APP_IDS:-}"
app_selection_args=()
if [[ -n "$e2e_app_ids" ]]; then
    if [[ "$e2e_app_ids" =~ (^|,)[[:space:]]*(,|$) ]]; then
        echo "APP_IDS must be a comma-separated list of non-empty app IDs" >&2
        exit 2
    fi
    IFS=',' read -r -a requested_app_ids <<<"$e2e_app_ids"
    for app_id in "${requested_app_ids[@]}"; do
        app_id="${app_id#"${app_id%%[![:space:]]*}"}"
        app_id="${app_id%"${app_id##*[![:space:]]}"}"
        if [[ -z "$app_id" ]]; then
            echo "APP_IDS must be a comma-separated list of non-empty app IDs" >&2
            exit 2
        fi
        app_selection_args+=(--app-id "$app_id")
    done
fi
mkdir -p "$ARTIFACT_DIR"
python3 scripts/devkit_dbus.py \
    "$ARTIFACT_DIR/gnoblin-dbus-preflight" "$GITHUB_WORKSPACE"

flatpak remote-add --system --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
python3 tests/e2e/app-catalog.py \
    --catalog-in "$GITHUB_WORKSPACE/e2e-ci-artifacts/app-catalog.json" \
    --shard-index "$SHARD_INDEX" --shard-count 40 \
    "${app_selection_args[@]}" \
    --shard-output "$ARTIFACT_DIR/shard.json"

python3 tests/e2e/install-shard.py \
    "$ARTIFACT_DIR/shard.json" "$ARTIFACT_DIR/installation-report.json"

extra_monitor=""
if ((SHARD_INDEX % 2 == 0)); then
    extra_monitor="1024x768"
fi
install -d -o e2e -g e2e -m 700 "/run/user/$e2e_uid"
chown -R e2e:e2e "$ARTIFACT_DIR"
runuser -u e2e -- test -x "$GITHUB_WORKSPACE/install/bin/gnome-shell"
runuser -u e2e -- env \
    "${trace_env[@]}" \
    XDG_RUNTIME_DIR="/run/user/$e2e_uid" \
    GNOBLIN_PREFIX="$GITHUB_WORKSPACE/install" \
    GNOBLIN_E2E_CATALOG="$GITHUB_WORKSPACE/e2e-ci-artifacts/app-catalog.json" \
    GNOBLIN_E2E_PREPARED_SHARD="$ARTIFACT_DIR/shard.json" \
    GNOBLIN_E2E_SHARD_INDEX="$SHARD_INDEX" \
    GNOBLIN_E2E_SHARD_COUNT=40 \
    GNOBLIN_E2E_EXTRA_MONITOR="$extra_monitor" \
    GNOBLIN_E2E_INSTALL_REPORT="$ARTIFACT_DIR/installation-report.json" \
    GNOBLIN_E2E_ARTIFACT_DIR="$ARTIFACT_DIR" \
    GNOBLIN_E2E_FAILURE_POLICY="$e2e_failure_policy" \
    GNOBLIN_E2E_REQUIRED_APP_IDS="$e2e_required_app_ids" \
    GNOBLIN_E2E_TIMEOUT=3300 \
    python3 tests/e2e/app-e2e.py

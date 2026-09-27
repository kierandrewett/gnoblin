#!/usr/bin/env bash
set -euo pipefail

: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must be set to the private app-test runtime}"
if (($# < 4)) || [[ "$3" != -- ]]; then
    echo "usage: $0 ARTIFACT_DIR SHELL_PID_FILE -- COMMAND [ARG ...]" >&2
    exit 2
fi

artifact_dir="$1"
shell_pid_file="$2"
shift 3
mkdir -p "$artifact_dir/audio-services"
audio_log_dir="$artifact_dir/audio-services"
service_pids=()
shell_pid=

stop_process() {
    local pid="$1"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        for _ in $(seq 1 20); do
            kill -0 "$pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$pid" 2>/dev/null || true
    fi
    if [[ -n "$pid" ]]; then
        wait "$pid" 2>/dev/null || true
    fi
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM HUP
    stop_process "$shell_pid"
    for ((index = ${#service_pids[@]} - 1; index >= 0; index--)); do
        stop_process "${service_pids[index]}"
    done
    return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

for service in pipewire wireplumber pipewire-pulse; do
    command -v "$service" >/dev/null || {
        echo "virtual audio service is unavailable: $service" >&2
        exit 1
    }
done
command -v pactl >/dev/null || {
    echo "virtual audio setup requires pactl from pulseaudio-utils" >&2
    exit 1
}
wireplumber_config="${XDG_CONFIG_HOME:?XDG_CONFIG_HOME must point at private test config}/wireplumber/wireplumber.conf.d"
mkdir -p "$wireplumber_config"
cat >"$wireplumber_config/90-gnoblin-e2e.conf" <<'EOF'
wireplumber.profiles = {
  main = {
    support.dbus = disabled
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
  }
}
EOF

pipewire >"$audio_log_dir/pipewire.log" 2>&1 &
pipewire_pid=$!
service_pids+=("$pipewire_pid")
for _ in $(seq 1 100); do
    [[ -S "$XDG_RUNTIME_DIR/pipewire-0" ]] && break
    kill -0 "$pipewire_pid" 2>/dev/null || break
    sleep 0.1
done
if [[ ! -S "$XDG_RUNTIME_DIR/pipewire-0" ]]; then
    echo "private PipeWire did not create its runtime socket" >&2
    tail -n 40 "$audio_log_dir/pipewire.log" >&2
    exit 1
fi

wireplumber >"$audio_log_dir/wireplumber.log" 2>&1 &
wireplumber_pid=$!
service_pids+=("$wireplumber_pid")
pipewire-pulse >"$audio_log_dir/pipewire-pulse.log" 2>&1 &
pipewire_pulse_pid=$!
service_pids+=("$pipewire_pulse_pid")

for _ in $(seq 1 100); do
    if pactl info >/dev/null 2>&1; then break; fi
    if ! kill -0 "$wireplumber_pid" 2>/dev/null || ! kill -0 "$pipewire_pulse_pid" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
if ! pactl info >/dev/null 2>&1; then
    echo "private PipeWire PulseAudio socket did not become ready" >&2
    tail -n 40 "$audio_log_dir/pipewire.log" "$audio_log_dir/wireplumber.log" \
        "$audio_log_dir/pipewire-pulse.log" >&2
    exit 1
fi

if ! pactl load-module module-null-sink sink_name=gnoblin_e2e sink_properties=device.description=Gnoblin-E2E \
    >"$audio_log_dir/null-sink-module.txt" 2>&1; then
    cat "$audio_log_dir/null-sink-module.txt" "$audio_log_dir/wireplumber.log" >&2
    exit 1
fi
pactl list short sinks >"$audio_log_dir/sinks.txt"
if ! grep -q 'gnoblin_e2e' "$audio_log_dir/sinks.txt"; then
    echo "private PipeWire did not expose the Gnoblin E2E null sink" >&2
    cat "$audio_log_dir/sinks.txt" >&2
    exit 1
fi
pactl set-default-sink gnoblin_e2e >"$audio_log_dir/default-sink-set.log" 2>&1 || {
    cat "$audio_log_dir/default-sink-set.log" >&2
    exit 1
}

selected_sink=
stable_reads=0
for _ in $(seq 1 100); do
    selected_sink="$(pactl get-default-sink 2>/dev/null || true)"
    if [[ "$selected_sink" == gnoblin_e2e ]]; then
        stable_reads=$((stable_reads + 1))
        if ((stable_reads >= 3)); then break; fi
    else
        stable_reads=0
        if ! pactl set-default-sink gnoblin_e2e >>"$audio_log_dir/default-sink-set.log" 2>&1; then
            cat "$audio_log_dir/default-sink-set.log" >&2
            exit 1
        fi
    fi
    sleep 0.1
done
printf '%s\n' "$selected_sink" >"$audio_log_dir/default-sink.txt"
if [[ "$selected_sink" != gnoblin_e2e || "$stable_reads" -lt 3 ]]; then
    pactl info >"$audio_log_dir/pulse-info.txt" 2>&1 || true
    echo "private PipeWire did not keep gnoblin_e2e as the default sink" >&2
    cat "$audio_log_dir/default-sink.txt" "$audio_log_dir/pulse-info.txt" \
        "$audio_log_dir/sinks.txt" >&2
    exit 1
fi
pactl info >"$audio_log_dir/pulse-info.txt"
if ! grep -Fxq 'Default Sink: gnoblin_e2e' "$audio_log_dir/pulse-info.txt"; then
    echo "PulseAudio compatibility info does not report the selected E2E null sink" >&2
    cat "$audio_log_dir/default-sink.txt" "$audio_log_dir/pulse-info.txt" >&2
    exit 1
fi
echo "GNOBLIN_TEST_PIPEWIRE_READY sink=gnoblin_e2e runtime=$XDG_RUNTIME_DIR"

"$@" &
shell_pid=$!
printf '%s\n' "$shell_pid" >"$shell_pid_file"
if wait "$shell_pid"; then
    shell_pid=
    exit 0
else
    result=$?
    shell_pid=
    exit "$result"
fi

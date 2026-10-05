#!/usr/bin/env bash
# Start a persistent IBus daemon on the current private D-Bus session.

gnoblin_test_ibus_start() {
    local pid_file="$1"
    local log_file="$2"
    local pid owner

    ibus-daemon --panel disable --xim >"$log_file" 2>&1 &
    pid=$!
    printf '%s\n' "$pid" >"$pid_file"

    for _ in $(seq 1 100); do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "IBus daemon exited before owning the private session bus:" >&2
            cat "$log_file" >&2
            return 1
        fi

        owner="$(gdbus call --session \
            --dest=org.freedesktop.DBus \
            --object-path=/org/freedesktop/DBus \
            --method=org.freedesktop.DBus.NameHasOwner \
            org.freedesktop.IBus 2>/dev/null || true)"
        if [[ "$owner" == "(true,)" ]]; then
            printf 'GNOBLIN_TEST_IBUS_READY name=org.freedesktop.IBus pid=%s\n' "$pid"
            return 0
        fi
        sleep 0.1
    done

    echo "IBus daemon did not own org.freedesktop.IBus within 10 seconds:" >&2
    cat "$log_file" >&2
    return 1
}

gnoblin_test_ibus_stop() {
    local pid_file="$1"
    local pid
    pid="$(cat "$pid_file" 2>/dev/null || true)"
    [[ -n "$pid" ]] || return 0

    kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 20); do
        if ! kill -0 "$pid" 2>/dev/null; then
            break
        fi
        sleep 0.1
    done
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    rm -f "$pid_file"
}

gnoblin_test_ibus_select_source() {
    local gnoblinctl="$1"
    local result_file="$2"

    for _ in {1..100}; do
        if "$gnoblinctl" --timeout 2 input select ibus xkb:us::eng >"$result_file" 2>&1; then
            return 0
        fi
        sleep 0.1
    done

    cat "$result_file" >&2
    echo 'Gnoblin could not select the configured IBus engine' >&2
    return 1
}

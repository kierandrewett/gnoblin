#!/usr/bin/env bash
# Shared assertions and waits for Gnoblin's shell integration tests.

# Print fatal diagnostics and return success when the shell log is not clean.
gnoblin_log_has_fatal() {
    local log_file="${1:?log file required}"

    LC_ALL=C grep -E \
        'GNOME Shell-CRITICAL|JS ERROR|Traceback \(most recent call last\)|assertion .* failed|SIG(SEGV|ABRT)|Aborted \(core dumped\)' \
        "$log_file"
}

# Poll a command until it succeeds or a bounded timeout expires.
gnoblin_wait_until() {
    local timeout="${1:?timeout required}"
    shift
    if ! [[ "$timeout" =~ ^[1-9][0-9]*$ ]]; then
        echo "gnoblin_wait_until: timeout must be a positive integer" >&2
        return 2
    fi
    if [ "$#" -eq 0 ]; then
        echo "gnoblin_wait_until: command required" >&2
        return 2
    fi

    local attempts=$((timeout * 10))
    local attempt
    for ((attempt = 0; attempt < attempts; attempt++)); do
        if "$@"; then
            return 0
        fi
        sleep 0.1
    done
    "$@"
}

# Wait for an observable log event instead of assuming fixed execution time.
gnoblin_wait_for_log() {
    local log_file="${1:?log file required}"
    local pattern="${2:?pattern required}"
    local timeout="${3:-10}"

    gnoblin_wait_until "$timeout" grep -qE "$pattern" "$log_file"
}

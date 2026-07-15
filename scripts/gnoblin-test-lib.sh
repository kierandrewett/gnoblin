#!/usr/bin/env bash
# Shared assertions and waits for Gnoblin's shell integration tests.

# Print fatal diagnostics and return success when the shell log is not clean.
gnoblin_log_has_fatal() {
    local log_file="${1:?log file required}"

    LC_ALL=C grep -E \
        'GNOME Shell-CRITICAL|JS ERROR|Traceback \(most recent call last\)|assertion .* failed|SIG(SEGV|ABRT)|Aborted \(core dumped\)' \
        "$log_file"
}

#!/usr/bin/env bash

# Retry a command that talks to an external Git host. Pinned refs and command
# arguments are unchanged across attempts; persistent failures still fail the
# calling build.
gnoblin_retry_command() {
    local max_attempts=5
    local attempt delay

    for ((attempt = 1; attempt <= max_attempts; attempt++)); do
        if "$@"; then
            return 0
        fi

        if ((attempt < max_attempts)); then
            delay=$((attempt * 15))
            printf 'Command failed; retrying in %ss (attempt %s/%s):' \
                "$delay" "$attempt" "$max_attempts" >&2
            printf ' %q' "$@" >&2
            printf '\n' >&2
            sleep "$delay"
        fi
    done

    printf 'Command failed after %s attempts:' "$max_attempts" >&2
    printf ' %q' "$@" >&2
    printf '\n' >&2
    return 1
}

#!/usr/bin/env bash
# Shared secure publication for persistent Gnoblin development logs.

# Print the private state directory after creating and validating it.
gnoblin_state_dir() {
    local dir owner mode

    dir="${GNOBLIN_STATE_DIR:-${XDG_STATE_HOME:-${HOME:?HOME is not set}/.local/state}/gnoblin}"
    install -d -m 700 -- "$dir"

    owner="$(stat -c %u -- "$dir")"
    mode="$(stat -c %a -- "$dir")"
    if [ "$owner" != "$(id -u)" ] || [ "$mode" != 700 ]; then
        echo "gnoblin: refusing unsafe state directory $dir (owner=$owner mode=$mode)" >&2
        return 1
    fi

    printf '%s\n' "$dir"
}

# Atomically replace a named state log without following destination symlinks.
gnoblin_publish_log() {
    local source="${1:?source log required}"
    local name="${2:?destination name required}"
    local dir tmp

    case "$name" in
        */* | . | ..)
            echo "gnoblin: invalid state filename: $name" >&2
            return 1
            ;;
    esac

    dir="$(gnoblin_state_dir)"
    tmp="$(mktemp "$dir/.${name}.XXXXXX")"
    if ! install -m 600 -- "$source" "$tmp"; then
        rm -f -- "$tmp"
        return 1
    fi
    if ! mv -fT -- "$tmp" "$dir/$name"; then
        rm -f -- "$tmp"
        return 1
    fi
}

#!/usr/bin/env bash
# Boot-time budget for the patched shell. Boots headless in gnoblin mode and
# measures compositor process start (first libmutter message) to the shell's
# own "GNOME Shell started" mark -- the thing gnome-session, and the user,
# actually wait on -- failing if it regresses past a budget.
#
# This exists because that number is easy to lose silently. Skipping the stock
# 500 ms startup animation in gnoblin mode
# (patches/gnome-shell/51-startup-animation) is worth ~400 ms here; a rebase
# that drops or misgates that patch would give it straight back with nothing
# failing. Same idea as the memory budgets in perf-smoke.sh.
#
# Calibrated on this machine (llvmpipe, headless, best of 3):
#
#     patched   ./install   1161 ms   (runs: 1161 1350 1495)
#     unpatched /usr        1541 ms   (runs: 1541 1596 1608)
#
# Budget 1350 ms sits between them with ~190 ms of margin either side. Note the
# run-to-run spread is wide (~25%), which is why this takes the BEST of N
# rather than a mean or median: noise only ever makes a boot slower, so the
# fastest run is the most stable estimate of what the code can do.
#
# These numbers are llvmpipe and headless. They are for detecting regressions
# against themselves, not a claim about real hardware.
#
# Env: GNOBLIN_PREFIX (default ./install), BOOT_BUDGET_MS (default 1350),
#      BOOT_RUNS (default 3, best reported), SETTLE (passed through).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
source "$ROOT/scripts/gnoblin-state.sh"
GNOBLIN_STATE_DIR="$(gnoblin_state_dir)" || exit 1
export GNOBLIN_STATE_DIR

PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"
BUDGET_MS="${BOOT_BUDGET_MS:-1350}"
RUNS="${BOOT_RUNS:-3}"
LAST_LOG="$GNOBLIN_STATE_DIR/gnome-shell-last.log"

[ -x "$PREFIX/bin/gnome-shell" ] || { echo "no gnome-shell in $PREFIX — build first" >&2; exit 1; }

# "13:45:04.213" -> ms since midnight. Used for both ends of the measurement,
# so the absolute epoch does not matter, only the difference.
hhmmss_to_ms() {
    local t="$1" h m s
    IFS=: read -r h m s <<<"$t"
    # s is "SS.mmm"; strip the dot rather than use floating point in bash
    printf '%s\n' "$(( (10#$h * 3600000) + (10#$m * 60000) + (10#${s%%.*} * 1000) + 10#${s##*.} ))"
}

measure_once() {
    SETTLE="${SETTLE:-15}" GNOBLIN_PREFIX="$PREFIX" \
        "$ROOT/scripts/run-gnome-shell.sh" >/dev/null 2>&1
    local rc=$?
    [ -f "$LAST_LOG" ] || { echo "no log at $LAST_LOG" >&2; return 1; }

    # Compositor start = first libmutter message; end = the shell's own
    # "started" mark, which is what gnome-session and the user actually wait on.
    local start_t end_t
    start_t="$(grep -oE 'libmutter-Message: [0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}' "$LAST_LOG" \
        | head -1 | grep -oE '[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}')"
    end_t="$(grep -E 'GNOME Shell started' "$LAST_LOG" \
        | grep -oE '[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}' | head -1)"
    [ -n "$start_t" ] && [ -n "$end_t" ] || { echo "could not find both marks (rc=$rc)" >&2; return 1; }

    local a b d
    a="$(hhmmss_to_ms "$start_t")"
    b="$(hhmmss_to_ms "$end_t")"
    d=$(( b - a ))
    [ "$d" -lt 0 ] && d=$(( d + 86400000 ))   # clock wrapped past midnight
    printf '%s\n' "$d"
}

echo "== boot time (gnoblin mode, headless, $PREFIX) =="
times=()
for i in $(seq 1 "$RUNS"); do
    if ms="$(measure_once)"; then
        echo "   run $i: ${ms} ms"
        times+=("$ms")
    else
        echo "   run $i: FAILED to measure" >&2
    fi
done

[ "${#times[@]}" -gt 0 ] || { echo "FAIL: no successful boots" >&2; exit 1; }

# Best of N, not mean or median. Scheduler noise and llvmpipe only ever make a
# boot slower, so the fastest run is the cleanest estimate of what the code can
# do -- and it is what makes the gate stable enough to be worth having. (An
# earlier version took sorted[n/2], which for an even count silently reports the
# slower half and flagged a healthy build at 1655 ms.)
mapfile -t sorted < <(printf '%s\n' "${times[@]}" | sort -n)
best="${sorted[0]}"
echo "   best: ${best} ms of ${#times[@]}/${RUNS} runs (budget ${BUDGET_MS} ms)"

if [ "$best" -gt "$BUDGET_MS" ]; then
    cat >&2 <<EOF
FAIL: boot time ${best} ms exceeds budget ${BUDGET_MS} ms.
      The usual cause is the stock startup animation running again -- check
      patches/gnome-shell/51-startup-animation still applies and that
      _startupAnimationSession()/_prepareStartupAnimation() are gated on the
      SAME condition. See TODO.md "Performance".
EOF
    exit 1
fi
echo "PASS: boot time within budget"

#!/usr/bin/env bash
# Regression test for `just gnome-devkit`: headless-boot a nested gnoblin session
# and confirm the spawned-terminal environment can actually drive gnoblin — i.e.
# the isolated D-Bus + gnoblinctl-on-PATH plumbing that a real terminal inherits.
# (The visible nested window + interactive terminal are exercised by hand; this
# guards the env wiring, which is the part that silently rots.)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

out="$(GNOME_DEVKIT_HEADLESS=1 \
       GNOME_DEVKIT_EXEC='echo "PING:$(gnoblinctl ping)"; echo "VER:$(gnoblinctl version)"' \
       timeout 120 bash "$ROOT/scripts/run-gnome-devkit.sh" 2>&1)"

rc=0
grep -q 'org.gnoblin.Shell owned' <<<"$out" || { echo "  FAIL: nested gnoblin did not come up"; rc=1; }
grep -q 'PING:pong'                <<<"$out" || { echo "  FAIL: gnoblinctl ping did not reach org.gnoblin.Shell in the devkit env"; rc=1; }
grep -q 'VER:.*gnoblin'            <<<"$out" || { echo "  FAIL: gnoblinctl version wrong in the devkit env"; rc=1; }

if [ "$rc" = 0 ]; then
  echo ">> RESULT: PASS (devkit env drives gnoblin: ping->pong, version ok)"
else
  echo ">> RESULT: FAIL. output tail:"; tail -n 20 <<<"$out"
fi
exit "$rc"

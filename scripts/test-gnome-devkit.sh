#!/usr/bin/env bash
# Regression test for `just gnome-devkit`: headless-boot a nested gnoblin session
# and confirm the spawned-terminal environment can actually drive gnoblin — i.e.
# the isolated D-Bus + gnoblinctl-on-PATH plumbing that a real terminal inherits.
# (The visible nested window + interactive terminal are exercised by hand; this
# guards the env wiring, which is the part that silently rots.)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

probe="$ROOT/scripts/test-shell-security-policy.py"
command='echo "PING:$(gnoblinctl ping)"; echo "VER:$(gnoblinctl version)"'
command="$command; \"$probe\""

rc=0
out=
if ! out="$(GNOME_DEVKIT_HEADLESS=1 \
            GNOME_DEVKIT_EXEC="$command" \
            timeout 120 bash "$ROOT/scripts/run-gnome-devkit.sh" 2>&1)"; then
  echo "  FAIL: default devkit command exited non-zero"
  rc=1
fi
grep -q 'org.gnoblin.Shell owned' <<<"$out" || { echo "  FAIL: nested gnoblin did not come up"; rc=1; }
grep -q 'PING:pong'                <<<"$out" || { echo "  FAIL: gnoblinctl ping did not reach org.gnoblin.Shell in the devkit env"; rc=1; }
grep -q 'VER:.*gnoblin'            <<<"$out" || { echo "  FAIL: gnoblinctl version wrong in the devkit env"; rc=1; }
grep -q 'PASS: org.gnome.Shell.Eval remained restricted' <<<"$out" || {
  echo "  FAIL: default devkit unexpectedly enabled org.gnome.Shell.Eval"
  rc=1
}

unsafe_out=
if ! unsafe_out="$(GNOME_DEVKIT_HEADLESS=1 \
                   GNOME_DEVKIT_UNSAFE_MODE=1 \
                   GNOME_DEVKIT_EXEC="GNOBLIN_EXPECT_UNSAFE_MODE=1 \"$probe\"" \
                   timeout 120 bash "$ROOT/scripts/run-gnome-devkit.sh" 2>&1)"; then
  echo "  FAIL: unsafe devkit command exited non-zero"
  rc=1
fi
grep -q 'WARNING: enabling unsafe mode for this isolated devkit shell' <<<"$unsafe_out" || {
  echo "  FAIL: unsafe devkit did not report its elevated policy"
  rc=1
}
grep -q 'PASS: org.gnome.Shell.Eval was explicitly enabled' <<<"$unsafe_out" || {
  echo "  FAIL: explicit unsafe devkit did not enable org.gnome.Shell.Eval"
  rc=1
}

if [ "$rc" = 0 ]; then
  echo ">> RESULT: PASS (devkit env drives gnoblin; Eval is restricted by default and explicitly opt-in)"
else
  echo ">> RESULT: FAIL. default output tail:"; tail -n 20 <<<"$out"
  echo ">> unsafe output tail:"; tail -n 20 <<<"$unsafe_out"
fi
exit "$rc"

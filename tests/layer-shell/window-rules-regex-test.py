#!/usr/bin/env python3
# Regression test for the `~=` regex matcher in [window-rules].
#
# `class~=<pattern>` matches the window class as a case-insensitive GLib/PCRE
# regex (vs `class=` which is a substring). We use a regex that ONLY matches via
# anchors/character-classes (so a plain substring of the literal pattern text
# would not hit), and assert the rule's observable side effect (workspace move)
# fires — proving the regex compiled and matched. A second run with a
# deliberately NON-matching regex must NOT move the window.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def foot_moved_off_ws1(extra_conf):
    """Boot with `extra_conf`, map foot, return True if a workspace-2 rule moved
    it off workspace 1 (reported hidden)."""
    dk = dh.Devkit()
    dk.extra_conf = extra_conf
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if not dk.spawn_and_wait("foot"):
            print("  (foot did not map)")
            return None
        time.sleep(1.5)
        foot = next((w for w in dk.list_windows() if w[2] == "foot"), None)
        if dk.crashed():
            print(f"  (crash {dk.crashed()})")
            return None
        return bool(foot and foot[4])  # [4] = hidden/off-workspace
    finally:
        dk.teardown()


def main():
    # `^f.o[t]$` matches "foot" only as a regex (anchored, char-class) — its
    # literal text is not a substring of "foot", so this proves regex matching.
    hit = foot_moved_off_ws1("[window-rules]\nrule = class~=^f.o[t]$ | workspace 2\n")
    if hit is None:
        return 1
    if not hit:
        print("FAIL: regex `class~=^f.o[t]$` did not match foot")
        return 1
    print("PASS: `class~=` regex matcher matches (anchored pattern hit 'foot')")

    # A regex that cannot match foot must leave it on workspace 1.
    miss = foot_moved_off_ws1("[window-rules]\nrule = class~=^zzz_nomatch$ | workspace 2\n")
    if miss is None:
        return 1
    if miss:
        print("FAIL: non-matching regex still moved foot (matcher too loose)")
        return 1
    print("PASS: non-matching `class~=` regex correctly does not match")
    return 0


if __name__ == "__main__":
    sys.exit(main())

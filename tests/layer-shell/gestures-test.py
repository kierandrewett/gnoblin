#!/usr/bin/env python3
# Regression test for touchpad gestures (Phase 7).
#
# Real multitouch can't be injected headlessly, so gestures are routed through
# the action dispatcher: the live handler decodes a swipe/pinch into a key like
# "swipe-3-up" and calls gnoblin_gestures_trigger, which the `gesture` action
# also exposes. This test drives that resolution path via `dispatch gesture <key>`
# and checks three behaviours through WorkspaceState:
#   * a [gestures] override fires its mapped action,
#   * an unset key falls back to a built-in default,
#   * `= none` disables a gesture.
import sys, time, importlib.util, pathlib
from gi.repository import Gio

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def main():
    dk = dh.Devkit()
    # swipe-3-up overridden to a concrete workspace; swipe-3-down disabled
    # with `none`; swipe-3-right disabled with an empty value; swipe-3-left
    # left unset (built-in default = workspace next).
    dk.extra_conf = "[gestures]\nswipe-3-up = workspace 3\nswipe-3-down = none\nswipe-3-right =\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)

        def active():
            return dk.shell_proxy().call_sync(
                "WorkspaceState", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]

        # Override: swipe-3-up -> "workspace 3" -> active index 2.
        dk.dispatch("gesture", "swipe-3-up")
        time.sleep(1)
        a_override = active()
        print(f"  after swipe-3-up (=workspace 3): active={a_override}")
        if a_override != 2:
            print(f"FAIL: gesture override did not switch workspace (active={a_override})")
            return 1

        # Disabled: swipe-3-down = none -> no change.
        log_before_none = dk.shell_log.stat().st_size if dk.shell_log.exists() else 0
        dk.dispatch("gesture", "swipe-3-down")
        time.sleep(1)
        a_none = active()
        print(f"  after swipe-3-down (=none): active={a_none}")
        if a_none != 2:
            print(f"FAIL: disabled gesture still acted (active={a_none})")
            return 1
        none_log = dk.shell_log.read_bytes()[log_before_none:].decode(errors="replace")
        if "unknown action 'none'" in none_log:
            print("FAIL: disabled gesture was dispatched as an unknown action")
            return 1

        # Disabled: swipe-3-right = empty -> no fallback/default action.
        dk.dispatch("gesture", "swipe-3-right")
        time.sleep(1)
        a_empty = active()
        print(f"  after swipe-3-right (=empty): active={a_empty}")
        if a_empty != 2:
            print(f"FAIL: empty disabled gesture fell back to default (active={a_empty})")
            return 1

        # Default fallback: swipe-3-left unset -> "workspace next" -> index moves.
        dk.dispatch("gesture", "swipe-3-left")
        time.sleep(1)
        a_default = active()
        print(f"  after swipe-3-left (default workspace next): active={a_default}")
        if a_default == 2:
            print("FAIL: default gesture mapping did not fire")
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        print("PASS: gestures resolve via [gestures] override, default, and none")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

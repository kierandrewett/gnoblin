#!/usr/bin/env python3
# Regression test for [window-rules] + the new WM dispatcher actions.
#
# Asserts: the new actions (center/move-to-monitor/focus-monitor/swap/opacity) are
# advertised; a `app-id=foot | workspace 2` rule moves a foot window to workspace
# 2 at map time (it's off the current workspace, present on ws2); and the new
# window-acting actions dispatch on a real window without crashing the compositor.
import sys, time, importlib.util, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)
from gi.repository import Gio


def main():
    dk = dh.Devkit()
    dk.extra_conf = "[window-rules]\nrule = app-id=foot | workspace 2, size 500x300, opacity 60\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        actions = dk.shell_proxy().call_sync(
            "ListActions", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
        for a in ("center", "move-to-monitor", "focus-monitor", "swap", "opacity"):
            if a not in actions:
                print(f"FAIL: action '{a}' not advertised")
                return 1
        print("  ok  new actions advertised")

        # The rule must move foot to workspace 2 at map time: on the current
        # workspace (1) it is reported off-workspace (minimized=is_hidden=True).
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1.5)
        on_ws1 = dk.list_windows()
        foot1 = next((w for w in on_ws1 if w[2] == "foot"), None)
        if not foot1 or not foot1[4]:   # [4] = minimized/hidden
            print(f"FAIL: workspace rule did not move foot off ws1: {on_ws1}")
            return 1
        dk.dispatch("workspace", "2")
        time.sleep(1)
        foot2 = next((w for w in dk.list_windows() if w[2] == "foot"), None)
        if not foot2 or foot2[4]:
            print("FAIL: foot is not present on workspace 2")
            return 1
        print("  ok  app-id=foot|workspace 2 rule placed the window on ws2")

        # The new window-acting actions must not crash the compositor.
        for action, arg in [("opacity", "40"), ("center", ""), ("opacity", "100")]:
            dk.dispatch(action, arg)
            time.sleep(0.4)
        if dk.crashed():
            print(f"FAIL: compositor crashed dispatching new actions: {dk.crashed()}")
            return 1
        print("PASS: window rules apply + new actions dispatch cleanly")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

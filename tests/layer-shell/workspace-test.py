#!/usr/bin/env python3
# Regression test for the workspace model (Phase 5): named workspaces + the
# scratchpad.
#
#  * `[workspaces]` index->name labels are surfaced over dev.gnoblin.Shell's new
#    WorkspaceNames() method (the topbar renders these instead of bare numbers).
#  * `scratchpad-stash <name>` tags the focused window and hides it (minimized);
#    `scratchpad <name>` toggles the whole named set visible/hidden again.
import sys, time, importlib.util, pathlib, subprocess
from gi.repository import Gio

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def main():
    dk = dh.Devkit()
    dk.extra_conf = "[workspaces]\n1 = Web\n2 = Code\n3 = Chat\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)

        names = dk.shell_proxy().call_sync(
            "WorkspaceNames", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
        print(f"  WorkspaceNames: {names}")
        if names[:3] != ["Web", "Code", "Chat"]:
            print(f"FAIL: workspace names not surfaced ({names})")
            return 1

        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1)

        def foot_minimized():
            return [w[4] for w in dk.list_windows() if w[2] == "foot"]

        seq = [("baseline", None, [False])]
        dk.dispatch("scratchpad-stash", "scratch")
        time.sleep(1)
        seq.append(("after stash", foot_minimized(), [True]))
        dk.dispatch("scratchpad", "scratch")
        time.sleep(1)
        seq.append(("after show", foot_minimized(), [False]))
        dk.dispatch("scratchpad", "scratch")
        time.sleep(1)
        seq.append(("after hide", foot_minimized(), [True]))

        for label, got, want in seq[1:]:
            print(f"  {label}: minimized={got}")
            if got != want:
                print(f"FAIL: {label} expected minimized={want}, got {got}")
                return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        print("PASS: workspace names surfaced + scratchpad stash/show/hide works")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

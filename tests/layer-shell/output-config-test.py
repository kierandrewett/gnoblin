#!/usr/bin/env python3
# Regression test for the [output] -> DisplayConfig bridge (Phase 5).
#
# libmutter's public monitor API only cycles presets, so gnoblin drives mutter's
# own org.gnome.Mutter.DisplayConfig (GetCurrentState + ApplyMonitorsConfig, all
# async — a sync call to mutter from its own loop would deadlock). The harness's
# single virtual monitor advertises one mode/scale, so the testable knob is
# transform (rotation): `transform 90` must show up as transform=1 in
# GetCurrentState. Crucially, applying a config must NOT loop (mutter re-emits
# monitors-changed, so the bridge must skip an already-satisfied config).
import sys, time, importlib.util, pathlib
from gi.repository import Gio

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def main():
    dk = dh.Devkit()
    dk.extra_conf = "[output]\nMeta-0 = transform 90\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(7)  # let the async apply settle
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        dc = Gio.DBusProxy.new_sync(
            dk.conn, Gio.DBusProxyFlags.NONE, None,
            "org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig", None)
        serial, monitors, logical, props = dc.call_sync(
            "GetCurrentState", None, Gio.DBusCallFlags.NONE, -1, None).unpack()

        transforms = [lm[3] for lm in logical]
        print(f"  logical transforms: {transforms}")
        applied = dk.shell_log.read_text().count("gnoblin-output: applied")
        print(f"  apply count: {applied}")

        if 1 not in transforms:
            print(f"FAIL: transform 90 not applied (logical transforms {transforms})")
            return 1
        # Exactly-once: a re-apply loop would log this hundreds of times.
        if applied != 1:
            print(f"FAIL: expected one apply, got {applied} (reconfigure loop?)")
            return 1

        print("PASS: [output] transform applied via DisplayConfig, exactly once (no loop)")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

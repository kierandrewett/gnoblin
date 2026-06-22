#!/usr/bin/env python3
# Regression test for the [input] -> GSettings bridge (Phase 5).
#
# libmutter has no public input-settings API; gnoblin maps its `[input]` config
# onto the org.gnome.desktop GSettings mutter's input backend reads. The harness
# runs the compositor with GSETTINGS_BACKEND=memory (so nothing touches the real
# user's dconf), which mutter and gnoblin_input_apply() share in-process. The
# compositor reads the keys back after writing them and logs a summary; this test
# boots with a distinctive [input] block and asserts that summary.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

EXPECT = [
    "layout=gb+extd",       # keyboard-layout + keyboard-variant
    "repeat-interval=100",  # repeat-rate 10 -> 1000/10 ms
    "delay=350",            # repeat-delay
    "natural-scroll=1",     # natural-scroll true
    "tap-to-click=1",       # tap-to-click true
    "accel-profile=flat",   # accel-profile (enum-validated)
    "focus-mode=sloppy",    # focus-follows-mouse true -> sloppy
]


def main():
    dk = dh.Devkit()
    dk.extra_conf = (
        "[input]\n"
        "keyboard-layout = gb\n"
        "keyboard-variant = extd\n"
        "repeat-rate = 10\n"
        "repeat-delay = 350\n"
        "natural-scroll = true\n"
        "tap-to-click = true\n"
        "accel-profile = flat\n"
        "focus-follows-mouse = true\n"
    )
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        line = next((l for l in dk.shell_log.read_text().splitlines()
                     if "gnoblin-input: applied" in l), None)
        if not line:
            print("FAIL: no gnoblin-input summary in the shell log")
            return 1
        print(f"  {line.split('gnoblin-input: ')[1]}")

        missing = [e for e in EXPECT if e not in line]
        if missing:
            print(f"FAIL: applied input settings missing {missing}")
            return 1

        print("PASS: [input] config maps onto the GSettings mutter reads")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

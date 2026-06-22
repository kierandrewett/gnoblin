#!/usr/bin/env python3
# Interactive regression test: prove the full input path works end-to-end —
# RemoteDesktop keyboard injection -> compositor keybind dispatch -> `spawn`
# action -> the layer-shell client launches; and that injected keystrokes reach
# the client (Escape closes the launcher).
#
# This exercises real wiring the unit tests can't: the seeded `[bind]` config
# (Super+Space = spawn gnoblin-launcher), the compositor's keybind grab, the
# spawn action, and the harness's mutter-RemoteDesktop key injection.
#
# Asserts (non-flaky, process-based, no OCR):
#   1. baseline: no gnoblin-launcher running
#   2. after Super+Space: exactly one gnoblin-launcher spawned
#   3. after Escape: the launcher has closed again
# Plus a crash check throughout. Driven by scripts/devkit-harness.py.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location(
    "dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def launchers(dk):
    return dk.processes("gnoblin-launcher")


def main():
    dk = dh.Devkit()
    fails = []
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if dk.crashed():
            print(f"FAIL: compositor crashed on boot: {dk.crashed()}")
            return 1

        base = launchers(dk)
        if base:
            fails.append(f"baseline not clean: {base}")

        dk.send_combo("Super+Space")
        time.sleep(2.5)
        after = launchers(dk)
        if len(after) != 1:
            fails.append(f"expected 1 launcher after Super+Space, got {after}")
        else:
            print(f"  ok  Super+Space spawned the launcher: {after[0]}")

        # Type into the search box (exercises per-key injection reaching the
        # client) and record a screenshot for eyeballing.
        dk.type_text("calc")
        time.sleep(0.8)
        try:
            dk.shot("/tmp/gnoblin-keybind-test.png")
        except Exception as e:
            print(f"  (screenshot skipped: {e})")

        dk.send_combo("Escape")
        time.sleep(2.5)
        closed = launchers(dk)
        if closed:
            fails.append(f"launcher did not close on Escape: {closed}")
        else:
            print("  ok  Escape reached the launcher and closed it")

        if dk.crashed():
            fails.append(f"compositor crashed: {dk.crashed()}")

        if fails:
            print("FAIL:")
            for f in fails:
                print(f"   - {f}")
            print(dk._tail(15))
            return 1
        print("PASS: keyboard injection drives keybind -> spawn -> client, "
              "and keystrokes reach the client")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

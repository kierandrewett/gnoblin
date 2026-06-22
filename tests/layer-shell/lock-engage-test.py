#!/usr/bin/env python3
# Regression test for the compositor lock screen (security-critical).
#
# The `lock` action calls gnoblin_lock_engage: a Clutter overlay covering the
# whole output + a ClutterGrab + PAM auth. This test asserts the lock screen
# OBSCURES the entire desktop (topbar, dock, and any window) when engaged — if
# the overlay ever failed to cover the screen, secrets behind it would leak.
#
# Asserts (pixel-based + input): after `lock`, the topbar band AND a lower
# desktop point are both the dark lock overlay (uniform), a password box is
# rendered, wrong-password input keeps the lock up, D-Bus action dispatch cannot
# mutate the session behind the lock, and a second `lock` is a safe no-op
# (the_lock guard) — no compositor crash.
# PAM accept/reject is covered separately by tests/lock-pam*.c.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

SHOT = "/tmp/gnoblin-lock-engage-test.png"
RESIZE_SHOT = "/tmp/gnoblin-lock-resize-test.png"
MULTIMON_SHOT = "/tmp/gnoblin-lock-multimon-test.png"
LOCK_BACKDROP = (16, 17, 19)
LOCK_FIELD = (40, 42, 46)


def dark(c, lim=40):
    return all(v < lim for v in c)


def near(c, target, tolerance=6):
    return all(abs(c[i] - target[i]) <= tolerance for i in range(3))


def pixel_at(path, x, y):
    im = Image.open(path).convert("RGB")
    if x >= im.size[0] or y >= im.size[1]:
        raise AssertionError(f"{path} is {im.size}, cannot sample {x},{y}")
    return im.load()[x, y], im.size


def run_basic_lock_security_test():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        # Put a window up so there's something the lock must hide.
        dk.spawn_and_wait("foot")
        dk.dispatch("spawn", "foot")
        deadline = time.time() + 6
        while time.time() < deadline and len(dk.list_windows()) < 2:
            time.sleep(0.5)
        if len(dk.list_windows()) < 2:
            print(f"FAIL: expected two windows before lock, got {dk.list_windows()}")
            return 1
        dk.dispatch("lock")
        time.sleep(2)
        if dk.crashed():
            print(f"FAIL: compositor crashed engaging lock: {dk.crashed()}")
            return 1
        dk.shot(SHOT)
        px = Image.open(SHOT).convert("RGB").load()
        topbar = px[640, 17]        # was the topbar / window
        desktop = px[640, 650]      # was the desktop / window body
        pwbox = px[640, 417]        # the lock's password field
        print(f"  after lock: topbar={topbar} desktop={desktop} pwbox={pwbox}")
        # The overlay must cover BOTH the panel band and the desktop (uniform dark).
        if not (dark(topbar) and dark(desktop)):
            print("FAIL: lock overlay does not cover the whole screen (desktop leaks)")
            return 1
        # The lock UI (password box) must be present and distinct from the overlay.
        if dark(pwbox, lim=30):
            print("FAIL: lock password box not rendered")
            return 1
        # Exercise the lock's keyboard path too: a wrong password must not crash,
        # unlock, or leak the desktop behind the overlay. Click the field first:
        # real users do this, and the display-only ClutterText must not steal
        # keyboard focus away from the overlay's manual password handler.
        dk.click(640, 417)
        dk.type_text("wrong")
        dk.send_combo("Return")
        time.sleep(1)
        if dk.crashed():
            print(f"FAIL: wrong-password submit crashed: {dk.crashed()}")
            return 1
        dk.shot(SHOT)
        px = Image.open(SHOT).convert("RGB").load()
        topbar = px[640, 17]
        desktop = px[640, 650]
        if not (dark(topbar) and dark(desktop)):
            print("FAIL: wrong password dismissed or broke the lock overlay")
            return 1
        # Mutating D-Bus calls must also be blocked while locked. Keyboard
        # accelerators are suppressed separately, but dev.gnoblin.Shell.Dispatch
        # and ActivateWindow are another path into the same actions.
        before_windows = dk.list_windows()
        before_ids = sorted(w[0] for w in before_windows)
        before_focus = next((w[0] for w in before_windows if w[3]), None)
        before_workspace = dk.workspace_state()[0]

        dk.dispatch("spawn", "foot")
        dk.dispatch("workspace", "2")
        dk.dispatch("close")
        activate_target = next((w[0] for w in before_windows if w[0] != before_focus), None)
        if activate_target is not None:
            dk.shell_proxy().call_sync(
                "ActivateWindow",
                dh.GLib.Variant("(t)", (activate_target,)),
                dh.Gio.DBusCallFlags.NONE,
                -1,
                None,
            )
        time.sleep(3)

        after_windows = dk.list_windows()
        after_ids = sorted(w[0] for w in after_windows)
        after_focus = next((w[0] for w in after_windows if w[3]), None)
        after_workspace = dk.workspace_state()[0]
        if after_ids != before_ids:
            print(f"FAIL: locked D-Bus Dispatch mutated windows: {before_ids} -> {after_ids}")
            return 1
        if after_workspace != before_workspace:
            print(f"FAIL: locked D-Bus Dispatch switched workspace: {before_workspace} -> {after_workspace}")
            return 1
        if before_focus is not None and after_focus != before_focus:
            print(f"FAIL: locked ActivateWindow changed focus: {before_focus} -> {after_focus}")
            return 1
        # A second lock must not stack or crash.
        dk.dispatch("lock")
        time.sleep(1)
        if dk.crashed():
            print(f"FAIL: double-lock crashed: {dk.crashed()}")
            return 1
        print("PASS: lock overlay covers the desktop, renders the prompt, "
              "blocks locked D-Bus mutation, and double-lock is a safe no-op")
        return 0
    finally:
        dk.teardown()


def run_resize_coverage_test():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    try:
        # The lock actor must follow output geometry changes. A clientless bright
        # background makes any newly exposed unlocked pixels obvious.
        dk.extra_appearance = 'background = "#884422"\n'
        dk.boot(with_monitor=False, per_output=False)
        if not dk.add_monitor_late(800, 600):
            print("SKIP: virtual monitor never materialized")
            return 0
        time.sleep(1)

        dk.shot(RESIZE_SHOT)
        unlocked, unlocked_size = pixel_at(RESIZE_SHOT, 790, 590)
        print(f"  before resize lock: size={unlocked_size} bg={unlocked}")
        if near(unlocked, LOCK_BACKDROP):
            print("FAIL: unlocked background unexpectedly matches lock backdrop")
            return 1

        dk.dispatch("lock")
        time.sleep(1)
        if dk.crashed():
            print(f"FAIL: compositor crashed engaging lock before resize: {dk.crashed()}")
            return 1
        dk.shot(RESIZE_SHOT)
        before_edge, before_size = pixel_at(RESIZE_SHOT, 790, 590)
        print(f"  locked 800x600 edge: size={before_size} edge={before_edge}")
        if before_size != (800, 600):
            print(f"FAIL: expected initial late monitor screenshot to be 800x600, got {before_size}")
            return 1
        if not near(before_edge, LOCK_BACKDROP):
            print("FAIL: lock overlay did not cover the initial 800x600 output")
            return 1

        if not dk.resize_storm([(1280, 800)], dk._sc_node):
            print(f"FAIL: compositor crashed during locked monitor resize: {dk.crashed()}")
            return 1
        time.sleep(2)
        dk.shot(RESIZE_SHOT)
        grown_edge, grown_size = pixel_at(RESIZE_SHOT, 1180, 760)
        grown_entry, _ = pixel_at(RESIZE_SHOT, 640, 418)
        print(
            f"  locked 1280x800 new area: size={grown_size} "
            f"edge={grown_edge} entry={grown_entry}"
        )
        if grown_size != (1280, 800):
            print(f"FAIL: expected resized screenshot to be 1280x800, got {grown_size}")
            return 1
        if not near(grown_edge, LOCK_BACKDROP):
            print("FAIL: lock overlay did not cover newly exposed pixels after output resize")
            return 1
        if not near(grown_entry, LOCK_FIELD):
            print("FAIL: lock password field did not recenter after output resize")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed after locked monitor resize: {dk.crashed()}")
            return 1

        print("PASS: lock overlay keeps covering the output after monitor resize")
        return 0
    finally:
        dh.CLIENTS = old_clients
        dk.teardown()


def run_multimonitor_prompt_test():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    try:
        dk.boot(monitors=["1280x800", "1280x800"], per_output=False)
        time.sleep(2)

        dk.dispatch("lock")
        time.sleep(1)
        if dk.crashed():
            print(f"FAIL: compositor crashed engaging multimonitor lock: {dk.crashed()}")
            return 1
        dk.shot(MULTIMON_SHOT)
        primary_entry, shot_size = pixel_at(MULTIMON_SHOT, 640, 418)
        seam, _ = pixel_at(MULTIMON_SHOT, 1280, 418)
        print(
            f"  locked multimonitor: size={shot_size} "
            f"primary_entry={primary_entry} seam={seam}"
        )
        if shot_size != (2560, 800):
            print(f"FAIL: expected two 1280x800 monitors, got screenshot {shot_size}")
            return 1
        if not near(primary_entry, LOCK_FIELD):
            print("FAIL: lock password field is not centered on the primary monitor")
            return 1
        if near(seam, LOCK_FIELD):
            print("FAIL: lock password field straddles the monitor seam")
            return 1

        print("PASS: lock prompt stays on the primary monitor in a side-by-side layout")
        return 0
    finally:
        dh.CLIENTS = old_clients
        dk.teardown()


def main():
    rc = run_basic_lock_security_test()
    if rc != 0:
        return rc
    rc = run_resize_coverage_test()
    if rc != 0:
        return rc
    return run_multimonitor_prompt_test()


if __name__ == "__main__":
    sys.exit(main())

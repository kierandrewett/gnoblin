#!/usr/bin/env python3
# Regression test for wl_region lifetime in the shared Slint layer-shell runtime.
#
# `wl_surface.set_input_region` has copy semantics, so the temporary wl_region
# object should be destroyed immediately after the request/commit. notifyd is a
# good real client for this because its input region changes as toast cards
# appear, stack, and expire.
import importlib.util, os, pathlib, re, shutil, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

if shutil.which("notify-send") is None:
    print("SKIP: no notify-send")
    sys.exit(0)

REGION_CREATE = re.compile(r"create_region\([^\n]*wl_region[@#](\d+)\)")
REGION_DESTROY = re.compile(r"wl_region[@#](\d+)\.destroy\(\)")
FRAME_REQUEST = re.compile(r"wl_surface[@#]\d+\.frame\([^\n]*wl_callback[@#](\d+)\)")
FRAME_DONE = re.compile(r"wl_callback[@#](\d+)\.done\(")


def notify(env, title, body):
    for _ in range(20):
        r = subprocess.run(
            ["notify-send", "-t", "1000", title, body],
            env=env,
            capture_output=True,
            text=True,
        )
        if r.returncode == 0:
            return True
        time.sleep(0.25)
    print(f"FAIL: notify-send failed: {r.stderr.strip()}")
    return False


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    runtime = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    files = [
        runtime / "gnoblin-notif-center",
        runtime / "gnoblin-notif-pending",
        runtime / "gnoblin-notif-summary",
        runtime / "gnoblin-notif-history",
    ]
    try:
        for path in files:
            path.unlink(missing_ok=True)
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["WAYLAND_DEBUG"] = "1"
        log_path = dk.tmp / "notifyd-wayland.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-notifyd")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        if not notify(env, "Region lifetime", "first card"):
            return 1
        time.sleep(0.8)
        if not notify(env, "Region lifetime", "second card"):
            return 1
        time.sleep(1.8)
        if not notify(env, "Region lifetime", "third card"):
            return 1
        time.sleep(1.4)

        if proc.poll() is not None:
            print(f"FAIL: gnoblin-notifyd exited early rc={proc.returncode}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        proc.terminate()
        try:
            proc.wait(timeout=2)
        except Exception:
            proc.kill()
        proc = None
        logf.close()
        logf = None
        shutil.copy(log_path, "/tmp/gnoblin-region-lifetime-wayland.log")
        text = log_path.read_text(errors="replace")
        created = REGION_CREATE.findall(text)
        destroyed = REGION_DESTROY.findall(text)
        frame_requests = FRAME_REQUEST.findall(text)
        frame_dones = FRAME_DONE.findall(text)
        created_set = set(created)
        destroyed_set = set(destroyed)
        missing = sorted(created_set - destroyed_set, key=int)
        print(f"  wl_region creates={len(created)} destroys={len(destroyed)}")
        print(f"  frame callbacks requested={len(frame_requests)} done={len(frame_dones)}")
        if len(created) < 2:
            print("FAIL: notifyd did not exercise changing input regions")
            return 1
        if missing:
            print(f"FAIL: wl_region objects were not destroyed: {missing[:8]}")
            return 1
        if len(frame_requests) < 2 or len(frame_dones) < 1:
            print("FAIL: Slint layer-shell client did not request/receive frame callbacks")
            return 1

        print("PASS: Slint layer-shell input regions and frame callbacks are healthy")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
        for path in files:
            path.unlink(missing_ok=True)
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())

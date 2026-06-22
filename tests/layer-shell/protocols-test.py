#!/usr/bin/env python3
# Regression test: the compositor advertises gnoblin's full Wayland protocol set.
#
# gnoblin adds wlr-/ext- protocols to mutter via overlays + patches. A patch that
# silently dropped a protocol's registration (e.g. a meson.build or init-call
# regression) would break the whole tool ecosystem for that protocol with no
# other symptom. This boots gnoblin-shell and asserts every expected global
# interface is in the registry.
#
# The wl_globals probe binary is built by run-protocols.sh and its path passed in
# GNOBLIN_WL_GLOBALS.
import os, sys, time, importlib.util, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

PROBE = os.environ.get("GNOBLIN_WL_GLOBALS", "/tmp/gnoblin-wl-globals")

# Wire-interface name for each gnoblin protocol (the [protocols] config keys).
EXPECTED = {
    "zwlr_layer_shell_v1":              "wlr-layer-shell",
    "zwlr_screencopy_manager_v1":       "wlr-screencopy",
    "ext_idle_notifier_v1":             "ext-idle-notify",
    "ext_foreign_toplevel_list_v1":     "ext-foreign-toplevel-list",
    "zwlr_foreign_toplevel_manager_v1": "wlr-foreign-toplevel-management",
    "zwlr_gamma_control_manager_v1":    "wlr-gamma-control",
    "zwlr_output_power_manager_v1":     "wlr-output-power-management",
    "ext_data_control_manager_v1":      "ext-data-control",
    "org_kde_kwin_appmenu_manager":     "kde-appmenu",
}


def main():
    if not pathlib.Path(PROBE).exists():
        print(f"SKIP: no wl-globals probe at {PROBE}")
        return 0
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(4)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        r = subprocess.run([PROBE], env=env, capture_output=True, text=True, timeout=15)
        if r.returncode != 0:
            print(f"FAIL: globals probe failed: {r.stderr.strip()}")
            return 1
        advertised = {line.split()[0] for line in r.stdout.splitlines() if line.split()}
        missing = [f"{iface} ({EXPECTED[iface]})"
                   for iface in EXPECTED if iface not in advertised]
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if missing:
            print("FAIL: protocols not advertised: " + ", ".join(missing))
            return 1
        print(f"PASS: all {len(EXPECTED)} gnoblin protocols advertised")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())

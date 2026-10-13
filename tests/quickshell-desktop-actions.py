#!/usr/bin/env python3
"""Check actual Quickshell DesktopAction identities with no display or session bus.

Exit 0 means the supplied runtime preserves action IDs. Exit 1 means the
runtime failed the probe; inspect result.json and runtime.log for the reason.
No action is executed. Pass a fresh artifact directory and the exact binary
or launcher used by the shell, not a QML mock or a version string.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


EXPECTED = {
    "proof": "Open normal-priority proof",
    "x": "One character",
    "new-window": "New window",
    "--private-mode": "Private window",
    "aproof": "Duplicate label",
    "bproof": "Duplicate label",
}
ENTRY_ID = "org.gnoblin.ActionIdentity"
MARKER = "GNOBLIN_ACTION_IDENTITY="


def probe(binary, directory):
    with tempfile.TemporaryDirectory(prefix="gnoblin-action-identity-") as temporary:
        root = Path(temporary)
        for name in ("home", "data/applications", "empty", "config", "cache", "runtime"):
            (root / name).mkdir(parents=True, mode=0o700)
        desktop = (
            "[Desktop Entry]\nType=Application\nName=Action identity fixture\n"
            "Exec=/bin/false\nActions=" + ";".join(EXPECTED) + ";\n"
        )
        for action_id, label in EXPECTED.items():
            desktop += f"\n[Desktop Action {action_id}]\nName={label}\nExec=/bin/false\n"
        (root / "data/applications" / f"{ENTRY_ID}.desktop").write_text(desktop)
        (directory / "fixture.desktop").write_text(desktop)
        shutil.copyfile(Path(__file__).with_suffix(".qml"), root / "shell.qml")
        environment = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(root / "home"),
            "LANG": "C.UTF-8",
            "XDG_DATA_HOME": str(root / "data"),
            "XDG_DATA_DIRS": str(root / "empty"),
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_CONFIG_DIRS": str(root / "empty"),
            "XDG_CACHE_HOME": str(root / "cache"),
            "XDG_RUNTIME_DIR": str(root / "runtime"),
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path={root}/no-session-bus",
            "DBUS_SYSTEM_BUS_ADDRESS": f"unix:path={root}/no-system-bus",
            "QT_QPA_PLATFORM": "offscreen",
            "QT_QUICK_BACKEND": "software",
            "QSG_RHI_BACKEND": "software",
        }
        command = [binary, "--no-color", "--path", str(root / "shell.qml")]
        try:
            run = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=20)
        except subprocess.TimeoutExpired as error:
            log = (error.stdout or b"") + (error.stderr or b"")
            (directory / "runtime.log").write_bytes(log)
            return {"passed": False, "error": "runtime timed out before reporting action identities"}
        log = run.stdout + run.stderr
        (directory / "runtime.log").write_text(log)
        observations = [json.loads(line.split(MARKER, 1)[1]) for line in log.splitlines() if MARKER in line]
        if len(observations) != 1:
            return {"passed": False, "returncode": run.returncode, "error": "expected one QML observation"}
        observed = observations[0]
        expected = {"id": ENTRY_ID, "actions": [{"id": key, "name": value} for key, value in EXPECTED.items()]}
        for item in (observed, expected):
            item["actions"].sort(key=lambda action: action["id"])
        return {
            "passed": run.returncode == 0 and observed == expected,
            "returncode": run.returncode,
            "expected": expected,
            "observed": observed,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("--quickshell", default="quickshell", help="exact runtime executable or launcher")
    args = parser.parse_args()
    binary = shutil.which(args.quickshell)
    if binary is None:
        parser.error(f"runtime not found: {args.quickshell}")
    directory = args.artifacts.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    result = probe(str(Path(binary).absolute()), directory)
    result["runtime"] = binary
    (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

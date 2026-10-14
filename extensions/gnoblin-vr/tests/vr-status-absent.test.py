#!/usr/bin/env python3
"""Exercise the client status writer with a loader manifest that selects no runtime."""

import configparser
import os
from pathlib import Path
import subprocess
import sys
import tempfile


binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="gnoblin-vr-absent-") as runtime_dir:
    env = os.environ.copy()
    env["XDG_RUNTIME_DIR"] = runtime_dir
    env["XR_RUNTIME_JSON"] = str(Path(runtime_dir) / "missing-runtime.json")
    result = subprocess.run([binary], env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
    status_path = Path(runtime_dir) / "gnoblin-vr" / "status.ini"
    assert status_path.is_file(), result.stderr
    status = configparser.ConfigParser()
    status.read(status_path)
    assert status.get("vr", "state") == "absent", status_path.read_text()
    assert status.get("vr", "diagnostic"), status_path.read_text()
    assert "gnoblin-vr:" in result.stderr, (result.stdout, result.stderr)

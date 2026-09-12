#!/usr/bin/env python3
"""Verify that a Gnoblin shell loaded the user's RustDesk policy."""

import json
import subprocess

from pathlib import Path

root = Path(__file__).resolve().parents[1]
ctl = root / "src/tools/gnoblinctl"


def call(*args):
    result = subprocess.run([str(ctl), "--json", *args], capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


policy = call("permissions", "list")
rustdesk = next(rule for rule in policy["policy"]["rules"] if rule["name"] == "rustdesk")
assert rustdesk["level"] == "allow", rustdesk
assert set(rustdesk["capabilities"]) == {"screen-cast", "remote-desktop", "input-capture", "screenshot", "access"}, (
    rustdesk
)

identity = "app-id:com.rustdesk.RustDesk"
for capability in rustdesk["capabilities"]:
    decision = call("permissions", "check", capability, identity)
    assert decision["level"] == "allow", (capability, decision)
    assert decision["rule"] == "rustdesk", (capability, decision)

remote = call("permissions", "check", "remote-desktop", identity)
assert remote["devices"] == 7 and remote["clipboard"] is True, remote
capture = call("permissions", "check", "screen-cast", identity)
assert capture["monitors"] == ["primary"], capture
print("PASS: user config loaded RustDesk allow policy for all portal capabilities")

#!/usr/bin/env python3
"""Check that the running session publishes every output as a complete `gnoblin.output` object.

The graph refuses an object that lacks a declared property, so an output that is
missing from `output list` means the publisher left one out. Run inside a supervised
Gnoblin session.
"""

import json
import os
import subprocess
import time

assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"

PROPERTIES = {
    "name": str,
    "make": str,
    "model": str,
    "modes": list,
    "mode": str,
    "scale": float,
    "transform": int,
    "primary": bool,
    "enabled": bool,
    "x": int,
    "y": int,
    "width": int,
    "height": int,
    "layout": str,
}


def call(*arguments):
    return json.loads(subprocess.check_output(["/usr/bin/gnoblinctl", "--json", *arguments], text=True))


def outputs():
    return call("output", "list")["objects"]


deadline = time.monotonic() + 10
while not outputs() and time.monotonic() < deadline:
    time.sleep(0.1)

listed = outputs()
assert listed, "`output list` is empty: no output object reached the graph"

for item in listed:
    assert item["type"] == "gnoblin.output", item
    properties = item["properties"]
    assert set(properties) == set(PROPERTIES), (
        f"output {item['id']} has {sorted(properties)}, the schema declares {sorted(PROPERTIES)}"
    )
    for name, kind in PROPERTIES.items():
        value = properties[name]
        if kind is float:
            assert isinstance(value, (int, float)) and not isinstance(value, bool), (name, value)
        else:
            assert isinstance(value, kind) and not (kind is int and isinstance(value, bool)), (name, value)
    assert properties["name"], "an output must carry its connector name"
    assert properties["mode"] in properties["modes"], (
        f"mode {properties['mode']!r} is not one of {properties['modes']!r}"
    )
    assert properties["width"] > 0 and properties["height"] > 0, properties

print(f"ok: {len(listed)} output object(s) with all {len(PROPERTIES)} declared properties")

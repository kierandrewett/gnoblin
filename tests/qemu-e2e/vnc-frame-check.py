#!/usr/bin/env python3
"""Host: grab one frame over VNC and report whether anything is painted.

Usage: vnc-frame-check.py RUN_DIRECTORY
Prints "distinct-colours=N" and exits 0 when the frame has more than one colour, 1 when it is one flat colour
(for example all black). QMP screendump fails under virtio-vga-gl, so this uses the same VNC reader as
dock-first-paint.py.
"""
import importlib.util
from pathlib import Path
import sys

here = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dock_first_paint", here / "dock-first-paint.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

vnc = module.Vnc(int((Path(sys.argv[1]) / "vnc-port").read_text()))
try:
    colours = len(set(vnc.frame().convert("RGB").getdata()))
finally:
    vnc.close()
print(f"distinct-colours={colours}")
sys.exit(0 if colours > 1 else 1)

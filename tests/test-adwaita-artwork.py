#!/usr/bin/env python3
"""Compare cropped cursor vectors with the untouched Adwaita artwork."""

import io
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET
from PIL import Image, ImageChops, ImageStat

root = Path(__file__).resolve().parent.parent
source = root / "src/cursor/adwaita"
theme = root / "build/Adwaita-Hyprcursor/source/hyprcursors"
svg = "{http://www.w3.org/2000/svg}"
sheet = ET.parse(source / "adwaita.svg").getroot()
slices = {
    e.get("id"): " ".join(e.get(k) for k in ("x", "y", "width", "height"))
    for layer in sheet
    if layer.get("id") == "layer2"
    for e in layer
    if e.tag == svg + "rect"
}
for layer in sheet:
    if layer.tag == svg + "g" and layer.get("id") not in ("layer5", "layer1", "g12244"):
        layer.set("style", "display:none")
sheet.set("width", "24")
sheet.set("height", "24")
checked = 0
worst_error = 0


def render(data, size):
    png = subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size)], input=data, capture_output=True, check=True
    ).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA")


for name, info in json.loads((source / "cursors.json").read_text()).items():
    for frame in range(1, info["frames"] + 1):
        key = f"{name}_{frame:04}" if info["frames"] > 1 else name
        sheet.set("viewBox", slices[key])
        full = ET.tostring(sheet)
        cropped = (theme / name / f"{frame:04}.svg").read_bytes()
        for size in [24, 96] if frame == 1 else [24]:
            expected, actual = render(full, size), render(cropped, size)
            # Cropping changes librsvg's blur-buffer alignment slightly. Compare
            # composited pixels so invisible RGB values do not inflate error.
            for background in ("#ffffff", "#353535"):
                a = Image.new("RGBA", expected.size, background)
                b = a.copy()
                a.alpha_composite(expected)
                b.alpha_composite(actual)
                error = max(ImageStat.Stat(ImageChops.difference(a, b)).mean[:3])
                worst_error = max(worst_error, error)
                assert error < 1, (key, size, background, error)
            checked += 1
print(f"PASS: {checked} renders match the original artwork; worst mean channel error {worst_error:.3f}/255")

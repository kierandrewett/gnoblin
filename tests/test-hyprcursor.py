#!/usr/bin/env python3
"""Generate a vector cursor theme and validate its rendered frames."""

import os
from pathlib import Path
import shutil
import subprocess

repo = Path(__file__).resolve().parent.parent
work = Path(os.environ["XDG_CONFIG_HOME"]) / "hyprcursor-test"
shape = work / "source/hyprcursors/arrow"
shape.mkdir(parents=True)
(work / "source/manifest.hl").write_text(
    "name = GnoblinVectorTest\ndescription = Test fixture\nversion = 1\ncursors_directory = hyprcursors\n"
)
(shape / "meta.hl").write_text(
    "resize_algorithm = bilinear\nhotspot_x = 0.25\nhotspot_y = 0.25\ndefine_override = default\ndefine_override = wait\ndefine_size = 24, first.svg, 35\ndefine_size = 24, second.svg, 70\n"
)
for filename, colour in [("first.svg", "#ff00ff"), ("second.svg", "#00ffff")]:
    (shape / filename).write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><rect width="24" height="24" fill="{colour}"/></svg>'
    )
subprocess.run(["hyprcursor-util", "--create", str(work / "source"), "--output", str(work)], check=True)
icons = Path.home() / ".icons"
icons.mkdir(exist_ok=True)
compiled = next(work.glob("theme_*"))
shutil.copytree(compiled, icons / "GnoblinVectorTest")
subprocess.run([str(repo / "build/test-hyprcursor")], check=True)

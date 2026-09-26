#!/usr/bin/env python3
"""Package the original Adwaita vectors, animation and aliases for Hyprcursor."""

import argparse
import copy
import ctypes
import ctypes.util
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src/cursor/adwaita"
SVG = "{http://www.w3.org/2000/svg}"
XLINK = "{http://www.w3.org/1999/xlink}"
THEME = "Adwaita-Hyprcursor"
ART_LAYERS = ("layer5", "layer1", "g12244")
ET.register_namespace("", SVG[1:-1])
ET.register_namespace("xlink", XLINK[1:-1])


def overlaps(a, b):
    x, y, w, h = a
    u, v, s, t = b
    return x < u + s and x + w > u and y < v + t and y + h > v


def crop_sheet(sheet, rectangle, bounds):
    """Keep intersecting drawing nodes and their transitive SVG references."""
    ids = {e.get("id"): e for e in sheet.iter() if e.get("id")}

    def crop(node):
        box = bounds.get(node.get("id"))
        # Empty or unresolved legacy objects have no rendered bounds.
        if box is None and node.tag != SVG + "g":
            return None
        if box is not None and not overlaps(box, rectangle):
            return None
        result = copy.deepcopy(node)
        if node.tag == SVG + "g":
            result[:] = [item for child in node if (item := crop(child)) is not None]
            if not len(result):
                return None
        return result

    result = ET.Element(
        SVG + "svg", {"width": "24", "height": "24", "viewBox": " ".join(f"{v:g}" for v in rectangle), "version": "1.1"}
    )
    definitions = ET.SubElement(result, SVG + "defs")
    for layer in sheet:
        if layer.get("id") in ART_LAYERS and (part := crop(layer)) is not None:
            result.append(part)
    available = {e.get("id") for e in result.iter()}
    queue = list(result.iter())
    while queue:
        element = queue.pop()
        references = re.findall(r"url\(#([^)]*)\)", " ".join(element.attrib.values()))
        href = element.get(XLINK + "href", element.get("href", ""))
        if href.startswith("#"):
            references.append(href[1:])
        for reference in references:
            if reference in available:
                continue
            if reference not in ids:
                raise ValueError(f"Missing SVG reference: {reference}")
            dependency = copy.deepcopy(ids[reference])
            definitions.append(dependency)
            available.update(e.get("id") for e in dependency.iter())
            queue.extend(dependency.iter())
    # Strip editor metadata and private export paths; preserve SVG geometry.
    for element in result.iter():
        for key in list(element.attrib):
            if key.startswith("{") and not key.startswith(XLINK):
                del element.attrib[key]
    return result


class RsvgRectangle(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double), ("width", ctypes.c_double), ("height", ctypes.c_double)]


def query_bounds(svg):
    """Return rendered element bounds using the system librsvg API."""
    library = ctypes.util.find_library("rsvg-2")
    if library is None:
        raise RuntimeError("librsvg is required to build the Adwaita-Hyprcursor theme")

    rsvg = ctypes.CDLL(library)
    gobject = ctypes.CDLL(ctypes.util.find_library("gobject-2.0"))
    glib = ctypes.CDLL(ctypes.util.find_library("glib-2.0"))
    error = ctypes.c_void_p()
    rsvg.rsvg_handle_new_from_file.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    rsvg.rsvg_handle_new_from_file.restype = ctypes.c_void_p
    rsvg.rsvg_handle_get_geometry_for_element.argtypes = [
        ctypes.c_void_p,
        ctypes.c_char_p,
        ctypes.POINTER(RsvgRectangle),
        ctypes.POINTER(RsvgRectangle),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    rsvg.rsvg_handle_get_geometry_for_element.restype = ctypes.c_int
    gobject.g_object_unref.argtypes = [ctypes.c_void_p]
    glib.g_error_free.argtypes = [ctypes.c_void_p]

    handle = rsvg.rsvg_handle_new_from_file(str(svg).encode(), ctypes.byref(error))
    if not handle:
        if error:
            glib.g_error_free(error)
        raise RuntimeError(f"librsvg could not load {svg}")

    bounds = {}
    try:
        root = ET.parse(svg).getroot()
        for element in root.iter():
            identifier = element.get("id")
            if not identifier:
                continue
            ink = RsvgRectangle()
            logical = RsvgRectangle()
            error = ctypes.c_void_p()
            found = rsvg.rsvg_handle_get_geometry_for_element(
                handle,
                f"#{identifier}".encode(),
                ctypes.byref(ink),
                ctypes.byref(logical),
                ctypes.byref(error),
            )
            if found:
                bounds[identifier] = (ink.x, ink.y, ink.width, ink.height)
            elif error:
                glib.g_error_free(error)
    finally:
        gobject.g_object_unref(handle)
    return bounds


def build(output, fallback):
    if output.exists():
        raise FileExistsError(f"Output exists: {output}; choose a new --output directory")
    svg = SOURCE / "adwaita.svg"
    assert (
        hashlib.sha256(svg.read_bytes()).hexdigest()
        == "3da7cbdaacb51ba90e6b7f903792f2ca58ca0ac3e01ba69c0cac248e397ba500"
    )
    metadata = json.loads((SOURCE / "cursors.json").read_text())
    sheet = ET.parse(svg).getroot()
    slices = {
        e.get("id"): tuple(float(e.get(k)) for k in ("x", "y", "width", "height"))
        for layer in sheet
        if layer.get("id") == "layer2"
        for e in layer
        if e.tag == SVG + "rect"
    }
    for layer in list(sheet):
        if layer.tag == SVG + "g" and layer.get("id") not in ART_LAYERS:
            layer.set("style", "display:none")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="adwaita-vector-", dir=output.parent) as temporary:
        work = Path(temporary)
        clean = work / "sheet.svg"
        ET.ElementTree(sheet).write(clean)
        bounds = query_bounds(clean)
        source = work / "source"
        source.mkdir()
        (source / "manifest.hl").write_text(
            f"name = {THEME}\ndescription = GNOME Adwaita 49 vector cursors, packaged by Gnoblin\nversion = 49.0\ncursors_directory = hyprcursors\n"
        )
        for name, info in metadata.items():
            shape = source / "hyprcursors" / name
            shape.mkdir(parents=True)
            lines = [
                "resize_algorithm = bilinear",
                f"hotspot_x = {info['hotspot'][0] / 24:.12g}",
                f"hotspot_y = {info['hotspot'][1] / 24:.12g}",
            ]
            lines.extend(f"define_override = {alias}" for alias in info["aliases"])
            for frame in range(1, info["frames"] + 1):
                key = f"{name}_{frame:04}" if info["frames"] > 1 else name
                rectangle = slices[key]
                assert rectangle[2:] == (24, 24), (key, rectangle)
                vector = crop_sheet(sheet, rectangle, bounds)
                assert not any(e.tag == SVG + "image" for e in vector.iter()), "Raster image in vector cursor"
                ET.ElementTree(vector).write(shape / f"{frame:04}.svg", encoding="utf-8", xml_declaration=True)
                lines.append(f"define_size = 24, {frame:04}.svg, {max(1, info['duration'])}")
            (shape / "meta.hl").write_text("\n".join(lines) + "\n")
        subprocess.run(
            ["hyprcursor-util", "--create", str(source), "--output", str(work)], check=True, stdout=subprocess.DEVNULL
        )
        compiled = work / f"theme_{THEME}"
        # Keep exact Xcursor assets for toolkits that supply their own buffers.
        shutil.copytree(fallback / "cursors", compiled / "cursors", symlinks=True)
        (compiled / "index.theme").write_text(
            "[Icon Theme]\nName=Adwaita (Vector)\nComment=GNOME Adwaita with Hyprcursor SVG support\nInherits=Adwaita\n"
        )
        for name in ("COPYING", "COPYING_CCBYSA3", "COPYING_LGPL", "README.md"):
            shutil.copy2(SOURCE / name, compiled / name)
        shutil.copy2(SOURCE / "cursors.json", compiled / "cursors.json")
        shutil.copytree(source, compiled / "source")
        compiled.rename(output)
    print(f"Built {len(metadata)} vector shapes and {sum(x['frames'] for x in metadata.values())} frames: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "build/Adwaita-Hyprcursor")
    parser.add_argument("--fallback", type=Path, default=Path("/usr/share/icons/Adwaita"))
    args = parser.parse_args()
    build(args.output.resolve(), args.fallback.resolve())

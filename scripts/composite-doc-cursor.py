#!/usr/bin/env python3
"""Add the live host cursor to a nested devkit screenshot.

Mutter's devkit screenshot backend omits the host pointer. XFixes supplies the
actual XWayland cursor image and position so the captured scene keeps its real
pointer without including any of the host desktop around the devkit window.
"""

import ctypes
import os
from pathlib import Path
import sys

from PIL import Image


class CursorImage(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_short),
        ("y", ctypes.c_short),
        ("width", ctypes.c_ushort),
        ("height", ctypes.c_ushort),
        ("xhot", ctypes.c_ushort),
        ("yhot", ctypes.c_ushort),
        ("serial", ctypes.c_ulong),
        ("pixels", ctypes.POINTER(ctypes.c_ulong)),
        ("atom", ctypes.c_ulong),
        ("name", ctypes.c_char_p),
    ]


def main() -> int:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} SCREENSHOT.png", file=sys.stderr)
        return 2

    output = Path(sys.argv[1])
    display = ctypes.CDLL("libX11.so.6")
    fixes = ctypes.CDLL("libXfixes.so.3")
    display.XOpenDisplay.restype = ctypes.c_void_p
    display.XOpenDisplay.argtypes = [ctypes.c_char_p]
    display.XDefaultScreen.argtypes = [ctypes.c_void_p]
    display.XDefaultScreen.restype = ctypes.c_int
    display.XDisplayWidth.argtypes = [ctypes.c_void_p, ctypes.c_int]
    display.XDisplayWidth.restype = ctypes.c_int
    display.XDisplayHeight.argtypes = [ctypes.c_void_p, ctypes.c_int]
    display.XDisplayHeight.restype = ctypes.c_int
    display.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display.XFree.argtypes = [ctypes.c_void_p]
    fixes.XFixesGetCursorImage.restype = ctypes.POINTER(CursorImage)
    fixes.XFixesGetCursorImage.argtypes = [ctypes.c_void_p]

    connection = display.XOpenDisplay(None)
    if not connection:
        raise RuntimeError("Could not read the live pointer from the host XWayland display")
    image_ptr = fixes.XFixesGetCursorImage(connection)
    if not image_ptr:
        display.XCloseDisplay(connection)
        raise RuntimeError("XFixes did not return the live pointer image")

    cursor = image_ptr.contents
    rgba = []
    for index in range(cursor.width * cursor.height):
        pixel = cursor.pixels[index] & 0xFFFFFFFF
        rgba.append(((pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF, (pixel >> 24) & 0xFF))
    pointer = Image.new("RGBA", (cursor.width, cursor.height))
    pointer.putdata(rgba)

    with Image.open(output) as scene:
        scene = scene.convert("RGBA")
        screen = display.XDefaultScreen(connection)
        screen_width = display.XDisplayWidth(connection, screen)
        screen_height = display.XDisplayHeight(connection, screen)
        # The devkit canvas is centered in Mutter's viewer; its title bar moves
        # the canvas center ten pixels above the host screen center.
        origin_x = int(os.environ.get("GNOBLIN_DOC_VIEWPORT_X") or (screen_width - scene.width) // 2)
        origin_y = int(os.environ.get("GNOBLIN_DOC_VIEWPORT_Y") or (screen_height - scene.height) // 2 - 10)
        position = (cursor.x - cursor.xhot - origin_x, cursor.y - cursor.yhot - origin_y)
        if position[0] < 0 or position[1] < 0 or position[0] >= scene.width or position[1] >= scene.height:
            raise RuntimeError(
                f"Pointer at {cursor.x},{cursor.y} is outside the devkit viewport; "
                "set GNOBLIN_DOC_POINTER and GNOBLIN_DOC_VIEWPORT_X/Y"
            )
        scene.alpha_composite(pointer, position)
        scene.convert("RGB").save(output)

    display.XFree(image_ptr)
    display.XCloseDisplay(connection)
    print(f"Included live host cursor at {position[0]},{position[1]}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"capture-doc-cursor: {error}", file=sys.stderr)
        raise SystemExit(1)

#!/usr/bin/env python3
"""Read a PNG (what QEMU's screendump and grim write) and say what colours it holds, with the standard library only.

    pixels.py PNG size                  WIDTH HEIGHT
    pixels.py PNG share RRGGBB          the share of pixels that are exactly that colour, as a fraction of the picture
    pixels.py PNG colours               how many distinct colours the picture holds
    pixels.py PNG at X Y                the colour at a pixel, as RRGGBB

A picture the compositor drew is exact: a window of one colour is that colour at every pixel, so a share is a count of
what was drawn and not a guess at a likeness. Only 8-bit RGB and RGBA pictures without interlacing are read, which is
what both writers make.
"""

import struct
import sys
import zlib


def read(path: str) -> tuple[int, int, list[bytes]]:
    """The picture's width, height and rows of RGB triples."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"[pixels] {path} is not a PNG")
    at, header, joined = 8, None, b""
    while at < len(data):
        length, kind = struct.unpack(">I4s", data[at : at + 8])
        body = data[at + 8 : at + 8 + length]
        at += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            joined += body
    if header is None:
        raise SystemExit(f"[pixels] {path} has no header")
    width, height, depth, colour, _, _, interlace = header
    if depth != 8 or colour not in (2, 6) or interlace != 0:
        raise SystemExit(f"[pixels] {path}: only 8-bit RGB or RGBA without interlacing is read")
    step = 3 if colour == 2 else 4
    stride = width * step
    raw = zlib.decompress(joined)
    rows: list[bytes] = []
    previous = bytearray(stride)
    at = 0
    for _ in range(height):
        filter_type = raw[at]
        line = bytearray(raw[at + 1 : at + 1 + stride])
        at += 1 + stride
        for i in range(stride):
            left = line[i - step] if i >= step else 0
            up = previous[i]
            upper_left = previous[i - step] if i >= step else 0
            if filter_type == 1:
                line[i] = (line[i] + left) & 255
            elif filter_type == 2:
                line[i] = (line[i] + up) & 255
            elif filter_type == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif filter_type == 4:
                estimate = left + up - upper_left
                near = min(
                    (abs(estimate - left), left), (abs(estimate - up), up), (abs(estimate - upper_left), upper_left)
                )[1]
                line[i] = (line[i] + near) & 255
            elif filter_type != 0:
                raise SystemExit(f"[pixels] {path}: unknown filter {filter_type}")
        previous = line
        if step == 4:
            rows.append(bytes(b for pixel in range(width) for b in line[pixel * 4 : pixel * 4 + 3]))
        else:
            rows.append(bytes(line))
    return width, height, rows


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    width, height, rows = read(argv[1])
    action = argv[2]
    if action == "size":
        print(width, height)
    elif action == "share":
        wanted = bytes.fromhex(argv[3])
        hits = sum(row[i : i + 3] == wanted for row in rows for i in range(0, len(row), 3))
        print(f"{hits / (width * height):.4f}")
    elif action == "colours":
        print(len({row[i : i + 3] for row in rows for i in range(0, len(row), 3)}))
    elif action == "at":
        x, y = int(argv[3]), int(argv[4])
        print(rows[y][x * 3 : x * 3 + 3].hex())
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

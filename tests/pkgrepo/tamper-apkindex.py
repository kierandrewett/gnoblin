#!/usr/bin/env python3
"""Change one package entry of an APKINDEX.tar.gz and keep the old signature: usage tamper-apkindex.py FILE.

The result is a valid archive whose index no longer matches its signature, which is what a server that serves a
modified index looks like. Flipping a byte of the compressed file would only make the archive unreadable.
"""

from __future__ import annotations

import gzip
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "scripts"))
from pkgrepo import alpine  # noqa: E402

path = Path(sys.argv[1])
with path.open("rb") as handle:
    (signature, _), (_, text) = alpine.read_streams(handle, 2)
members = alpine.read_members(text)
index = members["APKINDEX"].decode()
# Point the first package at another checksum: apk would install a different file than the one the index named.
first = index.split("\n", 1)[0]
assert first.startswith("C:Q1"), first
changed = "C:Q1" + ("A" if first[4] != "A" else "B") + first[5:] + "\n" + index.split("\n", 1)[1]
body = alpine.tar_member("DESCRIPTION", members["DESCRIPTION"]) + alpine.tar_member("APKINDEX", changed.encode())
path.write_bytes(signature + gzip.compress(body, mtime=0))

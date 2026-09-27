#!/usr/bin/env python3
"""Write the identity of the Gnoblin sources used for an installation."""

import json
import os
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


root = Path(__file__).resolve().parent.parent


def git(*args):
    result = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def safe_remote(remote):
    if not remote:
        return None
    parsed = urlsplit(remote)
    if parsed.scheme:
        host = parsed.netloc.rsplit("@", 1)[-1]
        return urlunsplit((parsed.scheme, host, parsed.path, "", ""))
    return remote


def source_remote():
    branch = git("symbolic-ref", "--quiet", "--short", "HEAD")
    tracking_remote = git("config", "--get", f"branch.{branch}.remote") if branch else None
    for name in (tracking_remote, "origin"):
        if name:
            url = git("remote", "get-url", name)
            if url:
                return safe_remote(url)
    return None


release = json.loads((root / "gnoblin-version.json").read_text())
gnome = json.loads((root / "gnome-versions.json").read_text())
components = gnome["components"]
embedded = root / "source-provenance.json"
provenance = json.loads(embedded.read_text()) if embedded.exists() else {}
sha = provenance.get("gitSha") or os.environ.get("GNOBLIN_SOURCE_GIT_SHA") or git("rev-parse", "HEAD")
remote = (
    safe_remote(provenance.get("gitRemote"))
    or safe_remote(os.environ.get("GNOBLIN_SOURCE_GIT_REMOTE"))
    or source_remote()
)
modified_override = os.environ.get("GNOBLIN_SOURCE_MODIFIED")
if "sourceModified" in provenance:
    source_modified = provenance["sourceModified"]
elif modified_override is not None:
    source_modified = modified_override == "1"
else:
    source_modified = bool(git("status", "--porcelain", "--untracked-files=all", "--ignore-submodules=all"))
identity = {
    "version": release["version"],
    "gnomeVersion": components["gnome-shell"]["version"],
    "mutterApi": components["mutter"]["api"],
    "components": {name: value["version"] for name, value in components.items()},
    "componentCommits": {name: value["commit"] for name, value in components.items()},
    "gitSha": sha,
    "gitRemote": remote,
    "sourceModified": source_modified,
}
if len(sys.argv) != 2:
    raise SystemExit("usage: build-identity.py <output>")
Path(sys.argv[1]).write_text(json.dumps(identity, indent=2, sort_keys=True) + "\n")

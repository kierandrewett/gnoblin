#!/usr/bin/env python3
"""Write the identity of the Gnoblin sources used for an installation."""

import argparse
import json
import os
import subprocess
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
# Only list components installed by the default Gnoblin session build.
shipped_components = {name: components[name] for name in ("mutter", "xdg-desktop-portal-gnome") if name in components}
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
    "gnomeVersion": components["mutter"]["version"],
    "mutterApi": components["mutter"]["api"],
    "components": {name: value["version"] for name, value in shipped_components.items()},
    "componentCommits": {name: value["commit"] for name, value in shipped_components.items()},
    "gitSha": sha,
    "gitRemote": remote,
    "sourceModified": source_modified,
}


def key_file_value(value):
    """Escape one string using the GLib KeyFile value escapes."""
    value = str(value)
    value = value.replace("\\", "\\\\").replace("\n", "\\n")
    value = value.replace("\r", "\\r").replace("\t", "\\t")
    if value.startswith(" "):
        value = "\\s" + value[1:]
    return value


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", help="write the existing JSON build identity here")
parser.add_argument("--ini-output", help="also write GLib KeyFile runtime metadata here")
arguments = parser.parse_args()
Path(arguments.output).write_text(json.dumps(identity, indent=2, sort_keys=True) + "\n")

if arguments.ini_output:
    metadata = ["[version]"]
    fields = {
        "gnoblin": identity.get("version"),
        "gnome": identity.get("gnomeVersion"),
        "mutter": identity.get("components", {}).get("mutter"),
        "git_remote": identity.get("gitRemote"),
        "git_sha": identity.get("gitSha"),
        "build_id": identity.get("buildId"),
    }
    metadata.extend(f"{name}={key_file_value(value)}" for name, value in fields.items() if value is not None)
    Path(arguments.ini_output).write_text("\n".join(metadata) + "\n")

#!/usr/bin/env bash
# Build the self-contained Gnoblin source tarball.
#
# The public GitHub source archive is insufficient: it omits Git submodules and
# therefore cannot reproduce Gnoblin's patched Mutter and portal backend. The
# component archives passed here are already materialised by
# scripts/make-tarball.sh and include Gnoblin's patch stacks.
set -euo pipefail

if [ "$#" -ne 4 ]; then
    echo "Usage: $0 <output> <gnoblin-version> <mutter.tar.xz> <portal.tar.xz>" >&2
    exit 2
fi

OUTPUT="$(realpath -m "$1")"
VERSION="$2"
MUTTER="$(realpath "$3")"
PORTAL="$(realpath "$4")"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" show -s --format=%ct HEAD)}"
STAGING="$(mktemp -d)"
cleanup() {
    rm -rf -- "$STAGING"
}
trap cleanup EXIT

for source in "$MUTTER" "$PORTAL"; do
    [ -f "$source" ] || {
        echo "missing prepared component source: $source" >&2
        exit 1
    }
done

TOP="gnoblin-$VERSION"
mkdir -p "$STAGING/$TOP/sources"
git -C "$ROOT" archive --format=tar HEAD | tar -xf - -C "$STAGING/$TOP"
python3 - "$ROOT" "$STAGING/$TOP/source-provenance.json" <<'PY'
import json
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

root, output = sys.argv[1:]
sha = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
def git(*args):
    result = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None

branch = git("symbolic-ref", "--quiet", "--short", "HEAD")
tracking_remote = git("config", "--get", f"branch.{branch}.remote") if branch else None
url = None
for name in (tracking_remote, "origin"):
    if name:
        url = git("remote", "get-url", name)
        if url:
            break
if url:
    parsed = urlsplit(url)
    if parsed.scheme:
        url = urlunsplit((parsed.scheme, parsed.netloc.rsplit("@", 1)[-1], parsed.path, "", ""))
# The main tree is archived from HEAD; prepared component archives can still
# reflect local patch and overlay edits when the release is made from a dirty
# checkout. Record that distinction for the installed --version output.
modified = bool(git("status", "--porcelain", "--untracked-files=all", "--ignore-submodules=all"))
Path(output).write_text(json.dumps({"gitSha": sha, "gitRemote": url, "sourceModified": modified}, sort_keys=True) + "\n")
PY
install -m 0644 -- "$MUTTER" "$STAGING/$TOP/sources/"
install -m 0644 -- "$PORTAL" "$STAGING/$TOP/sources/"

mkdir -p "$(dirname "$OUTPUT")"
tar -C "$STAGING" \
    --sort=name \
    --format=posix \
    --mtime="@$EPOCH" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --pax-option=delete=atime,delete=ctime \
    --mode='a=rX,u+w' \
    --use-compress-program='xz -T1 -9' \
    -cf "$OUTPUT" "$TOP"

echo "$OUTPUT"

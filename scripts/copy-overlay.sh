#!/usr/bin/env bash
# Copy gnoblin's overlay source files into a submodule.
#
# Large new files we author (e.g. the layer-shell implementation) live as real,
# tracked source under src/<feature>/ and are copied verbatim into the submodule
# here — they are NOT shipped as additive patches. Each src/<feature>/manifest
# maps a source file to its destination path inside a subproject. The copied
# files are untracked in the submodule (added to .git/info/exclude and removed by
# `git clean` on reset), so the submodule stays pristine.
#
# Usage: copy-overlay.sh <project> <submodule-dir>
set -euo pipefail

PROJ="${1:?usage: copy-overlay.sh <project> <submodule-dir>}"
SM="${2:?usage: copy-overlay.sh <project> <submodule-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

n=0
# Manifests live under src/ at any depth (src/protocols/<feature>/manifest,
# src/config/manifest, …) so the source tree can be reorganised by role freely.
while IFS= read -r manifest; do
  feature_dir="$(dirname "$manifest")"
  while read -r proj src dest _rest; do
    [[ -z "${proj:-}" || "$proj" == \#* ]] && continue
    [[ "$proj" != "$PROJ" ]] && continue
    [[ -f "$feature_dir/$src" ]] || { echo "overlay: missing $feature_dir/$src" >&2; exit 1; }
    mkdir -p "$SM/$(dirname "$dest")"
    cp "$feature_dir/$src" "$SM/$dest"
    # keep the submodule's git status clean
    excl="$SM/.git/info/exclude"
    [ -f "$excl" ] && ! grep -qxF "/$dest" "$excl" 2>/dev/null && echo "/$dest" >> "$excl"
    n=$((n + 1))
  done < "$manifest"
done < <(find "$ROOT/src" -name manifest -type f | sort)
echo ">> copied $n overlay file(s) into $PROJ"

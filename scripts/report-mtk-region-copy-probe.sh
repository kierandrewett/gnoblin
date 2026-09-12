#!/usr/bin/env bash
# Resolve MtkRegion-copy call-site offsets emitted by the preload probe.
set -euo pipefail

LOG="${1:?usage: report-mtk-region-copy-probe.sh LOG CLUTTER_LIBRARY}"
CLUTTER_LIBRARY="${2:?usage: report-mtk-region-copy-probe.sh LOG CLUTTER_LIBRARY}"

awk -v library="$CLUTTER_LIBRARY" '
  /^MTK_REGION_COPY / {
    path = offset = count = ""
    for (i = 2; i <= NF; i++) {
      split($i, pair, "=")
      if (pair[1] == "path") path = pair[2]
      if (pair[1] == "offset") offset = pair[2]
      if (pair[1] == "count") count = pair[2]
    }
    if (path == library)
      print offset, count
  }
' "$LOG" | while read -r offset count; do
  printf 'count=%s offset=%s ' "$count" "$offset"
  addr2line -f -C -e "$CLUTTER_LIBRARY" "$offset" | paste -sd ' ' -
done

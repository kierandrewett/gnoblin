#!/usr/bin/env bash
# Run external design critique against one or more screenshots.
#
# This is intentionally not part of Tier 1: it depends on paid/model CLIs and
# network/auth state. Use it before signing off visual Slint changes:
#
#   scripts/design-review.sh /tmp/gnoblin-notif-cc-history-expanded.png
#
# Outputs a Markdown report under build/design-review/.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${GNOBLIN_DESIGN_REVIEW_DIR:-$ROOT/build/design-review}"
BUDGET="${GNOBLIN_DESIGN_REVIEW_BUDGET_USD:-1}"
MODEL="${GNOBLIN_DESIGN_REVIEW_CLAUDE_MODEL:-sonnet}"
RUN_OPENCODE="${GNOBLIN_DESIGN_REVIEW_OPENCODE:-1}"

usage() {
  cat >&2 <<'USAGE'
usage: scripts/design-review.sh SCREENSHOT [SCREENSHOT...]

Runs external UI/design critique using available local model CLIs.

Environment:
  GNOBLIN_DESIGN_REVIEW_BUDGET_USD   Claude max budget, default 1
  GNOBLIN_DESIGN_REVIEW_CLAUDE_MODEL Claude model alias, default sonnet
  GNOBLIN_DESIGN_REVIEW_OPENCODE     1/0, default 1
  GNOBLIN_DESIGN_REVIEW_DIR          output dir, default build/design-review
USAGE
}

if [ "$#" -lt 1 ]; then
  usage
  exit 2
fi

mkdir -p "$OUT_DIR"
stamp="$(date +%Y%m%d-%H%M%S)"
report="$OUT_DIR/$stamp.md"

prompt='You are a senior desktop shell UI designer reviewing a Linux desktop shell built with Slint. Critique ONLY the visible top bar, dock-adjacent shell chrome, and quick settings/control-centre panel in the attached screenshot.

Use this rubric:
- Composition and hierarchy: clear grouping, enough whitespace, not visually noisy.
- Icon quality: recognized metaphors, consistent optical weight, crisp at displayed size, no mixed icon families.
- Desktop shell fit: compact but not cramped, status controls should feel native and calm.
- State clarity: active/hover/disabled states must be obvious without relying only on blue.
- Slint/design-system discipline: spacing should come from named tokens; avoid magic numbers and one-off component internals.
- Accessibility: readable text, adequate hit targets where the shell allows them, no ambiguous icon-only actions.

Return strict JSON with keys:
score_0_10, fatal_issues, improvements, first_change, reviewer_confidence.
Be blunt. Do not edit files.'

{
  echo "# Gnoblin Design Review"
  echo
  echo "- Time: $stamp"
  echo "- Root: $ROOT"
  echo "- Screenshots:"
  for shot in "$@"; do
    echo "  - $shot"
  done
  echo
} >"$report"

run_claude() {
  local shot="$1"

  if ! command -v claude >/dev/null 2>&1; then
    {
      echo "## Claude"
      echo
      echo "Skipped: \`claude\` not found."
      echo
    } >>"$report"
    return 0
  fi

  {
    echo "## Claude Vision Review: $shot"
    echo
  } >>"$report"

  timeout 120s claude -p \
    --permission-mode dontAsk \
    --allowedTools Read \
    --add-dir /tmp \
    --add-dir "$ROOT" \
    --max-budget-usd "$BUDGET" \
    --model "$MODEL" \
    "$prompt

Screenshot path: $shot" >>"$report" 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    {
      echo
      echo "Claude review exited with status $rc."
      echo
    } >>"$report"
  fi
}

run_opencode() {
  local shot="$1"

  if [ "$RUN_OPENCODE" != "1" ]; then
    return 0
  fi
  if ! command -v opencode >/dev/null 2>&1; then
    {
      echo "## Opencode"
      echo
      echo "Skipped: \`opencode\` not found."
      echo
    } >>"$report"
    return 0
  fi

  {
    echo "## Opencode Review: $shot"
    echo
  } >>"$report"

  timeout 180s opencode run \
    --file "$shot" \
    --format default \
    "$prompt

If your selected model cannot inspect the image, inspect these Slint files read-only and critique the implementation discipline instead:
- src/clients/shell-ui/vendor/slint/Panel.slint
- src/clients/shell-ui/vendor/slint/Popouts.slint
- src/clients/shell-ui/vendor/slint/Tokens.slint

Do not edit files." >>"$report" 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    {
      echo
      echo "Opencode review exited with status $rc."
      echo
    } >>"$report"
  fi
}

for shot in "$@"; do
  if [ ! -f "$shot" ]; then
    echo "missing screenshot: $shot" >&2
    exit 2
  fi

  run_claude "$shot"
  run_opencode "$shot"
done

echo "$report"

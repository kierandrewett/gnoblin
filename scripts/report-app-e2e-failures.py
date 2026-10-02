#!/usr/bin/env python3
"""Build a concise issue body from uploaded application E2E shard summaries."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re

ISSUE_TITLE = "[Gnoblin app E2E] Application compatibility triage"


def markdown_text(value: object, limit: int = 280) -> str:
    text = " ".join(str(value or "").split())
    text = text.replace("`", "")
    text = re.sub(r"https?://\S+", "[URL]", text)
    text = re.sub(r"(?<![A-Za-z0-9])/(?:[^\s/]+/)*[^\s/]*", "[path]", text)
    if len(text) > limit:
        return text[: limit - 1] + "…"
    return text


def load_summaries(artifact_root: Path) -> tuple[list[dict], list[str]]:
    summaries = []
    errors = []
    for path in sorted(artifact_root.rglob("summary.json")):
        try:
            summary = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{path.relative_to(artifact_root)}: {error}")
            continue
        summaries.append(summary)
    return summaries, errors


def load_artifact_names(artifact_root: Path) -> list[str]:
    try:
        return sorted(path.name for path in artifact_root.glob("gnoblin-app-e2e-shard-*") if path.is_dir())
    except OSError:
        return []


def load_expected_apps(artifact_root: Path) -> tuple[dict[str, dict], list[str]]:
    expected: dict[str, dict] = {}
    errors = []
    for path in sorted(artifact_root.glob("gnoblin-app-e2e-shard-*")):
        if not path.is_dir():
            continue
        manifest_path = path / "shard.json"
        if not manifest_path.exists():
            continue
        try:
            manifest = json.loads(manifest_path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{manifest_path.relative_to(artifact_root)}: {error}")
            continue
        shard_index = manifest.get("shard", {}).get("index")
        for app in manifest.get("apps", []):
            app_id = str(app.get("app_id") or "").strip()
            if app_id:
                expected[app_id] = {"app": app, "shard_index": shard_index}
    return expected, errors


def build_report(artifact_root: Path) -> str:
    summaries, parse_errors = load_summaries(artifact_root)
    artifacts = load_artifact_names(artifact_root)
    expected_apps, manifest_errors = load_expected_apps(artifact_root)
    outcomes = [outcome for summary in summaries for outcome in summary.get("apps", [])]
    status_counts = Counter(str(outcome.get("status") or "unknown") for outcome in outcomes)
    failed_outcomes = sorted(
        (outcome for outcome in outcomes if outcome.get("status") != "exercised"),
        key=lambda outcome: (str(outcome.get("app_id") or ""), str(outcome.get("status") or "unknown")),
    )
    outcome_app_ids = {str(outcome.get("app_id") or "") for outcome in outcomes}
    summaries_by_shard = {
        summary.get("shard", {}).get("index"): summary
        for summary in summaries
        if summary.get("shard", {}).get("index") is not None
    }
    unreported = []
    for app_id, expected in expected_apps.items():
        if app_id in outcome_app_ids:
            continue
        shard_index = expected["shard_index"]
        detail = (
            f"shard {shard_index} did not produce a summary"
            if shard_index not in summaries_by_shard
            else f"shard {shard_index} summary omitted this requested app"
        )
        unreported.append({"app_id": app_id, "status": "unreported", "error": detail})
    unreported.sort(key=lambda outcome: str(outcome.get("app_id") or ""))
    failed = sorted(
        [*failed_outcomes, *unreported],
        key=lambda outcome: (str(outcome.get("app_id") or ""), str(outcome.get("status") or "unknown")),
    )
    status_counts["unreported"] += len(unreported)
    fingerprint_data = sorted({(str(item.get("app_id", "")), str(item.get("status", "unknown"))) for item in failed})
    fingerprint = hashlib.sha256(json.dumps(fingerprint_data, separators=(",", ":")).encode()).hexdigest()[:16]

    repo = os.environ.get("GITHUB_REPOSITORY", "kierandrewett/gnoblin")
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
    run_id = os.environ.get("GITHUB_RUN_ID", "unknown")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
    commit = os.environ.get("GITHUB_SHA", "unknown")
    artifact_name = f"gnoblin-app-e2e-shard-*-{run_id}-{attempt}"
    commit_reference = f"[`{commit}`]({server}/{repo}/commit/{commit})" if commit != "unknown" else "`unknown`"

    lines = [
        f"# {ISSUE_TITLE}",
        "",
        "This automated report records compatibility outcomes from the real Gnoblin app E2E run. Failures can come from Gnoblin, an app, or the test environment; inspect the attached artifacts before assigning a cause.",
        "",
        f"- Run: {server}/{repo}/actions/runs/{run_id}",
        f"- Commit: {commit_reference}",
        f"- Expected artifact pattern: `{artifact_name}`",
        f"- Failure fingerprint: `{fingerprint}`",
        f"- Shards with summaries: {len(summaries)}",
        f"- Apps requested by shard manifests: {len(expected_apps)}",
        f"- Apps with outcomes: {len(outcomes)}",
        f"- Apps without outcomes: {len(unreported)}",
        f"- Apps with failed outcomes: {len(failed_outcomes)}",
        "",
        "## Outcome counts",
        "",
    ]
    if status_counts:
        lines.extend(
            f"- {count} `{markdown_text(status, limit=80)}`" for status, count in sorted(status_counts.items())
        )
    else:
        lines.append("- No per-app outcomes were recorded.")

    lines.extend(["", "## Shard artifacts", ""])
    if artifacts:
        lines.extend(f"- `{markdown_text(name, limit=180)}`" for name in artifacts)
    else:
        lines.append("- No shard artifacts were downloaded.")

    lines.extend(["", "## Apps needing triage", ""])
    if failed:
        for outcome in failed[:150]:
            app_id = markdown_text(outcome.get("app_id", "unknown app"), limit=180)
            status = markdown_text(outcome.get("status", "unknown"), limit=80)
            detail = outcome.get("error") or "See shard artifacts for app logs, operation traces and screenshots."
            lines.append(f"- `{app_id}` — `{status}`: `{markdown_text(detail, limit=120)}`")
        if len(failed) > 150:
            lines.append(f"- … {len(failed) - 150} more outcomes are listed in the shard artifacts.")
    else:
        lines.append("No failed app outcomes were recorded; inspect runner and Shell logs for the shard setup failure.")

    if parse_errors:
        lines.extend(["", "## Unreadable summaries", ""])
        lines.extend(f"- `{markdown_text(error)}`" for error in parse_errors)
    if manifest_errors:
        lines.extend(["", "## Unreadable shard manifests", ""])
        lines.extend(f"- `{markdown_text(error)}`" for error in manifest_errors)
    if not summaries:
        lines.extend(
            [
                "",
                "No `summary.json` artifact was available. Use the run link and its job logs to diagnose a build, setup or infrastructure failure.",
            ]
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = build_report(args.artifact_root)
    args.output.write_text(report)
    print(f"Wrote app E2E failure report: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

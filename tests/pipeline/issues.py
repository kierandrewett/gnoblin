#!/usr/bin/env python3
"""Raise, update and close one GitHub issue for each release that fails the release pipeline.

usage: tests/pipeline/issues.py (--coverage FILE | --results DIR) [--repo OWNER/NAME] [--run-url URL]
                                [--dry-run | --apply] [--issues-json FILE] [--summary FILE] [--out DIR]

--coverage reads the verdicts of the release gate (coverage.json of tests/packages/gate.py, key "targets"): every
target on every architecture with each stage (prefix, packages, suite, graphical), and every planned release with its
blocker ("build": "not built"). --results reads the result.json files of a graphical VM test run on its own
(.github/workflows/graphical.yml by hand); there, only the graphical stage is known.

A release has one issue at most, with the title "Release pipeline: <name>" and the label release-pipeline. For each
release, over all of its architectures:

  a stage failed or the release is not built, and the release is not at its end of life
      create the issue, or update the open one: the body is rewritten with this run. A comment is added only when the
      failures changed (another stage, check or detail), so a release that fails the same way every night gets no
      new comment. The body names each failing architecture, stage and check, the run and what unblocks it.
  a stage failed or the release is not built, and the release is at its end of life
      no issue: a notice only. An open issue of the release is closed with a comment.
  every stage passed on every architecture, in a run that knows every stage (--coverage)
      an open issue gets a comment with the run and is closed.
  anything else (a stage did not run, or a graphical-only run that passed)
      nothing changes. An open issue stays open, because the run does not show that the release passes.

The issues are found by their exact title among the open issues with the label. When two open issues have the same
title, the oldest is used and the others are named in the summary.

--dry-run (the default) prints the actions and changes nothing. --apply carries them out through the GitHub REST API
with GITHUB_TOKEN, and creates the label when it is missing. --issues-json reads the open issues from a file instead of
GitHub (a list of {"number", "title", "body", "html_url"}). --summary appends the table to that file
(GITHUB_STEP_SUMMARY). --out writes summary.md and actions.json there. actions.json is a list of
{"action": "create" | "update" | "close" | "notice", "release", "title", "number", "body", "comment"}.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
_specification = importlib.util.spec_from_file_location("gate", ROOT / "tests/packages/gate.py")
gate = importlib.util.module_from_spec(_specification)
_specification.loader.exec_module(gate)

LABEL = "release-pipeline"
LABEL_COLOUR = "d93f0b"
LABEL_DESCRIPTION = "A release fails a stage of the release pipeline, or Gnoblin has no build for it"
API = "https://api.github.com"
MARKER = re.compile(r"<!-- release-pipeline-signature: (.*?) -->")
FAILING = (gate.FAIL, gate.NOT_BUILT)
# What a person does about a failure of each stage. Plain words, for the issue body.
REMEDY = {
    "build": "Gnoblin has no build for this release. It is unblocked when the release ships the libraries above at "
    "the versions of the build floors, or when a patch series lets Gnoblin build against the older ones. Then move "
    "build_from in scripts/targets.json, so that the release gets a job.",
    "prefix": "The prefix build failed. The log of the prefix job names the step that failed.",
    "packages": "pkgsmith did not write the packages. The log of the packages job has the error.",
    "suite": "A layer of the package suite failed. The package-tests artifact of the target has the failing case and "
    "its log (design/package-testing.md).",
    "graphical": "The graphical VM test failed. The graphical artifact of the target has the serial log, the journal "
    "and the screenshots (design/graphical-testing.md).",
}


def title(name):
    return f"Release pipeline: {name}"


def from_graphical_result(result):
    """A verdict from a result.json of a graphical run on its own. Only the graphical stage is known."""
    if result.get("result") == "not packaged":
        stages = {"build": {"status": gate.NOT_BUILT, "check": "build", "detail": result.get("reason") or "not built"}}
    else:
        stages = {"graphical": gate.graphical_stage(result)}
    return {
        "name": result["name"],
        "slug": result["slug"],
        "arch": result.get("arch", "amd64"),
        "state": result.get("state") or "",
        "eol": bool(result.get("eol")),
        "eol_date": result.get("eol_date"),
        "stages": stages,
        "passed": result.get("result") == "pass",
        "complete": False,
        "evidence": result.get("artifact") or "",
    }


def arches_of(verdicts):
    return [verdict["arch"] for verdict in verdicts]


def failures(verdicts):
    """(arch, stage, check, detail) of each failing stage of a release, in a stable order. A release that is not built
    is not built on any architecture: its row has the architecture "-", so the verdict of the release gate and the
    "not packaged" results of a graphical run give one failure, and one issue."""
    found = set()
    for verdict in verdicts:
        for stage, data in verdict["stages"].items():
            if data["status"] in FAILING:
                arch = "-" if stage == "build" else verdict["arch"]
                found.add((arch, stage, data.get("check") or stage, data.get("detail") or ""))
    return sorted(found)


def signature(found):
    """What a new comment depends on. Numbers that change from run to run (times, process ids, sizes) are left out,
    so the same failure has the same signature."""
    parts = [f"{arch} {stage} {check}: {re.sub(r'[0-9]+', 'N', detail)}" for arch, stage, check, detail in found]
    return "; ".join(parts)[:600].replace("-->", "- ->")


def body(name, verdicts, found, run_url):
    eol_date = next((verdict["eol_date"] for verdict in verdicts if verdict.get("eol_date")), None)
    planned = all(stage == "build" for _, stage, _, _ in found)
    opening = (
        f"Gnoblin has no build for **{name}**." if planned else f"**{name}** fails the release pipeline of Gnoblin."
    )
    lines = [
        f"{opening} {name} is not at its end of life"
        + (f" (end of life {eol_date})." if eol_date else " (it has no end-of-life date)."),
        "",
        "| Architecture | Stage | Check | Detail |",
        "| --- | --- | --- | --- |",
    ]
    for arch, stage, check, detail in found:
        clean = detail.replace("|", "/").replace("\n", " ")
        lines.append(f"| {arch} | {stage} | `{check}` | {clean[:400]} |")
    evidence = sorted({verdict["evidence"] for verdict in verdicts if verdict.get("evidence")})
    lines += ["", f"- Run: {run_url or 'a local run'}"]
    if evidence:
        lines.append(f"- Evidence: the artifacts {', '.join(f'`{item}`' for item in evidence)} of that run")
    lines += ["", "What unblocks it:", ""]
    for stage in dict.fromkeys(stage for _, stage, _, _ in found):
        lines.append(f"- {stage}: {REMEDY.get(stage, 'See the run.')}")
    lines += [
        "",
        "The release workflow (`.github/workflows/release.yml`) keeps this issue up to date. It adds a comment when "
        "the failures change, and closes the issue when every stage passes on every architecture. See "
        "`design/release-pipeline.md`.",
        "",
        f"<!-- release-pipeline-signature: {signature(found)} -->",
    ]
    return "\n".join(lines)


def plan(verdicts, issues, run_url):
    """The actions for the verdicts, given the open issues with the label. Pure: no network."""
    by_title = {}
    for issue in sorted(issues, key=lambda item: item["number"]):
        by_title.setdefault(issue["title"], []).append(issue)
    releases: dict[str, list[dict]] = {}
    for verdict in verdicts:
        releases.setdefault(verdict["name"], []).append(verdict)
    for name, group in releases.items():
        arches = [verdict["arch"] for verdict in group]
        if len(arches) != len(set(arches)):
            raise ValueError(f"two verdicts for one architecture of {name}")
    actions, rows = [], []
    run = run_url or "a local run"
    for name, group in releases.items():
        name_title = title(name)
        found_issues = by_title.get(name_title, [])
        issue = found_issues[0] if found_issues else None
        link = issue["html_url"] if issue else ""
        found = failures(group)
        eol = any(verdict["eol"] for verdict in group)
        eol_date = next((verdict["eol_date"] for verdict in group if verdict.get("eol_date")), None)
        complete_pass = all(verdict["passed"] and verdict["complete"] for verdict in group)
        if found and eol:
            result = "not built" if all(stage == "build" for _, stage, _, _ in found) else "fail"
            note = "EOL: no issue, notice only"
            actions.append({"action": "notice", "release": name, "title": name_title,
                            "comment": f"{name} {'is not built' if result == 'not built' else 'failed a stage'}, and it is "
                                       f"at its end of life ({eol_date}): no issue"})  # fmt: skip
            if issue:
                actions.append({"action": "close", "release": name, "number": issue["number"], "title": name_title,
                                "comment": f"{name} reached its end of life ({eol_date}). It is no longer a target, "
                                           f"so this issue is closed. Run: {run}"})  # fmt: skip
                note += f"; closes #{issue['number']}"
        elif found:
            result = "not built" if all(stage == "build" for _, stage, _, _ in found) else "fail"
            new_body = body(name, group, found, run_url)
            if issue is None:
                actions.append({"action": "create", "release": name, "title": name_title, "body": new_body,
                                "labels": [LABEL]})  # fmt: skip
                note, link = "new issue", "(new)"
            else:
                old = MARKER.search(issue.get("body") or "")
                action = {"action": "update", "release": name, "number": issue["number"], "title": name_title,
                          "body": new_body}  # fmt: skip
                if not old or old.group(1) != signature(found):
                    listed = "; ".join(f"{arch} {stage} `{check}`" for arch, stage, check, _ in found)
                    action["comment"] = f"The failures changed: {listed}. Run: {run}"
                    note = "updated, failures changed"
                else:
                    note = "updated, same failures"
                actions.append(action)
        elif complete_pass:
            result, note = "pass", ""
            if issue:
                arches = ", ".join(sorted(arches_of(group)))
                actions.append({"action": "close", "release": name, "number": issue["number"], "title": name_title,
                                "comment": f"{name} passes every stage on {arches} again. Run: {run}"})  # fmt: skip
                note = f"passes; closes #{issue['number']}"
        else:
            result = "not run" if not all(verdict["passed"] for verdict in group) else "pass (not every stage ran)"
            waiting = [
                f"{verdict['arch']} {stage}: {data['detail']}"
                for verdict in group
                for stage, data in verdict["stages"].items()
                if data["status"] == gate.NOT_RUN
            ]
            note = "; ".join(waiting) or "the run does not show every stage"
            if issue:
                note += f"; #{issue['number']} stays open"
        for extra in found_issues[1:]:
            note += f"; duplicate open issue #{extra['number']}"
        rows.append(
            {
                "release": name,
                "eol": eol_date or "none",
                "eol_passed": eol,
                "result": result,
                "failures": "; ".join(f"{arch} {stage}: {check}" for arch, stage, check, _ in found),
                "issue": link,
                "note": note,
            }
        )
    return actions, rows


def table(rows, dry_run):
    lines = [
        "## Release pipeline issues" + (" (dry run: no issue was changed)" if dry_run else ""),
        "",
        "| Release | End of life | Result | Failing stages | Issue | Note |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        eol = f"{row['eol']} (passed)" if row["eol_passed"] else row["eol"]
        cells = [row["release"], eol, row["result"], row["failures"], row["issue"], row["note"]]
        lines.append("| " + " | ".join(str(cell).replace("|", "/") for cell in cells) + " |")
    return "\n".join(lines) + "\n"


class GitHub:
    def __init__(self, repo, token):
        self.repo, self.token = repo, token

    def call(self, method, path, payload=None):
        request = urllib.request.Request(
            f"{API}/repos/{self.repo}{path}",
            method=method,
            data=json.dumps(payload).encode() if payload is not None else None,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "X-GitHub-Api-Version": "2022-11-28",
                "User-Agent": "gnoblin-release-pipeline",
            },
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            text = response.read()
            return json.loads(text) if text else None

    def open_issues(self):
        issues, page = [], 1
        while True:
            batch = self.call("GET", f"/issues?state=open&labels={LABEL}&per_page=100&page={page}")
            issues += [item for item in batch if "pull_request" not in item]
            if len(batch) < 100:
                return issues
            page += 1

    def ensure_label(self):
        try:
            self.call("GET", f"/labels/{LABEL}")
        except urllib.error.HTTPError as exception:
            if exception.code != 404:
                raise
            self.call("POST", "/labels", {"name": LABEL, "color": LABEL_COLOUR, "description": LABEL_DESCRIPTION})

    def apply(self, action):
        if action["action"] == "notice":
            return "notice"
        if action["action"] == "create":
            created = self.call("POST", "/issues", {"title": action["title"], "body": action["body"],
                                                    "labels": action["labels"]})  # fmt: skip
            return created["html_url"]
        number = action["number"]
        if action["action"] == "update":
            self.call("PATCH", f"/issues/{number}", {"body": action["body"]})
        if action.get("comment"):
            self.call("POST", f"/issues/{number}/comments", {"body": action["comment"]})
        if action["action"] == "close":
            self.call("PATCH", f"/issues/{number}", {"state": "closed", "state_reason": "completed"})
        return f"#{number}"


def load(arguments):
    if arguments.coverage:
        return json.loads(arguments.coverage.read_text())["targets"]
    return [
        from_graphical_result(json.loads(path.read_text())) for path in sorted(arguments.results.rglob("result.json"))
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--coverage", type=Path, help="coverage.json of the release gate")
    source.add_argument("--results", type=Path, help="a folder with the result.json files of a graphical run")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--run-url", default="")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", dest="dry_run", action="store_true", default=True)
    mode.add_argument("--apply", dest="dry_run", action="store_false")
    parser.add_argument("--issues-json", type=Path, help="read the open issues from this file, not from GitHub")
    parser.add_argument("--summary", type=Path, help="append the table to this file")
    parser.add_argument("--out", type=Path, help="write summary.md and actions.json here")
    arguments = parser.parse_args()

    verdicts = load(arguments)
    if not verdicts:
        print("issues: no verdicts found", file=sys.stderr)
        return 1
    github = None
    if arguments.issues_json:
        issues = json.loads(arguments.issues_json.read_text())
    else:
        token = os.environ.get("GITHUB_TOKEN", "")
        if not (arguments.repo and token):
            print(
                "issues: --repo and GITHUB_TOKEN are needed to read the issues (or use --issues-json)", file=sys.stderr
            )
            return 1
        github = GitHub(arguments.repo, token)
        issues = github.open_issues()
    actions, rows = plan(verdicts, issues, arguments.run_url)
    dry_run = arguments.dry_run or github is None
    if dry_run:
        for action in actions:
            if action["action"] == "notice":
                print(f"notice: {action['comment']}")
                continue
            target = f"#{action['number']}" if "number" in action else "a new issue"
            print(f"would {action['action']} {target}: {action['title']}")
            if action.get("comment"):
                print(f"  comment: {action['comment']}")
    else:
        github.ensure_label()
        for action in actions:
            if action["action"] == "notice":
                print(f"::notice::{action['comment']}")
                continue
            print(f"{action['action']} {github.apply(action)}: {action['title']}")
    text = table(rows, dry_run)
    print(text)
    if arguments.summary:
        with arguments.summary.open("a") as handle:
            handle.write(text)
    if arguments.out:
        arguments.out.mkdir(parents=True, exist_ok=True)
        (arguments.out / "summary.md").write_text(text)
        (arguments.out / "actions.json").write_text(json.dumps(actions, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

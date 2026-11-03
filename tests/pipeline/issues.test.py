#!/usr/bin/env python3
"""Check the issue logic of the release pipeline (tests/pipeline/issues.py). No network.

1. plan() on made-up verdicts of the release gate: a failure of each stage, an end-of-life failure, an experimental
   failure, an arm64-only failure, a recovered release, a duplicate issue, an unchanged and a changed failure, a stage
   that did not run, and planned releases with and without an end of life.
2. plan() on the result.json fixtures of a graphical run on its own (fixtures/results): only the graphical stage is
   known there, so a pass closes nothing, and a "not packaged" release gives the same issue as a planned row.
3. End to end: the release gate (tests/packages/gate.py) on a small made-up run, then issues.py --dry-run on its
   coverage.json, read back from actions.json.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
specification = importlib.util.spec_from_file_location("issues", HERE / "issues.py")
issues = importlib.util.module_from_spec(specification)
specification.loader.exec_module(issues)

FIXTURES = HERE / "fixtures"
RUN = "https://github.com/kierandrewett/gnoblin/actions/runs/2"
PASS = {"status": "pass", "check": "", "detail": ""}


def verdict(name, arch="amd64", failing=None, eol=False, state="built", not_run=None):
    """A verdict of the release gate. failing: (stage, check, detail); the stages after it did not run."""
    stages, blocked = {}, None
    for stage in ("prefix", "packages", "suite", "graphical"):
        if blocked:
            stages[stage] = {"status": "not run", "check": "", "detail": f"not run: {blocked}"}
        elif failing and failing[0] == stage:
            stages[stage] = {"status": "fail", "check": failing[1], "detail": failing[2]}
            blocked = f"{stage} failed"
        elif not_run == stage:
            stages[stage] = {"status": "not run", "check": "", "detail": "not run: no graphical job on arm64"}
            blocked = f"{stage} did not run"
        else:
            stages[stage] = dict(PASS)
    passed = all(item["status"] == "pass" for item in stages.values())
    slug = name.lower().replace(" ", "")
    return {"name": name, "slug": slug, "arch": arch, "state": state, "eol": eol, "eol_date": "2027-01-01",
            "stages": stages, "passed": passed, "complete": True, "evidence": f"verdict-x-{slug}-{arch}"}  # fmt: skip


def planned(name, blocker, eol=False):
    return {"name": name, "slug": "", "arch": "-", "state": "planned", "eol": eol, "eol_date": "2028-06-30",
            "stages": {"build": {"status": "not built", "check": "build", "detail": blocker}}, "passed": False,
            "complete": True, "evidence": ""}  # fmt: skip


def by_release(actions):
    found = {}
    for action in actions:
        found.setdefault(action["release"], []).append(action)
    return found


class ReleaseRun(unittest.TestCase):
    """The verdicts of a release run, where every stage is known."""

    open_issues = [
        {"number": 30, "title": "Release pipeline: Fedora 43", "body": "", "html_url": "u/30"},
        {"number": 31, "title": "Release pipeline: Fedora 41", "body": "", "html_url": "u/31"},
        {"number": 32, "title": "Release pipeline: Arch Linux", "body": "", "html_url": "u/32"},
        {"number": 33, "title": "Release pipeline: Arch Linux", "body": "", "html_url": "u/33"},
        {"number": 34, "title": "Release pipeline: Ubuntu 26.04", "body": "", "html_url": "u/34"},
    ]

    def setUp(self):
        self.verdicts = [
            verdict("Fedora 44", failing=("prefix", "prefix", "build-prefix.sh exited 1")),
            verdict("Fedora 43"),
            verdict("Fedora 43", "arm64"),
            verdict("Fedora 42", failing=("packages", "packages", "pkgsmith exited 2"), state="experimental"),
            verdict("Fedora 41", failing=("graphical", "session", "no compositor process"), eol=True),
            verdict("Debian 13", failing=("suite", "runtime", "runtime: failed R3")),
            verdict("Arch Linux"),
            verdict("Arch Linux", "arm64", failing=("graphical", "image", "no cloud image for arm64: none")),
            verdict("Ubuntu 26.04"),
            verdict("Ubuntu 26.04", "arm64", not_run="graphical"),
            planned("Debian 12", "GLib 2.74.6 is below the build floors"),
            planned("Fedora 40", "GLib 2.80.5 is below the build floors", eol=True),
        ]
        self.actions, self.rows = issues.plan(self.verdicts, self.open_issues, RUN)
        self.found = by_release(self.actions)
        self.row = {row["release"]: row for row in self.rows}

    def test_a_build_failure_raises_an_issue_that_names_the_stage_the_check_and_the_run(self):
        (action,) = self.found["Fedora 44"]
        self.assertEqual((action["action"], action["title"], action["labels"]), ("create", "Release pipeline: Fedora 44",
                                                                                 ["release-pipeline"]))  # fmt: skip
        self.assertIn("| amd64 | prefix | `prefix` | build-prefix.sh exited 1 |", action["body"])
        self.assertIn(f"- Run: {RUN}", action["body"])
        self.assertIn("`verdict-x-fedora44-amd64`", action["body"])
        self.assertIn("- prefix: The prefix build failed.", action["body"])

    def test_a_package_writing_failure_of_an_experimental_target_raises_an_issue_too(self):
        (action,) = self.found["Fedora 42"]
        self.assertEqual(action["action"], "create")
        self.assertIn("| amd64 | packages | `packages` | pkgsmith exited 2 |", action["body"])

    def test_a_package_suite_failure_names_the_layer(self):
        (action,) = self.found["Debian 13"]
        self.assertIn("| amd64 | suite | `runtime` | runtime: failed R3 |", action["body"])

    def test_an_arm64_only_failure_names_arm64(self):
        (action,) = self.found["Arch Linux"]
        self.assertEqual((action["action"], action["number"]), ("update", 32))
        self.assertIn("| arm64 | graphical | `image` |", action["body"])
        self.assertNotIn("| amd64 |", action["body"])

    def test_duplicate_open_issues_use_the_oldest_and_are_named(self):
        self.assertIn("duplicate open issue #33", self.row["Arch Linux"]["note"])
        self.assertEqual([action.get("number") for action in self.found["Arch Linux"]], [32])

    def test_an_end_of_life_failure_raises_no_issue_and_closes_the_open_one(self):
        notice, close = self.found["Fedora 41"]
        self.assertEqual(notice["action"], "notice")
        self.assertEqual((close["action"], close["number"]), ("close", 31))
        self.assertIn("end of life", close["comment"])
        self.assertFalse(
            [action for action in self.actions if action["action"] == "create" and "41" in action["title"]]
        )

    def test_a_recovered_release_gets_its_issue_closed_with_a_comment(self):
        (action,) = self.found["Fedora 43"]
        self.assertEqual((action["action"], action["number"]), ("close", 30))
        self.assertEqual(action["comment"], f"Fedora 43 passes every stage on amd64, arm64 again. Run: {RUN}")

    def test_a_stage_that_did_not_run_changes_nothing(self):
        self.assertNotIn("Ubuntu 26.04", self.found)
        self.assertEqual(self.row["Ubuntu 26.04"]["result"], "not run")
        self.assertIn(
            "arm64 graphical: not run: no graphical job on arm64; #34 stays open", self.row["Ubuntu 26.04"]["note"]
        )

    def test_a_planned_release_raises_an_issue_with_its_blocker_and_what_unblocks_it(self):
        (action,) = self.found["Debian 12"]
        self.assertEqual(action["action"], "create")
        self.assertTrue(
            action["body"].startswith("Gnoblin has no build for **Debian 12**. Debian 12 is not at its end")
        )
        self.assertIn("| - | build | `build` | GLib 2.74.6 is below the build floors |", action["body"])
        self.assertIn("- build: Gnoblin has no build for this release. It is unblocked when", action["body"])
        self.assertEqual(self.row["Debian 12"]["result"], "not built")

    def test_an_end_of_life_planned_release_gets_a_notice_only(self):
        (action,) = self.found["Fedora 40"]
        self.assertEqual(action["action"], "notice")
        self.assertIn("not built", action["comment"])

    def test_the_same_failures_update_without_a_comment_and_changed_ones_comment(self):
        # Carry out the first run on the open issues: create, rewrite the bodies, close.
        open_issues = [dict(item) for item in self.open_issues]
        for number, action in enumerate(self.actions):
            if action["action"] == "create":
                open_issues.append({"number": 100 + number, "title": action["title"], "body": action["body"],
                                    "html_url": "u"})  # fmt: skip
            elif action["action"] == "update":
                next(item for item in open_issues if item["number"] == action["number"])["body"] = action["body"]
            elif action["action"] == "close":
                open_issues = [item for item in open_issues if item["number"] != action["number"]]
        actions, _ = issues.plan(self.verdicts, open_issues, RUN)
        self.assertEqual([action for action in actions if action["action"] in ("create", "close")], [])
        self.assertEqual([action for action in actions if action.get("comment") and action["action"] != "notice"], [])
        self.assertEqual({action["release"] for action in actions if action["action"] == "update"},
                         {"Fedora 44", "Fedora 42", "Debian 13", "Arch Linux", "Debian 12"})  # fmt: skip
        # The prefix build of Fedora 44 now passes and its suite fails: the failure changed, so a comment.
        changed = [item for item in self.verdicts if item["name"] != "Fedora 44"]
        changed.append(verdict("Fedora 44", failing=("suite", "install", "install: failed I2")))
        actions, _ = issues.plan(changed, open_issues, RUN)
        (action,) = by_release(actions)["Fedora 44"]
        self.assertEqual(action["action"], "update")
        self.assertIn("The failures changed: amd64 suite `install`", action["comment"])

    def test_numbers_in_the_detail_do_not_change_the_signature(self):
        first = issues.failures([verdict("A", failing=("graphical", "boot", "no SSH login within 600 s"))])
        second = issues.failures([verdict("A", failing=("graphical", "boot", "no SSH login within 1800 s"))])
        self.assertEqual(issues.signature(first), issues.signature(second))

    def test_two_verdicts_for_one_architecture_are_an_error(self):
        with self.assertRaises(ValueError):
            issues.plan([verdict("A"), verdict("A")], [], RUN)


class GraphicalRun(unittest.TestCase):
    """The result.json files of a graphical run on its own: only the graphical stage is known."""

    def setUp(self):
        verdicts = [
            issues.from_graphical_result(json.loads(path.read_text()))
            for path in sorted((FIXTURES / "results").rglob("result.json"))
        ]
        open_issues = json.loads((FIXTURES / "open-issues.json").read_text())
        self.actions, self.rows = issues.plan(verdicts, open_issues, RUN)
        self.found = by_release(self.actions)
        self.row = {row["release"]: row for row in self.rows}

    def test_a_pass_of_the_graphical_stage_alone_closes_nothing(self):
        self.assertNotIn("Fedora 44", self.found)
        self.assertIn("#12 stays open", self.row["Fedora 44"]["note"])

    def test_a_graphical_failure_raises_an_issue(self):
        (action,) = self.found["Debian 13"]
        self.assertEqual(action["action"], "create")
        self.assertIn("| amd64 | graphical | `keyboard` |", action["body"])

    def test_not_packaged_is_the_same_issue_as_a_planned_row(self):
        # Ubuntu 24.04's open issue was written from a planned row of a release run: same signature, no comment.
        (action,) = self.found["Ubuntu 24.04"]
        self.assertEqual((action["action"], action["number"]), ("update", 15))
        self.assertNotIn("comment", action)
        release_row = planned("Ubuntu 24.04", "GLib 2.80.0 is below the build floors")
        self.assertEqual(
            issues.failures([release_row]),
            issues.failures(
                [issues.from_graphical_result(json.loads((FIXTURES / "results/ubuntu24.04/result.json").read_text()))]
            ),
        )

    def test_changed_failure_comments(self):
        (action,) = self.found["openSUSE Leap 16.0"]
        self.assertEqual((action["action"], action["number"]), ("update", 16))
        self.assertIn("`window`", action["comment"])

    def test_end_of_life_failures(self):
        self.assertEqual([action["action"] for action in self.found["Fedora 42"]], ["notice"])
        self.assertEqual([(action["action"], action.get("number")) for action in self.found["Fedora 41"]],
                         [("notice", None), ("close", 9)])  # fmt: skip


class EndToEnd(unittest.TestCase):
    """The release gate on a made-up run of two targets, then issues.py on its coverage.json."""

    def test_gate_then_issues(self):
        work = Path(tempfile.mkdtemp(prefix="gnoblin-issues-test-"))
        tag = "commit-0123456789ab"
        target = {"name": "Fedora 44", "slug": "fc44", "release": "44", "kind": "rpm", "lane": "fedora",
                  "published": True, "experimental": False, "prefix": "own", "install_artifact": "packages",
                  "install_suffix": "-fc44"}  # fmt: skip
        matrix = [{**target, "arch": "amd64"}, {**target, "arch": "arm64"}]
        rows = [{"name": "Fedora 44", "state": "built", "note": "", "eol": "2027-06-02"},
                {"name": "Debian 12", "state": "planned", "note": "GLib 2.74.6 is below the build floors",
                 "eol": "2028-06-30"}]  # fmt: skip
        (work / "targets.json").write_text(json.dumps({
            "matrix": matrix, "rows": rows, "architectures": [{"arch": "amd64"}, {"arch": "arm64"}],
            "graphical_packaged": [{"name": "Fedora 44", "slug": "fc44", "arch": arch} for arch in ("amd64", "arm64")],
        }))  # fmt: skip
        run = work / "run"
        layers = {name: {"status": "pass", "cases": []} for name in ("static", "install", "runtime", "repository")}
        for arch in ("amd64", "arm64"):
            folder = run / f"fc44-{arch}"
            for stage in ("prefix", "packages"):
                (folder / stage).mkdir(parents=True)
                record = {"stage": stage, "slug": "fc44", "arch": arch, "status": "pass", "detail": ""}
                (folder / stage / "stage.json").write_text(json.dumps(record))
            (folder / "package-tests").mkdir()
            (folder / "package-tests/report.json").write_text(json.dumps({"target": matrix[0] | {"arch": arch},
                                                                          "layers": layers}))  # fmt: skip
            (folder / f"packages-{tag}-fc44-{arch}").mkdir()
        (run / "fc44-amd64/graphical").mkdir()
        (run / "fc44-amd64/graphical/result.json").write_text(
            json.dumps({"slug": "fc44", "arch": "amd64", "result": "pass", "checks": []})
        )
        # arm64: the graphical job ran out of time and left a result with the deadline.
        (run / "fc44-arm64/graphical").mkdir()
        (run / "fc44-arm64/graphical/result.json").write_text(json.dumps({
            "slug": "fc44", "arch": "arm64", "result": "fail",
            "first_failure": {"name": "install", "detail": "the run reached its deadline of 19800 s"},
        }))  # fmt: skip
        artifacts = work / "artifacts"
        for arch, cpu in (("amd64", "x86_64"), ("arm64", "aarch64")):
            (artifacts / f"packages-{tag}-fc44-{arch}").mkdir(parents=True)
            (artifacts / f"packages-{tag}-fc44-{arch}" / f"gnoblin-0.1.13-1.fc44.{cpu}.rpm").write_text("x")
        gate = subprocess.run(
            [sys.executable, ROOT / "tests/packages/gate.py", "--targets", work / "targets.json", "--stages", run,
             "--reports", run, "--graphical", run, "--artifacts", artifacts, "--release-tag", tag, "--out",
             work / "coverage", "--today", "2026-11-03"],
            capture_output=True, text=True,
        )  # fmt: skip
        self.assertEqual(gate.returncode, 1, gate.stdout + gate.stderr)
        coverage = json.loads((work / "coverage/coverage.json").read_text())
        self.assertEqual(coverage["publish_set"], [f"packages-{tag}-fc44-amd64/gnoblin-0.1.13-1.fc44.x86_64.rpm"])
        (work / "open.json").write_text("[]")
        result = subprocess.run(
            [sys.executable, HERE / "issues.py", "--coverage", work / "coverage/coverage.json", "--issues-json",
             work / "open.json", "--dry-run", "--run-url", RUN, "--out", work / "issues"],
            capture_output=True, text=True,
        )  # fmt: skip
        self.assertEqual(result.returncode, 0, result.stderr)
        actions = json.loads((work / "issues/actions.json").read_text())
        self.assertEqual([(action["action"], action["release"]) for action in actions],
                         [("create", "Fedora 44"), ("create", "Debian 12")])  # fmt: skip
        self.assertIn("| arm64 | graphical | `install` | the run reached its deadline of 19800 s |", actions[0]["body"])
        self.assertIn("would create a new issue: Release pipeline: Fedora 44", result.stdout)
        self.assertIn("(dry run: no issue was changed)", result.stdout)
        shutil.rmtree(work)


if __name__ == "__main__":
    unittest.main()

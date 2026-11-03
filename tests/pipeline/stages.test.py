#!/usr/bin/env python3
"""Check tests/pipeline/stages.py, which turns the job results of a target leg into the stage records of the release
gate. No network."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def target(slug="fc43", lane="fedora", prefix="own", prefix_artifact="prefix"):
    return {"slug": slug, "arch": "amd64", "lane": lane, "prefix": prefix, "prefix_artifact": prefix_artifact}


def run(*arguments):
    out = Path(tempfile.mkdtemp(prefix="gnoblin-stages-test-"))
    subprocess.run([sys.executable, HERE / "stages.py", *arguments, "--out", out], check=True, capture_output=True)
    return {str(path.relative_to(out).parent): json.loads(path.read_text()) for path in sorted(out.rglob("stage.json"))}


class Needs(unittest.TestCase):
    def needs(self, leg, **jobs):
        return run("needs", "--target", json.dumps(leg), "--needs", json.dumps(jobs))

    def test_an_experimental_job_that_failed_is_a_failure_although_its_result_is_success(self):
        # continue-on-error makes needs.prefix.result "success"; the job's own status output says failure.
        records = self.needs(target(), prefix={"result": "success", "outputs": {"status": "failure"}},
                             packages={"result": "skipped", "outputs": {}})  # fmt: skip
        self.assertEqual(records["prefix"]["status"], "fail")
        self.assertEqual((records["packages"]["status"], records["packages"]["detail"]),
                         ("not run", "the packages job did not run"))  # fmt: skip

    def test_passed_jobs_pass(self):
        records = self.needs(target(), prefix={"result": "success", "outputs": {"status": "success"}},
                             packages={"result": "success", "outputs": {"status": "success"}})  # fmt: skip
        self.assertEqual({name: record["status"] for name, record in records.items()},
                         {"prefix": "pass", "packages": "pass"})  # fmt: skip

    def test_a_job_that_ended_before_its_last_step_is_a_failure(self):
        records = self.needs(target(), prefix={"result": "cancelled", "outputs": {}}, packages={"result": "skipped"})
        self.assertEqual(records["prefix"]["status"], "fail")
        self.assertIn("ended before it recorded a status (cancelled)", records["prefix"]["detail"])

    def test_a_leg_writes_only_the_stages_it_runs(self):
        # A shared prefix comes from release.yml, and a target without a lane installs the shared packages.
        shared_prefix = self.needs(target(slug="fc44", prefix_artifact="prefix-shared"),
                                   packages={"result": "success", "outputs": {"status": "success"}})  # fmt: skip
        self.assertEqual(set(shared_prefix), {"packages"})
        self.assertEqual(self.needs(target(slug="arch", lane="", prefix="shared", prefix_artifact="")), {})


class Shared(unittest.TestCase):
    def test_the_shared_packages_job_records_every_target_without_a_lane_on_its_architecture(self):
        matrix = [
            target(),
            target(slug="arch", lane="", prefix="shared"),
            {**target(slug="arch", lane=""), "arch": "arm64"},
        ]
        records = run("shared", "--matrix", json.dumps(matrix), "--arch", "amd64", "--job-status", "failure")
        self.assertEqual(list(records), ["arch-amd64/packages"])
        self.assertEqual(records["arch-amd64/packages"]["status"], "fail")


if __name__ == "__main__":
    unittest.main()

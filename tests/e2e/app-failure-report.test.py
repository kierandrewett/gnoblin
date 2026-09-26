#!/usr/bin/env python3
"""Behavioral check for the E2E failure report consumed by the CI issue reporter."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
REPORTER = ROOT / "scripts/report-app-e2e-failures.py"


class AppFailureReportTests(unittest.TestCase):
    def test_report_lists_failed_apps_and_omits_exercised_apps(self):
        with tempfile.TemporaryDirectory(prefix="gnoblin-app-report-", dir="/var/tmp") as raw:
            artifact_root = Path(raw) / "artifacts"
            shard = artifact_root / "gnoblin-app-e2e-shard-7" / "summary.json"
            shard.parent.mkdir(parents=True)
            shard.write_text(
                json.dumps(
                    {
                        "shard": {"index": 7, "count": 40},
                        "requested_apps": 3,
                        "outcomes": {"control-failed": 1, "exercised": 1, "no-window": 1},
                        "apps": [
                            {"app_id": "org.example.Healthy", "status": "exercised"},
                            {
                                "app_id": "org.example.NoWindow",
                                "status": "no-window",
                                "error": "timed out waiting for a window at /home/runner/work/gnoblin/gnoblin/install/bin/gtk-launch",
                            },
                            {"app_id": "org.example.Control", "status": "control-failed"},
                        ],
                    }
                )
            )
            report = Path(raw) / "report.md"
            env = os.environ | {
                "GITHUB_RUN_ID": "77",
                "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_SHA": "abc123",
                "GITHUB_REPOSITORY": "kierandrewett/gnoblin",
                "GITHUB_SERVER_URL": "https://github.com",
            }
            subprocess.run(
                [sys.executable, str(REPORTER), "--artifact-root", str(artifact_root), "--output", str(report)],
                check=True,
                env=env,
            )

            content = report.read_text()
            self.assertIn("Application compatibility triage", content)
            self.assertIn("https://github.com/kierandrewett/gnoblin/actions/runs/77", content)
            self.assertIn("https://github.com/kierandrewett/gnoblin/commit/abc123", content)
            self.assertIn("gnoblin-app-e2e-shard-7", content)
            self.assertIn("org.example.NoWindow", content)
            self.assertIn("org.example.Control", content)
            self.assertNotIn("org.example.Healthy", content)
            self.assertIn("1 `control-failed`", content)
            self.assertIn("1 `no-window`", content)
            self.assertIn("[path]", content)
            self.assertNotIn("/home/runner", content)

    def test_missing_summaries_report_infrastructure_failure(self):
        with tempfile.TemporaryDirectory(prefix="gnoblin-app-report-", dir="/var/tmp") as raw:
            artifact_root = Path(raw) / "artifacts"
            artifact_root.mkdir()
            (artifact_root / "gnoblin-app-e2e-shard-3-77-1").mkdir()
            report = Path(raw) / "report.md"
            subprocess.run(
                [sys.executable, str(REPORTER), "--artifact-root", str(artifact_root), "--output", str(report)],
                check=True,
            )

            content = report.read_text()
            self.assertIn("gnoblin-app-e2e-shard-3-77-1", content)
            self.assertIn("No `summary.json` artifact was available", content)
            self.assertIn("its job logs", content)


if __name__ == "__main__":
    unittest.main()

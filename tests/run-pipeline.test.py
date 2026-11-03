#!/usr/bin/env python3
"""Check the stage logic and the matrix of scripts/run-pipeline.py on a fixture target list, with stub stages.

The failure modes this guards against:
  - a stage runs, or keeps an old result file, after a stage before it failed, so a target could pass with a
    broken build
  - the matrix shows pass for a stage that has a record but no result file that the gate reads
  - a restart runs a stage again whose result exists, or keeps a result when a stage before it ran again
  - the driver writes no stage.json for a failed build stage, so the gate would see "not run" and raise no issue
  - a target of another architecture is run, or not listed
"""

import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("run_pipeline", ROOT / "scripts/run-pipeline.py")
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)
OTHER = "arm64" if driver.HOST_ARCH != "arm64" else "amd64"


def entry(slug, arch, lane=True, shared_prefix=False, consumer=False):
    base = {
        "name": f"System {slug}",
        "family": "System",
        "release": slug,
        "slug": slug,
        "kind": "rpm",
        "arch": arch,
        "experimental": True,
        "prefix": "shared" if consumer else "own",
        "prefix_image": "image:1",
        "prefix_artifact": "prefix-shared" if shared_prefix else "prefix",
        "bootstrap": "true",
        "deps_script": "deps.sh",
        "lane": "fedora" if lane else "",
        "package_args": "--lanes fedora",
        "packages_artifact": "packages",
        "install_image": "image:1",
        "install_prepare": "",
        "install_artifact": "packages",
        "install_suffix": f"-{slug}" if lane else "",
    }
    return base


def fixture():
    good, bad, base, user = (
        entry("good", driver.HOST_ARCH),
        entry("bad", driver.HOST_ARCH),
        entry("base", driver.HOST_ARCH, lane=False, shared_prefix=True),
        entry("user", driver.HOST_ARCH, lane=False, consumer=True),
    )
    far = entry("good", OTHER)
    data = {
        "matrix": [good, bad, base, user, far],
        "prefixes": [good, bad, base, far],
        "packages": [good, bad, far],
        "graphical": [
            {"name": n, "slug": s, "packaged": True, "eol": False, "eol_date": "2099-01-01"}
            for n, s in (("System good", "good"), ("System bad", "bad"), ("System user", "user"))
        ],
        "rows": [{"slug": s, "state": "experimental", "eol": "2099-01-01"} for s in ("good", "bad", "user", "base")],
    }
    return data


class Stub(driver.Pipeline):
    """The pipeline with stages that write the files a real stage writes, and fail for the target named "bad"."""

    def __init__(self, args, data, fail=("bad", "prefix")):
        super().__init__(args, data)
        self.fail = fail
        self.calls = []

    def outcome(self, slug, stage):
        self.calls.append((slug, stage))
        return driver.FAIL if (slug, stage) == self.fail else driver.PASS

    def build_prefix(self, target, priority):
        archive = self.root / "prefixes" / target.prefix_name / "prefix.tar.zst"
        archive.parent.mkdir(parents=True, exist_ok=True)
        archive.write_text("prefix")
        status = self.outcome(target.slug, "prefix")
        return status, "" if status == driver.PASS else "build-prefix.sh exited 1", ""

    def write_packages(self, target):
        return self.outcome(target.slug, "packages"), "", ""

    def write_shared(self, arch, providers):
        return self.outcome("shared", "packages"), "", ""

    def package_suite(self, target, priority):
        status = self.outcome(target.slug, "suite")
        layers = {
            name: {"status": status, "cases": [{"id": "S1", "title": "t", "status": status}]} for name in driver.LAYERS
        }
        driver.write_json(self.dir(target.key) / "package-tests" / "report.json", {"layers": layers, "result": status})
        return status, "", ""

    def graphical_test(self, target, priority):
        status = self.outcome(target.slug, "graphical")
        driver.write_json(self.dir(target.key) / "graphical" / "result.json", {"result": status, "first_failure": None})
        return status, "", ""


def arguments(root, **changes):
    values = dict(
        root=str(root), only="", jobs=2, vms=2, prefix_jobs=0, vm_memory=1024, min_free_gb=0.0, min_memory_gb=0.0,
        force=False, rerun=set(), no_graphical=False, release_tag="local", pkgsmith_ref="", work=str(root / "scratch"),
    )  # fmt: skip
    values.update(changes)
    return argparse.Namespace(**values)


def run(root, data, **changes):
    fail = changes.pop("fail", ("bad", "prefix"))
    pipeline = Stub(arguments(root, **changes), data, fail)
    pipeline.monitor = lambda: None
    pipeline.run_all()
    return pipeline


def matrix_rows(root, data):
    text = driver.matrix(root, data, "2026-01-01")
    return {
        line.split("|")[1].strip() + "/" + line.split("|")[2].strip(): line
        for line in text.splitlines()
        if line.startswith("| System")
    }


def assert_blocked_after_failure(test, root):
    """A failed prefix leaves the later stages without result files and the matrix says why."""
    test.assertEqual(json.loads((root / f"bad-{driver.HOST_ARCH}/prefix/stage.json").read_text())["status"], "fail")
    for path in ("packages/stage.json", "package-tests/report.json", "graphical/result.json"):
        test.assertFalse((root / f"bad-{driver.HOST_ARCH}" / path).exists(), path)
    verdict = json.loads((root / f"bad-{driver.HOST_ARCH}/verdict.json").read_text())
    test.assertEqual(verdict["stages"]["graphical"]["detail"], "prefix failed")
    test.assertEqual(verdict["verdict"], "fail: prefix")


class StageLogic(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.data = fixture()

    def tearDown(self):
        self.directory.cleanup()

    def test_a_failed_stage_stops_the_later_ones(self):
        pipeline = run(self.root, self.data)
        self.assertNotIn(("bad", "packages"), pipeline.calls)
        self.assertNotIn(("bad", "graphical"), pipeline.calls)
        assert_blocked_after_failure(self, self.root)
        rows = matrix_rows(self.root, self.data)
        bad = rows[f"System bad/{driver.HOST_ARCH}"]
        self.assertIn("**FAIL**", bad)
        self.assertIn("not run: prefix failed", bad)
        self.assertNotIn("| pass", bad)
        good = rows[f"System good/{driver.HOST_ARCH}"]
        self.assertEqual(good.split("|")[13].strip(), "pass", good)

    def test_the_check_catches_a_driver_that_does_not_stop(self):
        """Negative proof: with the stage gate broken, the assertion above fails."""
        original = driver.Pipeline.not_run

        def run_anyway(pipeline, target, stage, reason):
            function = {"packages": pipeline.write_packages, "suite": pipeline.package_suite,
                        "graphical": pipeline.graphical_test}[stage]  # fmt: skip
            args = (target,) if stage == "packages" else (target, 0)
            return pipeline.stage(target.key, stage, function, True, *args)

        driver.Pipeline.not_run = run_anyway
        try:
            run(self.root, self.data)
            with self.assertRaises(AssertionError):
                assert_blocked_after_failure(self, self.root)
        finally:
            driver.Pipeline.not_run = original

    def test_a_shared_prefix_failure_blocks_its_consumers(self):
        run(self.root, self.data, fail=("base", "prefix"))
        user = self.root / f"user-{driver.HOST_ARCH}"
        self.assertFalse((user / "packages/stage.json").exists())
        self.assertFalse((user / "package-tests/report.json").exists())
        self.assertIn("shared prefix base failed", (user / "verdict.json").read_text())

    def test_a_shared_package_failure_is_written_for_each_consumer(self):
        run(self.root, self.data, fail=("shared", "packages"))
        stage = json.loads((self.root / f"user-{driver.HOST_ARCH}/packages/stage.json").read_text())
        self.assertEqual((stage["slug"], stage["status"]), ("user", "fail"))

    def test_a_restart_keeps_results_and_reruns_after_a_change(self):
        run(self.root, self.data)
        again = run(self.root, self.data)
        self.assertEqual(again.calls, [])
        rerun = run(self.root, self.data, rerun={"packages"}, only="good")
        self.assertEqual(sorted(rerun.calls), [("good", "graphical"), ("good", "packages"), ("good", "suite")])

    def test_no_pass_without_a_result_file(self):
        run(self.root, self.data)
        (self.root / f"good-{driver.HOST_ARCH}/graphical/result.json").unlink()
        (self.root / f"good-{driver.HOST_ARCH}/package-tests/report.json").unlink()
        good = matrix_rows(self.root, self.data)[f"System good/{driver.HOST_ARCH}"]
        cells = [cell.strip() for cell in good.split("|")]
        self.assertNotEqual(cells[12], "pass")
        self.assertTrue(all(cell == "not run" for cell in cells[8:12]), cells)
        self.assertNotEqual(cells[13], "pass")

    def test_other_architectures_are_listed_and_not_run(self):
        pipeline = run(self.root, self.data)
        self.assertFalse((self.root / f"good-{OTHER}").exists())
        self.assertEqual(pipeline.calls.count(("good", "prefix")), 1)
        row = matrix_rows(self.root, self.data)[f"System good/{OTHER}"]
        self.assertIn(f"not run: no {OTHER} runner here", row)


if __name__ == "__main__":
    unittest.main()

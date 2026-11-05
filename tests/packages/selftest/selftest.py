#!/usr/bin/env python3
"""Self test of the package suite. It needs no network and no container.

usage: python3 tests/packages/selftest/selftest.py

1. The release gate (tests/packages/gate.py) on made-up runs: every stage passes; a prefix build fails; a package
   suite layer fails; only the graphical test fails; an experimental target fails; an arm64 target has no arm64
   graphical result or no arm64 graphical job; a planned release; a suite that wrote no report; a per-release deb in a
   published artifact. The test checks the exact verdicts, the exclusions, the publish set and the exit status.
2. The static layer and the mutations on a small package set that the pinned pkgsmith writes from the fixture prefix
   in this folder (fixture.star): a good set passes the format cases, and a broken copy fails the case it must. Part 2
   is skipped, with the reason, when the pinned pkgsmith is not built (scripts/install-pkgsmith.sh).
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SUITE = HERE.parent
ROOT = SUITE.parent.parent
sys.path.insert(0, str(SUITE))

import mutate  # noqa: E402
import report  # noqa: E402
import static  # noqa: E402
import targets  # noqa: E402

TAG = "commit-0123456789ab"


def entry(name, slug, release, kind, arch, lane, published, experimental, prefix="own"):
    own = bool(lane)
    artifact = "packages" if published or not own else "testpkgs"
    return {
        "name": name,
        "slug": slug,
        "release": release,
        "kind": kind,
        "arch": arch,
        "lane": lane,
        "published": published,
        "experimental": experimental,
        "prefix": prefix,
        "install_artifact": artifact,
        "install_suffix": f"-{slug}" if own else "",
        "install_image": "none",
        "install_prepare": "",
        "package_args": "",
    }


def stage(name, slug, arch, status="pass", detail=""):
    return {"stage": name, "slug": slug, "arch": arch, "status": status, "detail": detail}


def graphical(slug, arch, result="pass", check="", detail=""):
    data = {"slug": slug, "arch": arch, "result": result, "checks": [{"name": "boot", "status": "pass"}]}
    if result == "fail":
        data["first_failure"] = {"name": check, "detail": detail}
    return data


def run_gate(matrix, rows, artifacts, stages, reports, results, graphical_jobs=None, no_image=()):
    """Run the gate. stages: stage.json records; reports: {(slug, arch): layers}; results: graphical result.json
    records; graphical_jobs: the (slug, arch) pairs with a graphical job, every target by default; no_image: the pairs
    whose graphical entry has no image yet."""
    work = Path(tempfile.mkdtemp(prefix="gnoblin-gate-test-"))
    jobs = graphical_jobs if graphical_jobs is not None else [(item["slug"], item["arch"]) for item in matrix]
    missing = {"image": None, "image_missing": "none yet: build one", "guest": {"cpu": "aarch64"}}
    (work / "targets.json").write_text(
        json.dumps(
            {
                "matrix": matrix,
                "rows": rows,
                "architectures": [{"arch": "amd64"}, {"arch": "arm64"}],
                "graphical_packaged": [
                    {
                        "name": item["name"],
                        "slug": item["slug"],
                        "arch": item["arch"],
                        **(missing if (item["slug"], item["arch"]) in no_image else {}),
                    }
                    for item in matrix
                    if (item["slug"], item["arch"]) in jobs
                ],  # fmt: skip
            }
        )
    )
    for directory, files in artifacts.items():
        (work / "artifacts" / directory).mkdir(parents=True)
        for name in files:
            (work / "artifacts" / directory / name).write_text(name)
    # One folder for each target, as the verdict artifacts are laid out.
    run = work / "run"
    run.mkdir()
    for record in stages:
        folder = run / f"{record['slug']}-{record['arch']}" / record["stage"]
        folder.mkdir(parents=True)
        (folder / "stage.json").write_text(json.dumps(record))
    for (slug, arch), layers_of in reports.items():
        target = next(item for item in matrix if item["slug"] == slug and item["arch"] == arch)
        folder = run / f"{slug}-{arch}" / "package-tests"
        folder.mkdir(parents=True)
        data = {"target": target, "suite_layers": list(report.LAYERS), "layers": layers_of}
        (folder / "report.json").write_text(json.dumps(data))
    for result in results:
        folder = run / f"{result['slug']}-{result['arch']}" / "graphical"
        folder.mkdir(parents=True)
        (folder / "result.json").write_text(json.dumps(result))
    arguments = ["--stages", run, "--reports", run, "--graphical", run, "--artifacts", work / "artifacts"]
    result = subprocess.run(
        [sys.executable, str(SUITE / "gate.py"), "--targets", str(work / "targets.json"),
         *map(str, arguments), "--release-tag", TAG, "--out", str(work / "out"), "--today", "2026-11-03"],
        capture_output=True,
        text=True,
    )  # fmt: skip
    coverage = json.loads((work / "out/coverage.json").read_text())
    shutil.rmtree(work)
    return result.returncode, coverage, result.stdout


def layers(**states):
    result = {}
    for name in report.LAYERS:
        state = states.get(name, "pass")
        if state == "absent":
            continue
        cases = [{"id": "X1", "status": "fail" if state == "fail" else "pass"}] if state != "missing" else []
        result[name] = {"layer": name, "status": state if state != "absent" else "missing", "cases": cases}
    return result


ARCH_FILES = {
    "amd64": ["gnoblin-1.0.1.13-1-x86_64.pkg.tar.zst", "gnoblin-ananicy-rules-0.1.13-1-any.pkg.tar.zst"],
    "arm64": ["gnoblin-1.0.1.13-1-aarch64.pkg.tar.zst", "gnoblin-ananicy-rules-0.1.13-1-any.pkg.tar.zst"],
}
ARCH_AMD64_ONLY = f"packages-{TAG}-amd64/gnoblin-1.0.1.13-1-x86_64.pkg.tar.zst"
FEDORA45 = f"packages-{TAG}-fc45-amd64/gnoblin-0.1.13-1.fc45.x86_64.rpm"


class Gate(unittest.TestCase):
    matrix = [
        entry("Arch Linux", "arch", "rolling", "pacman", "amd64", "", True, False, "shared"),
        entry("Arch Linux", "arch", "rolling", "pacman", "arm64", "", True, False, "shared"),
        entry("Fedora 43", "fc43", "43", "rpm", "amd64", "fedora", True, False),
        entry("Fedora 45", "fc45", "45", "rpm", "amd64", "fedora", True, True),
        entry("Debian 13", "deb13", "13", "deb", "amd64", "deb", False, True),
    ]
    rows = [
        {"name": "Arch Linux", "state": "built", "note": "", "eol": None},
        {"name": "Fedora 45", "state": "experimental", "note": "not out yet", "eol": "2027-12-08"},
        {"name": "Fedora 43", "state": "built", "note": "", "eol": "2026-12-09"},
        {"name": "Fedora 40", "state": "planned", "note": "GLib 2.80.5 is below the build floors", "eol": "2025-05-13"},
        {"name": "Debian 12", "state": "planned", "note": "GLib 2.74.6 is below the build floors", "eol": "2028-06-30"},
        {"name": "Debian 13", "state": "experimental", "note": "", "eol": "2030-06-30"},
    ]
    artifacts = {
        f"packages-{TAG}-amd64": ARCH_FILES["amd64"],
        f"packages-{TAG}-arm64": ARCH_FILES["arm64"],
        f"packages-{TAG}-fc43-amd64": ["gnoblin-0.1.13-1.fc43.x86_64.rpm"],
        f"packages-{TAG}-fc45-amd64": ["gnoblin-0.1.13-1.fc45.x86_64.rpm"],
        f"testpkgs-{TAG}-deb13-amd64": ["gnoblin_0.1.13-1~deb13_amd64.deb"],
    }

    def passing(self):
        """Every stage of every target passes: the stage records, the reports and the graphical results."""
        stages = []
        for item in self.matrix:
            if item["prefix"] == "own":
                stages.append(stage("prefix", item["slug"], item["arch"]))
            stages.append(stage("packages", item["slug"], item["arch"]))
        reports = {(item["slug"], item["arch"]): layers() for item in self.matrix}
        results = [graphical(item["slug"], item["arch"]) for item in self.matrix]
        return stages, reports, results

    def gate(self, stages, reports, results, **options):
        return run_gate(self.matrix, self.rows, self.artifacts, stages, reports, results, **options)

    @staticmethod
    def target(coverage, slug, arch):
        return next(item for item in coverage["targets"] if item["slug"] == slug and item["arch"] == arch)

    def test_everything_passes(self):
        status, coverage, _ = self.gate(*self.passing())
        self.assertEqual(status, 0, coverage["errors"])
        self.assertEqual(coverage["exclusions"], [])
        self.assertIn(FEDORA45, coverage["publish_set"])
        self.assertIn(ARCH_AMD64_ONLY, coverage["publish_set"])
        self.assertFalse([name for name in coverage["publish_set"] if "deb13" in name])
        arch = self.target(coverage, "arch", "amd64")
        self.assertEqual(arch["stages"]["prefix"]["status"], "shared")
        self.assertTrue(arch["passed"] and arch["published"])
        self.assertIn("Arch Linux (amd64, arm64)", coverage["published_targets"])

    def test_a_failed_prefix_build_runs_nothing_after_it_and_fails_the_run(self):
        stages, reports, results = self.passing()
        stages = [stage("prefix", "fc43", "amd64", "fail", "build-prefix.sh exited 1")] + [
            record for record in stages if (record["slug"], record["stage"]) != ("fc43", "prefix")
        ]
        status, coverage, output = self.gate(stages, reports, results)
        self.assertEqual(status, 1)
        fc43 = self.target(coverage, "fc43", "amd64")["stages"]
        self.assertEqual((fc43["prefix"]["status"], fc43["prefix"]["detail"]), ("fail", "build-prefix.sh exited 1"))
        # The packages, suite and graphical records say pass, and still count as not run: the prefix failed first.
        for name in ("packages", "suite", "graphical"):
            self.assertEqual((fc43[name]["status"], fc43[name]["detail"]), ("not run", "not run: prefix failed"))
        self.assertNotIn(f"packages-{TAG}-fc43-amd64/gnoblin-0.1.13-1.fc43.x86_64.rpm", coverage["publish_set"])
        self.assertIn("built target failed: Fedora 43 amd64: prefix: build-prefix.sh exited 1", coverage["errors"])
        self.assertIn("| Fedora 43 | amd64 | built | FAIL prefix: build-prefix.sh exited 1 | not run: prefix", output)

    def test_a_failed_package_suite_layer_excludes_the_target(self):
        stages, reports, results = self.passing()
        reports[("fc43", "amd64")] = layers(runtime="fail")
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual(status, 1)
        fc43 = self.target(coverage, "fc43", "amd64")["stages"]
        self.assertEqual((fc43["suite"]["status"], fc43["suite"]["check"]), ("fail", "runtime"))
        self.assertEqual(fc43["graphical"]["detail"], "not run: suite failed")
        self.assertIn("excluded: Fedora 43 amd64: suite: runtime: failed X1", coverage["exclusions"])

    def test_one_failed_built_target_holds_no_other_back(self):
        # Fedora 43 (built) fails its graphical test; every other target passes. The run fails (exit 1), and the
        # publish set holds exactly the files of the targets that passed: a failed target's file is never in it.
        stages, reports, results = self.passing()
        results = [item for item in results if item["slug"] != "fc43"]
        results.append(graphical("fc43", "amd64", "fail", "keyboard", "the client saw the keys []"))
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual(status, 1)
        expected = sorted(
            [f"{name}/{file}" for name, files in self.artifacts.items() if name.startswith("packages-") for file in files
             if "fc43" not in name]
        )  # fmt: skip
        self.assertEqual(coverage["publish_set"], expected)
        self.assertFalse([name for name in coverage["publish_set"] if "fc43" in name])
        self.assertEqual(coverage["left_out"], ["Fedora 43 amd64, graphical"])
        self.assertIn("Fedora 45 (amd64)", coverage["published_targets"])
        self.assertNotIn("Fedora 43 (amd64)", coverage["published_targets"])

    def test_an_empty_publish_set_fails_the_gate(self):
        stages, reports, results = self.passing()
        results = [graphical(item["slug"], item["arch"], "fail", "boot", "no SSH") for item in self.matrix]
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual((status, coverage["publish_set"]), (1, []))
        self.assertIn("the publish set is empty", "; ".join(coverage["errors"]))

    def test_a_graphical_failure_alone_never_publishes(self):
        stages, reports, results = self.passing()
        results = [item for item in results if item["slug"] != "fc45"]
        results.append(graphical("fc45", "amd64", "fail", "keyboard", "the client saw the keys []"))
        status, coverage, output = self.gate(stages, reports, results)
        # Fedora 45 is experimental: excluded, and the run does not fail.
        self.assertEqual(status, 0, coverage["errors"])
        self.assertNotIn(FEDORA45, coverage["publish_set"])
        fc45 = self.target(coverage, "fc45", "amd64")
        self.assertEqual(fc45["stages"]["suite"]["status"], "pass")
        self.assertEqual(
            (fc45["stages"]["graphical"]["status"], fc45["stages"]["graphical"]["check"]), ("fail", "keyboard")
        )
        self.assertFalse(fc45["published"])
        self.assertIn("excluded: Fedora 45 amd64: graphical: the client saw the keys []", output)

    def test_a_built_target_that_fails_the_graphical_test_fails_the_run(self):
        stages, reports, results = self.passing()
        results = [item for item in results if item["slug"] != "fc43"]
        results.append(graphical("fc43", "amd64", "fail", "session", "no compositor process"))
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual(status, 1)
        self.assertIn("built target failed: Fedora 43 amd64: graphical: no compositor process", coverage["errors"])

    def test_arm64_needs_its_own_graphical_result(self):
        stages, reports, results = self.passing()
        results = [item for item in results if item["arch"] != "arm64"]
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual(status, 1)
        arch = self.target(coverage, "arch", "arm64")
        self.assertEqual(arch["stages"]["suite"]["status"], "pass")
        self.assertEqual(
            (arch["stages"]["graphical"]["status"], arch["stages"]["graphical"]["check"]), ("fail", "no result")
        )
        # The aarch64 package is out, and so is the package that is the same file on both architectures. The amd64
        # graphical result does not stand in for arm64.
        self.assertEqual(
            [name for name in coverage["publish_set"] if name.startswith(f"packages-{TAG}-a") or "-arm64/" in name],
            [ARCH_AMD64_ONLY],
        )

    def test_an_arm64_target_without_a_graphical_image_is_an_unmet_requirement(self):
        # Arch Linux arm64: no aarch64 image yet. Its graphical job does not run, and a result left by a run by hand
        # does not stand in for the release's own run.
        stages, reports, results = self.passing()
        status, coverage, output = self.gate(stages, reports, results, no_image=[("arch", "arm64")])
        self.assertEqual(status, 1)
        graphical_stage = self.target(coverage, "arch", "arm64")["stages"]["graphical"]
        self.assertEqual(
            (graphical_stage["status"], graphical_stage["detail"]),
            ("not run", "not run: no aarch64 image for Arch Linux (none yet: build one)"),
        )
        self.assertIn(
            "unmet requirement of a built target: Arch Linux arm64: graphical: not run: no aarch64 image for Arch Linux"
            " (none yet: build one)",
            coverage["errors"],
        )
        self.assertFalse([name for name in coverage["publish_set"] if "-arm64/" in name or "-any." in name])
        self.assertIn(ARCH_AMD64_ONLY, coverage["publish_set"])
        self.assertIn("### Errors that fail the release", output)

    def test_arm64_without_a_graphical_job_is_not_run_and_not_published(self):
        stages, reports, results = self.passing()
        results = [item for item in results if item["arch"] != "arm64"]
        jobs = [(item["slug"], item["arch"]) for item in self.matrix if item["arch"] == "amd64"]
        status, coverage, _ = self.gate(stages, reports, results, graphical_jobs=jobs)
        self.assertEqual(status, 1)
        graphical_stage = self.target(coverage, "arch", "arm64")["stages"]["graphical"]
        self.assertEqual(graphical_stage["detail"], "not run: no graphical job on arm64")
        self.assertFalse([name for name in coverage["publish_set"] if "-arm64/" in name])

    def test_a_suite_that_wrote_no_report_is_a_failure_not_a_pass(self):
        stages, reports, results = self.passing()
        del reports[("fc45", "amd64")]
        status, coverage, _ = self.gate(stages, reports, results)
        self.assertEqual(status, 0)
        suite = self.target(coverage, "fc45", "amd64")["stages"]["suite"]
        self.assertEqual(
            (suite["status"], suite["detail"]), ("fail", "no report.json: the package suite did not finish")
        )
        self.assertNotIn(FEDORA45, coverage["publish_set"])

    def test_planned_releases_have_a_verdict_and_no_packages(self):
        status, coverage, output = self.gate(*self.passing())
        planned = {item["name"]: item for item in coverage["targets"] if item["state"] == "planned"}
        self.assertEqual(set(planned), {"Fedora 40", "Debian 12"})
        self.assertEqual(planned["Debian 12"]["stages"]["build"], {"status": "not built", "check": "build",
                                                                    "detail": "GLib 2.74.6 is below the build floors"})  # fmt: skip
        self.assertEqual((planned["Debian 12"]["eol"], planned["Fedora 40"]["eol"]), (False, True))
        self.assertFalse(any(item["published"] for item in planned.values()))
        self.assertIn("| Debian 12 | - | planned | not built |", output)
        self.assertIn("GLib 2.74.6 is below the build floors |", output)

    def test_per_release_deb_in_a_published_artifact(self):
        artifacts = dict(self.artifacts)
        artifacts[f"packages-{TAG}-amd64"] = [*ARCH_FILES["amd64"], "gnoblin_0.1.13-1~deb13_amd64.deb"]
        matrix = [
            *self.matrix,
            entry("Debian testing", "debian-testing", "rolling", "deb", "amd64", "", True, False, "shared"),
        ]
        stages, reports, results = self.passing()
        stages.append(stage("packages", "debian-testing", "amd64"))
        reports[("debian-testing", "amd64")] = layers()
        results.append(graphical("debian-testing", "amd64"))
        status, coverage, _ = run_gate(matrix, self.rows, artifacts, stages, reports, results)
        self.assertEqual(status, 1)
        self.assertTrue(any("(G3)" in error or "(G4)" in error for error in coverage["errors"]))


def pinned_pkgsmith() -> Path | None:
    ref = (ROOT / "scripts/pkgsmith.ref").read_text().strip()
    cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "gnoblin/pkgsmith" / ref / "bin"
    return cache if (cache / "pkgsmith").is_file() else None


@unittest.skipIf(pinned_pkgsmith() is None, "the pinned pkgsmith is not built: run scripts/install-pkgsmith.sh")
class StaticOnFixture(unittest.TestCase):
    target = entry("Fixture", "fixture", "rolling", "pacman", "amd64", "", True, False, "shared")

    @classmethod
    def setUpClass(cls):
        cls.work = Path(tempfile.mkdtemp(prefix="gnoblin-static-test-"))
        prefix = cls.work / "prefix"
        program = Path(shutil.which("true")).resolve()
        for path in (
            "usr/lib/gnoblin/bin/gnoblin",
            "usr/lib/gnoblin/libexec/xdg-desktop-portal-gnoblin",
        ):
            (prefix / path).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(program, prefix / path)
        (prefix / "usr/bin").mkdir(parents=True)
        os.symlink("/usr/lib/gnoblin/bin/gnoblin", prefix / "usr/bin/gnoblin")
        for path, text in {
            "usr/share/wayland-sessions/gnoblin.desktop": "[Desktop Entry]\nName=Gnoblin\nExec=/usr/lib/gnoblin/bin/gnoblin\nType=Application\n",
            "etc/geoclue/conf.d/50-gnoblin.conf": "[gnoblin]\nallowed=true\n",
            "etc/ananicy.d/99-gnoblin.rules": "{}\n",
        }.items():
            (prefix / path).parent.mkdir(parents=True, exist_ok=True)
            (prefix / path).write_text(text)
        environment = {
            **os.environ,
            "PATH": f"{pinned_pkgsmith()}:{os.environ['PATH']}",
            "SOURCE_DATE_EPOCH": "1700000000",
        }
        cls.dist = cls.work / "dist"
        subprocess.run(
            [
                "pkgsmith",
                "build",
                str(HERE),
                "-f",
                str(HERE / "fixture.star"),
                "--prefix",
                str(prefix),
                "-o",
                str(cls.dist),
                "-F",
                "archlinux,tar",
                "--progress",
                "plain",
            ],
            check=True,
            capture_output=True,
            env=environment,
        )
        for file in cls.dist.glob("*:*"):
            file.rename(file.with_name(file.name.replace(":", ".")))

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.work)

    def static(self, directory: Path) -> dict[tuple[str, str], str]:
        files = targets.select(self.target, directory)
        versions = {"gnoblin": "0.1.13", "portal": "51.0.0"}
        layer = static.run(self.target, files, self.work / f"scratch-{directory.name}", versions, "1", None)
        return {(case["id"], case["subject"]): case["status"] for case in layer.cases}

    def test_good_set(self):
        result = self.static(self.dist)
        for case_id in ("S1", "S4", "S5", "S6", "S7", "S12"):
            states = [status for (identity, _), status in result.items() if identity == case_id]
            self.assertTrue(states, case_id)
            self.assertTrue(all(status in ("pass", "skip") for status in states), (case_id, result))

    def mutated(self, name: str) -> dict[tuple[str, str], str]:
        copy = self.work / f"mutated-{name}"
        shutil.copytree(self.dist, copy)
        mutate.apply(name, copy)
        return self.static(copy)

    def test_foreign_file_fails_s4(self):
        result = self.mutated("foreign-file")
        self.assertIn("fail", [status for (identity, _), status in result.items() if identity == "S4"])

    def test_build_path_fails_s7(self):
        result = self.mutated("build-path")
        self.assertIn("fail", [status for (identity, _), status in result.items() if identity == "S7"])


if __name__ == "__main__":
    unittest.main()

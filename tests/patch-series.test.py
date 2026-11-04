#!/usr/bin/env python3
"""The patch series is the one source of patch order, and the checks catch a layout that disagrees with it."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import patch_series  # noqa: E402

PROJECT = "demo"
LAYOUT = [
    "01-alpha/0001-first.patch",
    "02-beta/0001-second.patch",
    "01-alpha/0002-third.patch",
]
HEADER = (
    "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001\nFrom: kierandrewett <kieran@drewett.dev>\n"
)


class PatchSeriesTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.base = self.root / "patches" / PROJECT
        for line in LAYOUT:
            self.write_patch(line)
        self.write_series(LAYOUT)
        for name, value in (("ROOT", self.root), ("PATCH_ROOT", self.root / "patches")):
            original = getattr(patch_series, name)
            setattr(patch_series, name, value)
            self.addCleanup(setattr, patch_series, name, original)

    def write_patch(self, line):
        path = self.base / line
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(HEADER + f"Subject: [PATCH] {line}\n")

    def write_series(self, lines):
        (self.base / "series").write_text("".join(f"{line}\n" for line in lines))

    def test_a_consistent_layout_has_no_problems_and_keeps_series_order(self):
        self.assertEqual(patch_series.problems(PROJECT), [])
        self.assertEqual([p.relative_to(self.base).as_posix() for p in patch_series.series(PROJECT)], LAYOUT)

    def test_a_patch_missing_from_the_series_is_reported(self):
        self.write_patch("02-beta/0002-unlisted.patch")
        self.assertEqual(
            patch_series.problems(PROJECT),
            ["patches/demo/02-beta/0002-unlisted.patch: not listed in patches/demo/series"],
        )

    def test_a_series_entry_without_a_file_is_reported(self):
        self.write_series([*LAYOUT, "02-beta/0002-ghost.patch"])
        self.assertEqual(
            patch_series.problems(PROJECT),
            ["patches/demo/series: lists 02-beta/0002-ghost.patch, which does not exist"],
        )

    def test_a_duplicate_entry_is_reported(self):
        self.write_series([*LAYOUT, LAYOUT[0]])
        self.assertTrue(any("listed twice" in problem for problem in patch_series.problems(PROJECT)))

    def test_a_missing_series_file_is_reported(self):
        (self.base / "series").unlink()
        self.assertTrue(any("missing" in problem for problem in patch_series.problems(PROJECT)))

    def test_blank_lines_and_comments_are_rejected(self):
        for text in ("\n".join(LAYOUT) + "\n\n", "# note\n" + "\n".join(LAYOUT) + "\n"):
            (self.base / "series").write_text(text)
            self.assertTrue(patch_series.problems(PROJECT), text)

    def test_gaps_and_duplicate_numbers_are_reported(self):
        for name in ("02-beta/0001-second.patch", "01-alpha/0002-third.patch"):
            (self.base / name).rename(self.base / name.replace("0001", "0003").replace("0002", "0004"))
        self.write_series(["01-alpha/0001-first.patch", "02-beta/0003-second.patch", "01-alpha/0004-third.patch"])
        problems = patch_series.problems(PROJECT)
        self.assertEqual(len(problems), 1)
        self.assertIn("02-beta/0003-second.patch should be 02-beta/0001-second.patch", problems[0])

    def test_renumber_rewrites_names_and_series_from_the_edited_series(self):
        (self.base / "01-alpha/0002-third.patch").rename(self.base / "01-alpha/0009-third.patch")
        self.write_series(["02-beta/0001-second.patch", "01-alpha/0001-first.patch", "01-alpha/0009-third.patch"])
        loader = importlib.util.spec_from_file_location("manage_patches", ROOT / "scripts/manage-patches.py")
        module = importlib.util.module_from_spec(loader)
        loader.loader.exec_module(module)
        module.renumber(PROJECT)
        self.assertEqual(
            (self.base / "series").read_text().splitlines(),
            ["01-beta/0001-second.patch", "02-alpha/0001-first.patch", "02-alpha/0002-third.patch"],
        )
        self.assertEqual(patch_series.problems(PROJECT), [])
        self.assertEqual(
            sorted(p.parent.name for p in self.base.glob("*/*.patch")), ["01-beta", "02-alpha", "02-alpha"]
        )

    def test_find_and_topic_resolve_by_slug_and_topic(self):
        self.assertEqual(patch_series.find(PROJECT, "second"), self.base / "02-beta/0001-second.patch")
        self.assertEqual(
            patch_series.topic(PROJECT, "alpha"),
            [self.base / "01-alpha/0001-first.patch", self.base / "01-alpha/0002-third.patch"],
        )
        with self.assertRaises(patch_series.SeriesError):
            patch_series.find(PROJECT, "absent")

    def test_repository_series_agree_with_their_files(self):
        for name in ("mutter", "xdg-desktop-portal-gnome", "portal-dependencies"):
            with self.subTest(project=name):
                patch_series.ROOT, patch_series.PATCH_ROOT = ROOT, ROOT / "patches"
                self.assertEqual(patch_series.problems(name), [])

    def test_manage_patches_check_fails_loudly_for_an_unlisted_patch(self):
        (self.root / "scripts").mkdir()
        for name in ("manage-patches.py", "patch_series.py"):
            (self.root / "scripts" / name).write_text((ROOT / "scripts" / name).read_text())
        for project in patch_series.PROJECTS:
            self.write_patch_in(project, "01-topic/0001-only.patch")
            (self.root / "patches" / project / "series").write_text("01-topic/0001-only.patch\n")
        command = [sys.executable, str(self.root / "scripts/manage-patches.py"), "check"]
        self.assertEqual(subprocess.run(command, capture_output=True, text=True).returncode, 0)
        self.write_patch_in("mutter", "01-topic/0002-unlisted.patch")
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("patches/mutter/01-topic/0002-unlisted.patch: not listed in patches/mutter/series", result.stderr)

    def write_patch_in(self, project, line):
        path = self.root / "patches" / project / line
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(HEADER + "Subject: [PATCH] x\n")


if __name__ == "__main__":
    unittest.main()

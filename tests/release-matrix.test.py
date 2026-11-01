#!/usr/bin/env python3
"""Check the Fedora release matrix: the candidate list (scripts/update-release-matrix.py), the build matrix that the
workflows read from the inventory (scripts/fedora-releases.py) and the warnings where the two meet
(scripts/check-packaging-targets.py). No network: the sources are fixtures."""

import datetime
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    specification = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


matrix = load("update-release-matrix")
releases = load("fedora-releases")
inventory_check = load("check-packaging-targets")
TODAY = datetime.date(2026, 11, 1)


def cycle(release, released, eol):
    return {"cycle": release, "releaseDate": released, "eol": eol}


def bodhi(release, state, released, eol):
    return {"name": f"F{release}", "state": state, "released_on": released, "eol": eol}


# What the three sources said on 2026-11-01. endoflife.date does not list 45 and 46. Bodhi has no release date for 43.
ENDOFLIFE = [
    cycle("44", "2026-04-28", "2027-06-02"),
    cycle("43", "2025-10-28", "2026-12-09"),
    cycle("42", "2025-04-15", "2026-05-27"),
]
BODHI = [
    bodhi("46", "pending", None, None),
    bodhi("45", "current", "2026-11-03", "2027-12-08"),
    bodhi("44", "current", "2026-04-28", "2027-06-02"),
    bodhi("43", "current", None, "2026-12-02"),
    {"name": "F44C", "state": "current", "released_on": None, "eol": None},
]
IMAGES = {"42", "43", "44", "45", "46"}
PUBLISHED = {"42", "43", "44"}


def build(**changes):
    arguments = {"endoflife": ENDOFLIFE, "bodhi": BODHI, "images": IMAGES, "published": PUBLISHED}
    arguments.update(changes)
    return matrix.build(TODAY, arguments["endoflife"], arguments["bodhi"], arguments["images"], arguments["published"])


class Candidates(unittest.TestCase):
    def test_released_releases_and_the_pre_release_are_candidates(self):
        found = {item["release"]: item for item in build()["candidates"]}
        self.assertEqual(set(found), {"43", "44", "45"})
        self.assertEqual([item["prerelease"] for item in found.values()], [False, False, True])
        self.assertEqual(found["43"]["image"], "fedora:43")
        # Bodhi has no date for 43, so the date comes from endoflife.date and the end of life from Fedora.
        self.assertEqual((found["43"]["released"], found["43"]["eol"]), ("2025-10-28", "2026-12-02"))

    def test_rawhide_and_ended_releases_are_not_candidates_and_need_no_review(self):
        result = build()
        self.assertEqual(result["needs_review"], [])
        self.assertNotIn("46", {item["release"] for item in result["candidates"]})
        self.assertNotIn("42", {item["release"] for item in result["candidates"]})

    def test_only_the_newest_four_released_releases_are_kept(self):
        many = [cycle(str(n), "2020-01-01", "2030-01-01") for n in range(30, 45)]
        official = [bodhi(n, "current", "2020-01-01", "2030-01-01") for n in range(30, 45)]
        images = {str(n) for n in range(30, 45)}
        result = build(endoflife=many, bodhi=official, images=images, published=images)
        self.assertEqual([item["release"] for item in result["candidates"]], ["41", "42", "43", "44"])

    def test_a_tag_alone_does_not_make_a_pre_release(self):
        # Fedora says 46 is rawhide. A fedora:46 tag exists, and still it is not a candidate.
        self.assertNotIn("46", {item["release"] for item in build()["candidates"]})
        # Nothing official about a release that endoflife.date lists and whose end is ahead: review, not a candidate.
        result = build(bodhi=[entry for entry in BODHI if entry["name"] != "F44"])
        self.assertEqual([item["release"] for item in result["needs_review"]], ["44"])
        self.assertNotIn("44", {item["release"] for item in result["candidates"]})

    def test_disagreement_goes_to_review_with_the_three_observations(self):
        result = build(images=IMAGES - {"44"})
        self.assertNotIn("44", {item["release"] for item in result["candidates"]})
        (review,) = result["needs_review"]
        self.assertEqual(review["release"], "44")
        self.assertEqual(set(review["observations"]), {"endoflife_date", "official", "image"})
        self.assertIn("does not exist", review["observations"]["image"])
        # A published release that endoflife.date has not listed yet is also a disagreement.
        result = build(endoflife=[item for item in ENDOFLIFE if item["cycle"] != "44"])
        self.assertEqual([item["release"] for item in result["needs_review"]], ["44"])
        # A release that Fedora dates in the past but has not published.
        result = build(published=PUBLISHED - {"44"})
        self.assertEqual([item["release"] for item in result["needs_review"]], ["44"])

    def test_a_pre_release_that_endoflife_date_already_lists_needs_review(self):
        result = build(endoflife=ENDOFLIFE + [cycle("45", "2026-11-03", "2027-12-08")])
        self.assertEqual([item["release"] for item in result["needs_review"]], ["45"])

    def test_floors_decide_eligibility(self):
        with tempfile.TemporaryDirectory() as directory:
            floors = Path(directory)
            (floors / "fedora-44.json").write_text(json.dumps({"floors_met": True, "evidence": "all met"}))
            (floors / "fedora-43.txt").write_text(
                "PASS\tglib-2.0\t>= 2.81.1\t2.86.5\nFAIL\tgtk4\t>= 4.20.0\t4.18.0\nMISSING\tlibadwaita-1\t>= 1.7\t\n"
            )
            result = matrix.build(TODAY, ENDOFLIFE, BODHI, IMAGES, PUBLISHED, floors)
        found = {item["release"]: item for item in result["candidates"]}
        self.assertEqual((found["44"]["floors_met"], found["44"]["eligible"]), (True, True))
        self.assertEqual((found["43"]["floors_met"], found["43"]["eligible"]), (False, False))
        self.assertIn("2 of 3 pkg-config floors not met", found["43"]["evidence"])
        self.assertEqual((found["45"]["floors_met"], found["45"]["eligible"]), (None, False))

    def test_the_diff_lists_what_changed_and_is_empty_when_nothing_did(self):
        before = build()
        self.assertEqual(matrix.diff(before, build()), "")
        after = build(endoflife=ENDOFLIFE[:1], bodhi=[entry for entry in BODHI if entry["name"] != "F43"])
        text = matrix.diff(before, after)
        self.assertIn("No longer a candidate: Fedora 43", text)
        text = matrix.diff(before, build(images=IMAGES - {"44"}))
        self.assertIn("Needs review: Fedora 44", text)
        self.assertIn("image: fedora:44 does not exist", text)

    def test_the_candidate_file_is_what_the_script_writes(self):
        shipped = json.loads((ROOT / "scripts/release-candidates.json").read_text())
        self.assertEqual(shipped["schema_version"], 1)
        for item in shipped["candidates"]:
            self.assertEqual(item["image"], f"fedora:{item['release']}")
            self.assertEqual(item["eligible"], item["floors_met"] is True)


def inventory(*targets):
    return {"targets": list(targets)}


def target(release, status="existing-path", lifecycle="upstream-current", **extra):
    return {
        "id": f"fedora-{release}",
        "family": "fedora",
        "release": release,
        "image": f"fedora:{release}",
        "status": status,
        "lifecycle": lifecycle,
        **extra,
    }


class BuildMatrix(unittest.TestCase):
    def test_the_shipped_inventory_gives_a_well_formed_matrix(self):
        description = releases.describe(json.loads((ROOT / "scripts/package-targets.json").read_text()))
        numbers = [item["release"] for item in description["releases"]]
        self.assertEqual(numbers, sorted(numbers, key=int))
        self.assertIn(description["source"], numbers)
        source = next(item for item in description["releases"] if item["release"] == description["source"])
        self.assertFalse(source["prerelease"])
        self.assertEqual(source["image"], description["source_image"])

    def test_only_targets_that_may_be_built_are_in_the_matrix(self):
        description = releases.describe(
            inventory(
                target("43", shared_prefix=True),
                target("44", status="planned"),
                target("45", lifecycle="upstream-pre-release"),
            )
        )
        self.assertEqual(
            description["releases"],
            [
                {"release": "43", "image": "fedora:43", "prerelease": False},
                {"release": "45", "image": "fedora:45", "prerelease": True},
            ],
        )

    def test_exactly_one_release_makes_the_shared_prefix_and_it_is_not_a_pre_release(self):
        with self.assertRaises(ValueError):
            releases.describe(inventory(target("43"), target("44")))
        with self.assertRaises(ValueError):
            releases.describe(inventory(target("43", shared_prefix=True), target("44", shared_prefix=True)))
        with self.assertRaises(ValueError):
            releases.describe(inventory(target("45", lifecycle="upstream-pre-release", shared_prefix=True)))

    def test_the_image_names_the_release(self):
        broken = target("43", shared_prefix=True)
        broken["image"] = "fedora:44"
        with self.assertRaises(ValueError):
            releases.describe(inventory(broken))


class Warnings(unittest.TestCase):
    def candidates(self, *items, review=()):
        return {"candidates": list(items), "needs_review": list(review)}

    def item(self, release, prerelease=False, eligible=True, eol="2027-01-01"):
        return {
            "family": "fedora",
            "release": release,
            "prerelease": prerelease,
            "eligible": eligible,
            "eol": eol,
            "evidence": "evidence",
        }

    def test_a_candidate_without_an_entry_and_an_entry_past_its_end_are_warned_about(self):
        found = inventory_check.candidate_warnings(
            inventory(target("43"), target("44")),
            self.candidates(self.item("44"), self.item("45", prerelease=True)),
            TODAY,
        )
        self.assertEqual(len(found), 2)
        self.assertIn(
            "Fedora 45 is a release candidate that meets the build floors and has no inventory entry", found[0]
        )
        self.assertIn("Fedora 43 has an inventory entry but is no longer a release candidate", found[1])

    def test_the_end_of_life_and_the_lifecycle_are_checked(self):
        found = inventory_check.candidate_warnings(
            inventory(target("43"), target("44", lifecycle="upstream-pre-release")),
            self.candidates(self.item("43", eol="2026-10-01"), self.item("44")),
            TODAY,
        )
        self.assertEqual(len(found), 2)
        self.assertIn("past its end of life", " ".join(found))
        self.assertIn("Fedora 44 lifecycle is upstream-pre-release", " ".join(found))

    def test_a_clean_state_has_no_warnings(self):
        self.assertEqual(
            inventory_check.candidate_warnings(inventory(target("44")), self.candidates(self.item("44")), TODAY), []
        )


if __name__ == "__main__":
    unittest.main()

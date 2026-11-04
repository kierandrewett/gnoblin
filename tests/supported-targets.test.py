#!/usr/bin/env python3
"""Check the list of targets (scripts/supported-targets.py). No network: endoflife.date, Bodhi and the image check are
fixtures."""

import datetime
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
specification = importlib.util.spec_from_file_location("supported_targets", ROOT / "scripts/supported-targets.py")
targets = importlib.util.module_from_spec(specification)
specification.loader.exec_module(targets)

TODAY = datetime.date(2026, 11, 1)


def cycle(release, released, eol, lts=False):
    return {"cycle": release, "releaseDate": released, "eol": eol, "lts": lts}


def bodhi(release, state, released_on, eol="2027-12-08"):
    return {"name": f"F{release}", "state": state, "released_on": released_on, "eol": eol}


FEDORA = {
    "name": "Fedora",
    "product": "fedora",
    "image": "fedora:{release}",
    "slug": "fc{release}",
    "kind": "rpm",
    "prefix": "own",
    "bootstrap": "dnf -y install git-core",
    "deps_script": "install-fedora-build-deps.sh",
    "lane": "fedora",
    "published": True,
    "build_from": "43",
    "verified_from": "43",
    "verified_through": "44",
    "planned_reason": {
        release: f"GLib 2.{release} is below the build floors" for release in ("42", "41", "40", "36", "35")
    },
    "source_release": "44",
    "upcoming": "bodhi",
}
UBUNTU = {
    "name": "Ubuntu",
    "product": "ubuntu",
    "image": "ubuntu:{release}",
    "slug": "ubuntu{release}",
    "kind": "deb",
    "prefix": "own",
    "bootstrap": "apt-get install -y git",
    "deps_script": "install-debian-build-deps.sh",
    "lane": "deb",
    "lts_only": True,
    "build_from": "26.04",
    "verified_from": None,
    "verified_through": None,
    "planned_reason": {"24.04": "GLib 2.80.0 is below the build floors", "22.04": "no libei", "20.04": "no GTK 4"},
}
DEBIAN = {
    "name": "Debian",
    "product": "debian",
    "image": "debian:{release}",
    "slug": "deb{release}",
    "kind": "deb",
    "prefix": "own",
    "bootstrap": "apt-get install -y git",
    "deps_script": "install-debian-build-deps.sh",
    "lane": "deb",
    "build_from": None,
    "verified_from": None,
    "verified_through": None,
    "planned_reason": {release: "no native build in CI yet" for release in ("13", "12", "11", "10")},
}
LEAP = {
    "name": "openSUSE Leap",
    "product": "opensuse",
    "image": "leap:{release}",
    "slug": "leap{release}",
    "kind": "rpm",
    "prefix": "shared",
    "build_from": None,
    "verified_from": None,
    "verified_through": None,
    "planned_reason": {"16.0": "no native build in CI yet", "15.6": "no native build in CI yet"},
}
# An openSUSE Leap family with a prefix and an rpm lane of its own, and an Alpine family whose branches make their own apks.
LEAP_OWN = {
    **LEAP,
    "prefix": "own",
    "bootstrap": "zypper -n install git-core",
    "deps_script": "install-opensuse-build-deps.sh",
    "lane": "opensuse",
    "install_prepare": "zypper -n install update-alternatives",
    "build_from": "16.0",
    "planned_reason": {"15.6": "GLib 2.78.6 is below the build floors"},
}
ALPINE = {
    "name": "Alpine",
    "product": "alpine-linux",
    "image": "alpine:{release}",
    "slug": "alpine{release}",
    "kind": "apk",
    "prefix": "own",
    "bootstrap": "apk add bash git",
    "deps_script": "install-alpine-build-deps.sh",
    "lane": "other",
    "install_prepare": "apk add bash",
    "build_from": "3.22",
    "verified_from": None,
    "verified_through": None,
    "planned_reason": {"3.21": "GTK 4.16.13 is below the build floors"},
}
ARCH = {
    "name": "Arch Linux",
    "image": "archlinux:base",
    "arm64_image": "arm:base",
    "slug": "arch",
    "kind": "pacman",
    "prefix": "shared",
}
EDGE = {
    "name": "Alpine edge",
    "image": "alpine:edge",
    "slug": "alpine-edge",
    "kind": "apk",
    "prefix": "own",
    "bootstrap": "apk add git",
    "deps_script": "install-alpine-build-deps.sh",
    "install_prepare": "apk add bash",
    "shared_source": True,
}
POLICY = {
    "newest": 4,
    "window_years": 5,
    "architectures": [{"arch": "amd64", "runner": "x64"}, {"arch": "arm64", "runner": "a64"}],
    "families": [FEDORA, UBUNTU, DEBIAN, LEAP],
    "rolling": [ARCH, EDGE],
}
CYCLES = {
    "fedora": [
        cycle("44", "2026-04-28", "2027-06-02"),
        cycle("43", "2025-10-28", "2026-12-09"),
        cycle("42", "2025-04-15", "2026-05-27"),
        cycle("41", "2024-10-29", "2025-12-15"),
        cycle("40", "2024-04-23", "2025-05-13"),
        cycle("36", "2022-05-10", "2023-05-16"),
        cycle("35", "2021-11-02", "2022-12-13"),
        cycle("34", "2021-04-27", "2022-06-07"),
        cycle("33", "2020-10-27", "2021-11-30"),
    ],
    "ubuntu": [
        cycle("26.10", "2026-10-15", "2027-07-01"),
        cycle("26.04", "2026-04-23", "2031-05-29", lts=True),
        cycle("25.10", "2025-10-09", "2026-07-01"),
        cycle("24.04", "2024-04-25", "2029-05-31", lts=True),
        cycle("22.04", "2022-04-21", "2027-06-01", lts=True),
        cycle("20.04", "2020-04-23", "2025-05-31", lts=True),
        cycle("18.04", "2018-04-26", "2023-05-31", lts=True),
    ],
    "debian": [
        cycle("13", "2025-08-09", "2030-06-30"),
        cycle("12", "2023-06-10", "2028-06-30"),
        cycle("11", "2021-08-14", "2026-08-31"),
        cycle("10", "2019-07-06", "2024-06-30"),
        cycle("9", "2017-06-17", "2022-07-01"),
        cycle("forky", None, None),
    ],
    "opensuse": [cycle("16.0", "2025-10-01", "2027-10-31"), cycle("15.6", "2024-06-12", "2026-04-30")],
    "alpine-linux": [
        cycle("3.24", "2026-06-09", "2028-06-01"),
        cycle("3.23", "2025-12-04", "2027-11-01"),
        cycle("3.22", "2025-05-30", "2027-05-01"),
        cycle("3.21", "2024-12-05", "2026-11-01"),
    ],
}
BODHI = [bodhi("45", "current", "2026-11-03"), bodhi("46", "pending", "2027-04-21"), bodhi("43", "current", None)]


def compute(policy=POLICY, cycles=CYCLES, bodhi_releases=BODHI, today=TODAY, images=("fedora:45", "fedora:46")):
    return targets.compute(policy, cycles, bodhi_releases, lambda image: image in images, today)


def releases(result, family):
    return [row["release"] for row in result["rows"] if row["family"] == family]


def states(result, family):
    return {row["release"]: row["state"] for row in result["rows"] if row["family"] == family}


def entries(result, view="matrix", **fields):
    return [item for item in result[view] if all(item[key] == value for key, value in fields.items())]


def names(result, view="matrix", arch="amd64"):
    return [item["name"] for item in result[view] if item["arch"] == arch]


class Listing(unittest.TestCase):
    def test_a_release_is_listed_when_it_is_among_the_newest_four_or_in_the_window(self):
        # Window start is 2021-11-01: Fedora 35 (2021-11-02) is in, 34 is out. The newest four are 45, 44, 43, 42.
        self.assertEqual(releases(compute(), "Fedora"), ["45", "44", "43", "42", "41", "40", "36", "35"])

    def test_the_newest_four_are_listed_even_when_they_are_older_than_the_window(self):
        self.assertEqual(releases(compute(), "Debian"), ["13", "12", "11", "10"])

    def test_an_old_release_outside_the_newest_four_and_the_window_is_not_listed(self):
        self.assertNotIn("9", releases(compute(), "Debian"))
        self.assertNotIn("33", releases(compute(), "Fedora"))

    def test_the_window_moves_with_today(self):
        later = compute(today=datetime.date(2026, 11, 3))
        self.assertNotIn("35", releases(later, "Fedora"))
        earlier = compute(today=datetime.date(2026, 10, 30))
        self.assertIn("35", releases(earlier, "Fedora"))

    def test_the_window_comes_from_the_policy(self):
        short = compute({**POLICY, "window_years": 1})
        self.assertEqual(releases(short, "Fedora"), ["45", "44", "43", "42"])
        self.assertEqual(targets.years_before(datetime.date(2028, 2, 29), 1), datetime.date(2027, 2, 28))

    def test_ubuntu_lists_long_term_support_releases_only(self):
        self.assertEqual(releases(compute(), "Ubuntu"), ["26.04", "24.04", "22.04", "20.04"])

    def test_end_of_life_does_not_remove_a_release(self):
        rows = {row["release"]: row for row in compute()["rows"] if row["family"] == "Debian"}
        self.assertEqual(rows["10"]["eol"], "2024-06-30")
        self.assertEqual(rows["10"]["state"], targets.PLANNED)

    def test_a_release_that_is_not_listed_yet_needs_an_image_and_a_state_other_than_pending(self):
        self.assertNotIn("45", releases(compute(images=()), "Fedora"))
        self.assertNotIn("46", releases(compute(), "Fedora"))

    def test_bodhi_cannot_bring_back_a_release_that_endoflife_left_out(self):
        old = [bodhi("37", "current", "2022-11-15", eol="2027-01-01")]
        self.assertNotIn("37", releases(compute(bodhi_releases=old, images=("fedora:37",)), "Fedora"))

    def test_versions_compare_as_numbers(self):
        self.assertLess(targets.version_key("9"), targets.version_key("10"))
        self.assertLess(targets.version_key("26.4"), targets.version_key("26.10"))


class States(unittest.TestCase):
    def test_a_release_between_build_from_and_verified_through_is_built(self):
        self.assertEqual(states(compute(), "Fedora")["43"], targets.BUILT)
        self.assertEqual(states(compute(), "Fedora")["44"], targets.BUILT)

    def test_only_a_release_newer_than_the_verified_one_is_experimental(self):
        result = compute()
        self.assertEqual(states(result, "Fedora")["45"], targets.EXPERIMENTAL)
        experimental = [row["name"] for row in result["rows"] if row["state"] == targets.EXPERIMENTAL]
        self.assertEqual(experimental, ["Fedora 45", "Ubuntu 26.04"])

    def test_a_release_older_than_build_from_is_planned_with_its_own_reason(self):
        rows = {row["release"]: row for row in compute()["rows"] if row["family"] == "Fedora"}
        self.assertEqual(rows["42"]["state"], targets.PLANNED)
        self.assertEqual(rows["42"]["note"], "GLib 2.42 is below the build floors")
        self.assertEqual(rows["41"]["note"], "GLib 2.41 is below the build floors")

    def test_a_planned_release_without_a_reason_is_an_error(self):
        reasons = {key: value for key, value in FEDORA["planned_reason"].items() if key != "40"}
        with self.assertRaises(ValueError) as raised:
            compute({**POLICY, "families": [{**FEDORA, "planned_reason": reasons}]})
        self.assertIn("Fedora 40", str(raised.exception))

    def test_an_experimental_release_can_name_the_known_cause_of_its_failure(self):
        cause = {"26.04": "the deb lane fails: a known pkgsmith defect"}
        result = compute({**POLICY, "families": [FEDORA, {**UBUNTU, "experimental_reason": cause}]})
        rows = {row["release"]: row for row in result["rows"] if row["family"] == "Ubuntu"}
        self.assertEqual((rows["26.04"]["state"], rows["26.04"]["note"]), (targets.EXPERIMENTAL, cause["26.04"]))
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [FEDORA, {**UBUNTU, "experimental_reason": "x"}]})

    def test_a_reason_is_an_object_of_releases(self):
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [{**FEDORA, "planned_reason": "below the build floors"}]})

    def test_a_release_between_build_from_and_verified_from_is_experimental(self):
        result = compute({**POLICY, "families": [{**FEDORA, "build_from": "42"}, UBUNTU, DEBIAN, LEAP]})
        self.assertEqual(
            {release: state for release, state in states(result, "Fedora").items() if state != targets.PLANNED},
            {"45": targets.EXPERIMENTAL, "44": targets.BUILT, "43": targets.BUILT, "42": targets.EXPERIMENTAL},
        )
        rows = {row["release"]: row for row in result["rows"] if row["family"] == "Fedora"}
        self.assertEqual(rows["42"]["note"], "no full run has verified it yet")
        self.assertTrue(entries(result, release="42", arch="amd64")[0]["experimental"])
        self.assertEqual(entries(result, "compile", family="Fedora")[0]["release"], "42")

    def test_a_family_without_build_from_is_planned_only(self):
        self.assertEqual(set(states(compute(), "Debian").values()), {targets.PLANNED})

    def test_a_family_with_build_from_and_no_verified_release_is_experimental(self):
        self.assertEqual(
            states(compute(), "Ubuntu"),
            {
                "26.04": targets.EXPERIMENTAL,
                "24.04": targets.PLANNED,
                "22.04": targets.PLANNED,
                "20.04": targets.PLANNED,
            },
        )
        rows = {row["release"]: row for row in compute()["rows"] if row["family"] == "Ubuntu"}
        self.assertEqual(rows["26.04"]["note"], "no full run has verified it yet")

    def test_a_verified_release_older_than_build_from_is_an_error(self):
        with self.assertRaises(ValueError):
            compute(
                {
                    **POLICY,
                    "families": [
                        FEDORA,
                        {**UBUNTU, "build_from": "26.04", "verified_from": "24.04", "verified_through": "26.04"},
                    ],
                }
            )

    def test_verified_through_older_than_verified_from_is_an_error(self):
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [{**FEDORA, "verified_from": "44", "verified_through": "43"}]})

    def test_verified_from_and_verified_through_come_together(self):
        for bad in ({"verified_from": None}, {"verified_through": None}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                compute({**POLICY, "families": [{**FEDORA, **bad}]})

    def test_a_verified_release_without_build_from_is_an_error(self):
        with self.assertRaises(ValueError):
            compute(
                {
                    **POLICY,
                    "families": [
                        FEDORA,
                        {**UBUNTU, "build_from": None, "verified_from": "24.04", "verified_through": "24.04"},
                    ],
                }
            )

    def test_the_rolling_targets_are_built(self):
        self.assertEqual(set(states(compute(), "Rolling").values()), {targets.BUILT})

    def test_the_experimental_note_says_why(self):
        rows = {row["release"]: row for row in compute()["rows"] if row["family"] == "Fedora"}
        self.assertEqual(rows["45"]["note"], "not out yet")
        cycles = {**CYCLES, "fedora": [cycle("45", "2026-10-30", "2027-12-08")] + CYCLES["fedora"]}
        row = next(r for r in compute(cycles=cycles, bodhi_releases=[])["rows"] if r["name"] == "Fedora 45")
        self.assertEqual(row["state"], targets.EXPERIMENTAL)
        self.assertEqual(row["note"], "newer than 44, the last verified release")


class Jobs(unittest.TestCase):
    def test_the_matrix_is_every_built_and_experimental_target_on_every_architecture(self):
        result = compute()
        self.assertEqual(
            names(result),
            ["Fedora 43", "Fedora 44", "Fedora 45", "Ubuntu 26.04", "Arch Linux", "Alpine edge"],
        )
        self.assertEqual(names(result, arch="arm64"), names(result))
        self.assertEqual({item["arch"]: item["runner"] for item in result["matrix"]}, {"amd64": "x64", "arm64": "a64"})
        self.assertEqual(result["architectures"], POLICY["architectures"])

    def test_a_release_leaves_the_matrix_when_it_is_below_build_from(self):
        reasons = {**FEDORA["planned_reason"], "43": "GLib 2.43 is below the build floors"}
        fedora = {**FEDORA, "build_from": "44", "verified_from": "44", "planned_reason": reasons}
        policy = {**POLICY, "families": [fedora, UBUNTU, DEBIAN, LEAP]}
        self.assertEqual(names(compute(policy))[:2], ["Fedora 44", "Fedora 45"])

    def test_a_release_past_its_end_of_life_stays_built(self):
        later = compute(today=datetime.date(2026, 12, 9))
        self.assertFalse(entries(later, release="43")[0]["experimental"])

    def test_experimental_follows_the_state(self):
        result = compute()
        experimental = {item["name"] for item in result["matrix"] if item["experimental"]}
        self.assertEqual(experimental, {"Fedora 45", "Ubuntu 26.04"})

    def test_a_planned_release_gets_no_job_in_any_view(self):
        result = compute()
        for view in ("matrix", "prefixes", "packages", "compile"):
            self.assertFalse([item for item in result[view] if item["release"] in ("42", "24.04", "12", "15.6")])

    def test_the_source_is_a_built_release(self):
        result = compute()
        self.assertEqual((result["source"], result["source_image"]), ("44", "fedora:44"))

    def test_a_planned_experimental_or_missing_source_is_an_error(self):
        for source in ("45", "42", "30"):
            with self.assertRaises(ValueError):
                compute({**POLICY, "families": [{**FEDORA, "source_release": source}]})

    def test_one_family_names_the_source(self):
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [FEDORA, {**UBUNTU, "source_release": "26.04"}]})
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [{key: value for key, value in FEDORA.items() if key != "source_release"}]})

    def test_the_prefixes_are_the_entries_with_an_own_prefix(self):
        result = compute()
        self.assertEqual(
            names(result, "prefixes"), ["Fedora 43", "Fedora 44", "Fedora 45", "Ubuntu 26.04", "Alpine edge"]
        )
        self.assertEqual(len(result["prefixes"]), 10)
        deb = entries(result, "prefixes", slug="ubuntu26.04", arch="arm64")[0]
        self.assertEqual(
            (deb["prefix_image"], deb["deps_script"], deb["bootstrap"], deb["runner"]),
            ("ubuntu:26.04", "install-debian-build-deps.sh", "apt-get install -y git", "a64"),
        )

    def test_the_shared_sources_have_their_own_artifact_name_and_nothing_else_does(self):
        shared = {item["slug"] for item in compute()["prefixes"] if item["prefix_artifact"] == "prefix-shared"}
        self.assertEqual(shared, {"fc44", "alpine-edge"})

    def test_a_package_lane_has_its_own_arguments_and_the_release_in_the_revision(self):
        result = compute()
        args = {item["slug"]: item["package_args"] for item in result["packages"]}
        self.assertEqual(
            args,
            {
                "fc43": "--lanes fedora --fedora 43",
                "fc44": "--lanes fedora --fedora 44",
                "fc45": "--lanes fedora --fedora 45",
                "ubuntu26.04": "--lanes deb --deb-release ubuntu26.04",
            },
        )

    def test_only_a_published_release_makes_a_published_artifact(self):
        result = compute()
        self.assertEqual({item["slug"]: item["packages_artifact"] for item in result["packages"]}["fc43"], "packages")
        self.assertEqual(
            {item["slug"]: item["packages_artifact"] for item in result["packages"]}["ubuntu26.04"], "testpkgs"
        )

    def test_the_artifact_names_of_two_releases_never_meet(self):
        result = compute()
        names_ = [(item["packages_artifact"], item["slug"], item["arch"]) for item in result["packages"]]
        self.assertEqual(len(names_), len(set(names_)))
        prefixes = [(item["prefix_artifact"], item["slug"], item["arch"]) for item in result["prefixes"]]
        self.assertEqual(len(prefixes), len(set(prefixes)))
        self.assertNotIn(
            "packages", {item["packages_artifact"] for item in result["packages"] if not item["published"]}
        )

    def test_an_install_check_reads_the_set_of_its_own_release_or_the_shared_set(self):
        result = compute()
        own = entries(result, slug="ubuntu26.04", arch="amd64")[0]
        self.assertEqual(
            (own["install_artifact"], own["install_suffix"], own["install_image"]),
            ("testpkgs", "-ubuntu26.04", "ubuntu:26.04"),
        )
        fedora = entries(result, slug="fc44", arch="amd64")[0]
        self.assertEqual((fedora["install_artifact"], fedora["install_suffix"]), ("packages", "-fc44"))
        for slug in ("arch", "alpine-edge"):
            shared = entries(result, slug=slug, arch="amd64")[0]
            self.assertEqual((shared["install_artifact"], shared["install_suffix"]), ("packages", ""))

    def test_the_install_image_has_an_arm64_variant_only_where_the_policy_says_so(self):
        result = compute()
        self.assertEqual(entries(result, slug="arch", arch="amd64")[0]["install_image"], "archlinux:base")
        self.assertEqual(entries(result, slug="arch", arch="arm64")[0]["install_image"], "arm:base")
        self.assertEqual(entries(result, slug="alpine-edge", arch="arm64")[0]["install_image"], "alpine:edge")

    def test_the_install_image_can_differ_from_the_build_image(self):
        policy = {**POLICY, "families": [{**FEDORA, "install_image": "fedora-minimal:{release}"}, UBUNTU, DEBIAN, LEAP]}
        item = entries(compute(policy), "prefixes", slug="fc43", arch="amd64")[0]
        self.assertEqual((item["prefix_image"], item["install_image"]), ("fedora:43", "fedora-minimal:43"))

    def test_a_planned_family_with_an_own_prefix_still_checks_its_data(self):
        for bad in (
            {"deps_script": "no-such-script.sh"},
            {"bootstrap": ""},
            {"lane": "fedora"},
            {"kind": "zip"},
            {"prefix": "x"},
        ):
            with self.assertRaises(ValueError, msg=bad):
                compute({**POLICY, "families": [FEDORA, UBUNTU, {**DEBIAN, **bad}]})

    def test_the_opensuse_lane_names_its_rpms_after_the_release_and_installs_them_only_there(self):
        result = compute({**POLICY, "families": [FEDORA, UBUNTU, DEBIAN, LEAP_OWN]})
        self.assertEqual(states(result, "openSUSE Leap"), {"16.0": targets.EXPERIMENTAL, "15.6": targets.PLANNED})
        item = entries(result, slug="leap16.0", arch="amd64")[0]
        self.assertEqual(
            (item["package_args"], item["packages_artifact"], item["install_suffix"], item["prefix_image"]),
            ("--lanes opensuse --opensuse-release leap16.0", "testpkgs", "-leap16.0", "leap:16.0"),
        )
        self.assertEqual(item["install_prepare"], "zypper -n install update-alternatives")
        self.assertIn(("openSUSE Leap 16.0", "amd64"), [(i["name"], i["arch"]) for i in result["compile"]])

    def test_each_alpine_branch_builds_its_own_apks_and_never_publishes_them(self):
        result = compute({**POLICY, "families": [FEDORA, UBUNTU, DEBIAN, LEAP, ALPINE]})
        self.assertEqual(
            states(result, "Alpine"),
            {
                "3.24": targets.EXPERIMENTAL,
                "3.23": targets.EXPERIMENTAL,
                "3.22": targets.EXPERIMENTAL,
                "3.21": targets.PLANNED,
            },
        )
        branches = entries(result, "packages", family="Alpine", arch="amd64")
        self.assertEqual(
            [(i["prefix_artifact"], i["slug"], i["package_args"], i["packages_artifact"]) for i in branches],
            [
                ("prefix", "alpine3.22", "--lanes other", "testpkgs"),
                ("prefix", "alpine3.23", "--lanes other", "testpkgs"),
                ("prefix", "alpine3.24", "--lanes other", "testpkgs"),
            ],
        )
        self.assertEqual({i["install_suffix"] for i in branches}, {"-alpine3.22", "-alpine3.23", "-alpine3.24"})
        self.assertEqual(entries(result, slug="alpine-edge", arch="amd64")[0]["prefix_artifact"], "prefix-shared")

    def test_install_prepare_is_empty_unless_the_policy_names_it(self):
        result = compute()
        self.assertEqual(entries(result, slug="fc44", arch="amd64")[0]["install_prepare"], "")
        self.assertEqual(entries(result, slug="alpine-edge", arch="arm64")[0]["install_prepare"], "apk add bash")
        with self.assertRaises(ValueError):
            compute({**POLICY, "rolling": [ARCH, {**EDGE, "install_prepare": ["apk", "add", "bash"]}]})

    def test_a_shared_prefix_has_no_build_data(self):
        with self.assertRaises(ValueError):
            compute({**POLICY, "families": [FEDORA, {**LEAP, "lane": "deb"}]})

    def test_a_prefix_without_a_lane_must_feed_the_shared_packages(self):
        with self.assertRaises(ValueError):
            compute({**POLICY, "rolling": [ARCH, {**EDGE, "shared_source": False}]})

    def test_the_compile_gate_is_the_oldest_release_of_each_family_on_the_first_architecture(self):
        result = compute()
        self.assertEqual(
            [(item["name"], item["arch"]) for item in result["compile"]],
            [("Fedora 43", "amd64"), ("Ubuntu 26.04", "amd64")],
        )
        self.assertTrue(entries(result, "compile", release="26.04")[0]["experimental"])

    def test_a_floor_that_is_not_listed_has_no_compile_job(self):
        policy = {**POLICY, "families": [{**FEDORA, "build_from": "30"}, UBUNTU, DEBIAN, LEAP]}
        self.assertEqual([item["name"] for item in compute(policy)["compile"]], ["Ubuntu 26.04"])

    def test_the_job_counts(self):
        self.assertEqual(
            compute()["jobs"],
            {"shared prefix": 4, "shared packages": 2, "target legs": 12, "prefix": 6, "packages": 8,
             "package suite": 12, "graphical": 0, "verdict": 12, "compile": 2},
        )  # fmt: skip

    def test_the_legs_split_on_the_shared_stage(self):
        result = compute(GRAPHICAL_POLICY, GRAPHICAL_CYCLES)
        shared = {(leg["slug"], leg["arch"]) for leg in result["legs_shared"]}
        own = {(leg["slug"], leg["arch"]) for leg in result["legs_own"]}
        self.assertEqual(len(shared) + len(own), len(result["matrix"]))
        self.assertFalse(shared & own)
        for leg in result["legs_shared"]:
            self.assertTrue(leg["prefix_artifact"] == "prefix-shared" or not leg["lane"])
        self.assertEqual(
            {(item["slug"], item["arch"]) for item in result["shared_prefixes"]},
            {(leg["slug"], leg["arch"]) for leg in result["legs_shared"] if leg["prefix"] == "own"},
        )
        with_graphical = [leg for leg in result["legs_own"] + result["legs_shared"] if leg["graphical"]]
        self.assertEqual(len(with_graphical), len(result["graphical_packaged"]))


class Table(unittest.TestCase):
    def test_the_table_lists_every_state_the_dates_and_the_rule(self):
        text = targets.table(compute())
        for expected in (
            "built",
            "experimental",
            "planned",
            "GLib 2.42 is below the build floors",
            "2027-06-02",
            "rolling",
        ):
            self.assertIn(expected, text)
        self.assertIn("newest 4 releases", text)
        self.assertIn("last 5 years", text)
        self.assertIn("Ubuntu: long-term support releases only", text)

    def test_the_table_counts_the_states_of_each_family(self):
        lines = targets.table(compute()).splitlines()
        self.assertIn("Fedora: 2 built, 1 experimental, 5 planned", lines)
        self.assertIn("Ubuntu: 1 experimental, 3 planned", lines)
        self.assertIn("Debian: 4 planned", lines)
        self.assertIn("Rolling: 2 built", lines)
        self.assertIn(
            "Jobs: 4 shared prefix, 2 shared packages, 12 target legs (6 prefix, 8 packages, 12 package suite, "
            "0 graphical, 12 verdict), 2 compile",
            lines,
        )


class ShippedPolicy(unittest.TestCase):
    policy = json.loads((ROOT / "scripts/targets.json").read_text())

    def test_every_family_has_a_release_image_a_slug_and_build_data(self):
        self.assertGreaterEqual(self.policy["newest"], 1)
        self.assertGreaterEqual(self.policy["window_years"], 1)
        for family in self.policy["families"]:
            targets.check_family(family)
        for item in self.policy["rolling"]:
            targets.check_target(item)

    def test_one_family_names_the_source(self):
        self.assertEqual(["source_release" in family for family in self.policy["families"]].count(True), 1)

    def test_every_architecture_has_a_runner_and_a_graphical_guest(self):
        guest_keys = {
            "cpu",
            "boot",
            "qemu",
            "machine",
            "tcg_cpu",
            "gpu",
            "firmware",
            "host_packages",
            "timeout_minutes",
        }
        for architecture in self.policy["architectures"]:
            self.assertEqual(set(architecture), {"arch", "runner", "guest"})
            self.assertEqual(set(architecture["guest"]), guest_keys)
        self.assertEqual(
            set(self.policy["graphical_architectures"]), {item["arch"] for item in self.policy["architectures"]}
        )

    def test_the_shipped_policy_gives_unique_job_names_for_a_full_target_list(self):
        # A listing with every family fully populated: the names that jobs and artifacts take must not repeat. The fixture
        # releases that the shipped policy would list as planned without a reason are left out: the reasons are data of
        # the real releases.
        cycles = {}
        for family in self.policy["families"]:
            floor, reasons = family["build_from"], family["planned_reason"]
            cycles[family["product"]] = [
                item
                for item in CYCLES[family["product"]]
                if not targets.is_version(item["cycle"])
                or item["cycle"] in reasons
                or (floor and targets.version_key(item["cycle"]) >= targets.version_key(floor))
            ]
        result = compute(self.policy, cycles)
        keys = [(item["slug"], item["arch"]) for item in result["matrix"]]
        self.assertEqual(len(keys), len(set(keys)))


def with_image(item, **image):
    return {**item, "graphical": image or {"index": ["https://example.invalid/{release}/"], "file": "img-{release}\\.qcow2",
                                           "checksum": "SUMS"}}  # fmt: skip


GRAPHICAL_POLICY = {
    **POLICY,
    "graphical_newest": 3,
    "graphical_architectures": ["amd64"],
    "families": [
        with_image(FEDORA),
        with_image(UBUNTU),
        with_image(
            DEBIAN,
            index=["https://cloud.example.invalid/{codename}/latest/"],
            file="debian-{release}-generic-amd64\\.qcow2",
            checksum="SHA512SUMS",
        ),  # fmt: skip
        with_image(LEAP),
    ],
    "rolling": [with_image(ARCH), {**EDGE, "graphical": {"base": "Debian", "upgrade": "switch to testing"}}],
}
GRAPHICAL_CYCLES = {
    **CYCLES,
    "debian": [{**item, "codename": name} for item, name in zip(CYCLES["debian"], ("Trixie", "Bookworm", "Bullseye"))]
    + CYCLES["debian"][3:],
}


class Graphical(unittest.TestCase):
    def setUp(self):
        self.entries = compute(GRAPHICAL_POLICY, GRAPHICAL_CYCLES)["graphical"]
        self.by_name = {item["name"]: item for item in self.entries}

    def releases(self, family):
        return [item["release"] for item in self.entries if item["family"] == family]

    def test_the_newest_releases_of_each_family_with_or_without_packages(self):
        # Fedora 45 is a target outside the window (not out yet): it has packages, so it has an entry too.
        self.assertEqual(self.releases("Fedora"), ["45", "44", "43", "42"])
        self.assertTrue(self.by_name["Fedora 45"]["packaged"])
        self.assertEqual(self.releases("Ubuntu"), ["26.04", "24.04", "22.04"])
        self.assertEqual(self.releases("Debian"), ["13", "12", "11"])
        self.assertEqual(self.releases("openSUSE Leap"), ["16.0", "15.6"])
        self.assertEqual(self.releases("Rolling"), ["rolling", "rolling"])

    def test_end_of_life_is_on_or_before_today(self):
        self.assertFalse(self.by_name["Fedora 43"]["eol"])
        self.assertTrue(self.by_name["Fedora 42"]["eol"])
        self.assertEqual(self.by_name["Fedora 42"]["eol_date"], "2026-05-27")
        self.assertFalse(self.by_name["Arch Linux"]["eol"])
        on_the_day = compute(GRAPHICAL_POLICY, GRAPHICAL_CYCLES, today=datetime.date(2026, 12, 9))["graphical"]
        self.assertTrue(next(item for item in on_the_day if item["name"] == "Fedora 43")["eol"])

    def test_a_release_without_packages_carries_its_blocker(self):
        fedora42 = self.by_name["Fedora 42"]
        self.assertEqual((fedora42["packaged"], fedora42["state"]), (False, "planned"))
        self.assertEqual(fedora42["reason"], "GLib 2.42 is below the build floors")
        self.assertEqual(fedora42["install_artifact"], "")
        self.assertTrue(self.by_name["Fedora 44"]["packaged"])
        self.assertEqual(self.by_name["Fedora 44"]["reason"], "")

    def test_a_release_without_a_recorded_blocker_says_why_it_has_no_packages(self):
        policy = {**GRAPHICAL_POLICY, "graphical_newest": 5}
        entries = compute(policy, GRAPHICAL_CYCLES)["graphical"]
        ubuntu1804 = next(item for item in entries if item["name"] == "Ubuntu 18.04")
        self.assertFalse(ubuntu1804["packaged"])
        self.assertIn("older than 26.04", ubuntu1804["reason"])

    def test_the_image_data_is_resolved_for_the_release(self):
        image = self.by_name["Ubuntu 26.04"]["image"]
        self.assertEqual(image["index"], ["https://example.invalid/26.04/"])
        self.assertEqual(image["file"], "img-26\\.04\\.qcow2")
        self.assertEqual(self.by_name["Debian 13"]["image"]["index"], ["https://cloud.example.invalid/trixie/latest/"])
        self.assertEqual(self.by_name["Debian 13"]["image"]["upgrade"], "")

    def test_a_rolling_target_can_boot_the_newest_release_of_a_family_and_upgrade(self):
        edge = self.by_name["Alpine edge"]
        self.assertEqual(edge["image"]["file"], "debian-13-generic-amd64\\.qcow2")
        self.assertEqual(edge["image"]["upgrade"], "switch to testing")
        self.assertEqual(self.by_name["Arch Linux"]["image"]["upgrade"], "")

    def test_entries_name_their_package_artifact_on_the_graphical_architectures_only(self):
        self.assertEqual({item["arch"] for item in self.entries}, {"amd64"})
        self.assertEqual({item["runner"] for item in self.entries}, {"x64"})
        fedora44 = self.by_name["Fedora 44"]
        self.assertEqual((fedora44["install_artifact"], fedora44["install_suffix"]), ("packages", "-fc44"))
        self.assertEqual(self.by_name["Arch Linux"]["install_suffix"], "")

    def test_without_graphical_newest_there_are_no_entries(self):
        self.assertEqual(compute()["graphical"], [])

    def test_a_target_without_image_data_is_an_error(self):
        policy = {**GRAPHICAL_POLICY, "rolling": [ARCH]}
        with self.assertRaisesRegex(ValueError, "graphical names the cloud image"):
            compute(policy, GRAPHICAL_CYCLES)
        policy = {**GRAPHICAL_POLICY, "rolling": [{**ARCH, "graphical": {"base": "Gentoo", "upgrade": "x"}}]}
        with self.assertRaisesRegex(ValueError, "graphical base names a family"):
            compute(policy, GRAPHICAL_CYCLES)

    def test_the_table_counts_the_graphical_jobs(self):
        text = targets.table(compute(GRAPHICAL_POLICY, GRAPHICAL_CYCLES))
        self.assertIn("Graphical VM test by hand (graphical.yml): 14 entries", text)
        self.assertIn("6 graphical, 12 verdict", text)

    def test_arm64_entries_take_the_guest_data_and_the_image_of_their_architecture(self):
        architectures = [
            {"arch": "amd64", "runner": "x64", "guest": {"cpu": "x86_64", "boot": "bios"}},
            {"arch": "arm64", "runner": "a64", "guest": {"cpu": "aarch64", "boot": "uefi"}},
        ]
        image = {"index": ["https://example.invalid/{release}/{cpu}/"], "file": "img-{release}-{cpu}-{boot}\\.qcow2",
                 "checksum": "SUMS", "arm64": {"index": ["https://ports.example.invalid/{release}/"]}}  # fmt: skip
        policy = {
            **GRAPHICAL_POLICY,
            "architectures": architectures,
            "graphical_architectures": ["amd64", "arm64"],
            "families": [with_image(FEDORA, **image), *GRAPHICAL_POLICY["families"][1:]],
            "rolling": [{**with_image(ARCH), "graphical": {**with_image(ARCH)["graphical"], "arm64": {"none": "no"}}}],
        }
        result = compute(policy, GRAPHICAL_CYCLES)
        entries = {(item["name"], item["arch"]): item for item in result["graphical"]}
        amd64, arm64 = entries[("Fedora 44", "amd64")], entries[("Fedora 44", "arm64")]
        self.assertEqual(amd64["image"]["file"], "img-44-x86_64-bios\\.qcow2")
        self.assertEqual(amd64["image"]["index"], ["https://example.invalid/44/x86_64/"])
        self.assertEqual(arm64["image"]["file"], "img-44-aarch64-uefi\\.qcow2")
        self.assertEqual(arm64["image"]["index"], ["https://ports.example.invalid/44/"])
        self.assertEqual((arm64["runner"], arm64["guest"]["cpu"]), ("a64", "aarch64"))
        arch_arm64 = entries[("Arch Linux", "arm64")]
        self.assertEqual((arch_arm64["image"], arch_arm64["image_missing"]), (None, "no"))
        self.assertIn(arch_arm64, result["graphical_packaged"])
        self.assertIn("No graphical image yet, so the graphical stage is not run", targets.table(result))
        self.assertIn("Arch Linux arm64", targets.table(result))

    def test_an_architecture_without_a_published_image_can_name_a_build(self):
        architectures = [
            {"arch": "amd64", "runner": "x64", "guest": {"cpu": "x86_64", "boot": "bios"}},
            {"arch": "arm64", "runner": "a64", "guest": {"cpu": "aarch64", "boot": "uefi"}},
        ]
        build = {"script": "scripts/build.sh", "file": "image.qcow2", "sudo": True}
        graphical = {**with_image(ARCH)["graphical"], "arm64": {"build": build}}
        policy = {**GRAPHICAL_POLICY, "architectures": architectures, "graphical_architectures": ["amd64", "arm64"],
                  "rolling": [{**ARCH, "graphical": graphical}]}  # fmt: skip
        result = compute(policy, GRAPHICAL_CYCLES)
        arm64 = next(
            item for item in result["graphical_packaged"] if item["name"] == "Arch Linux" and item["arch"] == "arm64"
        )
        self.assertEqual(arm64["image"], {"build": build, "upgrade": ""})
        self.assertNotIn("No graphical image yet", targets.table(result))
        broken = {**policy, "rolling": [{**ARCH, "graphical": {**graphical, "arm64": {"build": {"file": "x"}}}}]}
        with self.assertRaisesRegex(ValueError, "arm64 build needs script"):
            compute(broken, GRAPHICAL_CYCLES)

    def test_a_guest_placeholder_without_guest_data_is_an_error(self):
        policy = {**GRAPHICAL_POLICY, "families": [with_image(FEDORA, index=["x/{cpu}"], file="f", checksum="c")]}
        with self.assertRaisesRegex(ValueError, "names {cpu}"):
            compute(policy, GRAPHICAL_CYCLES)

    def test_the_shipped_policy_has_image_data_for_every_graphical_target(self):
        policy = json.loads((ROOT / "scripts/targets.json").read_text())
        self.assertGreaterEqual(policy["graphical_newest"], 1)
        for item in policy["families"] + policy["rolling"]:
            data = item["graphical"]
            self.assertTrue(("base" in data and data.get("upgrade")) or {"index", "file", "checksum"} <= set(data))


if __name__ == "__main__":
    unittest.main()

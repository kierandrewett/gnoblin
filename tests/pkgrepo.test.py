#!/usr/bin/env python3
"""Unit tests of scripts/pkgrepo that need no RPM, APT or pacman tools.

tests/pkgrepo/containers.sh runs the same tool against the real tools in containers; this file covers the rules:
version naming and ordering, merge and retention, layout, publish plans and the two back ends that run anywhere
(rsync to a directory and a Git branch).
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

from pkgrepo import index, publish, tree, versions  # noqa: E402
from pkgrepo.packages import Package  # noqa: E402


def package(name="bingux", version="0.1.12-1", family="fedora", release="43", arch="x86_64", fmt="rpm", sha="a" * 64):
    return Package(fmt, family, release, name, version, arch, f"{name}-{version}.{arch}", 1, sha)


class NightlyNaming(unittest.TestCase):
    def test_next_stable_bumps_the_patch_of_the_last_tag(self):
        self.assertEqual(versions.next_stable("0.1.12"), "0.1.13")
        self.assertEqual(versions.next_stable("0.1.12", "0.1.12"), "0.1.13")

    def test_a_tree_ahead_of_the_last_tag_names_the_release_being_prepared(self):
        self.assertEqual(versions.next_stable("0.1.12", "0.2.0"), "0.2.0")

    def test_rejects_prerelease_input(self):
        with self.assertRaises(ValueError):
            versions.next_stable("0.1.12-rc1")

    def test_spellings(self):
        value = versions.nightly("0.1.12", "0.1.12", "20261031", 142, "abc1234def")
        self.assertEqual(value.rpm, "0.1.13~nightly.20261031.142.gabc1234")
        self.assertEqual(value.deb, value.rpm)
        self.assertEqual(value.arch, "0.1.13nightly20261031.142")
        self.assertEqual(value.semver, "0.1.13-nightly.20261031.142.gabc1234")

    def test_input_validation(self):
        with self.assertRaises(ValueError):
            versions.nightly("0.1.12", None, "2026-10-31", 1, "abc1234")
        with self.assertRaises(ValueError):
            versions.nightly("0.1.12", None, "20261031", 1, "ABC")


class Ordering(unittest.TestCase):
    """A nightly sorts above the last stable and below the next stable, in all three orderings."""

    def setUp(self):
        self.old = versions.nightly("0.1.12", None, "20261031", 142, "abc1234")
        self.new = versions.nightly("0.1.12", None, "20261101", 150, "def5678")
        self.same_day = versions.nightly("0.1.12", None, "20261031", 143, "0000001")

    def test_rpm(self):
        c = versions.rpmvercmp
        self.assertEqual(c(self.old.rpm, "0.1.13"), -1)
        self.assertEqual(c(self.old.rpm, "0.1.12"), 1)
        self.assertEqual(c(self.old.rpm, self.new.rpm), -1)
        self.assertEqual(c(self.same_day.rpm, self.old.rpm), 1)

    def test_deb(self):
        c = versions.dpkg_compare
        self.assertEqual(c(self.old.deb + "-1", "0.1.13-1"), -1)
        self.assertEqual(c(self.old.deb + "-1", "0.1.12-9"), 1)
        self.assertEqual(c(self.old.deb + "-1", self.new.deb + "-1"), -1)
        self.assertEqual(c(self.same_day.deb + "-1", self.old.deb + "-1"), 1)

    def test_pacman(self):
        c = versions.pacman_compare
        self.assertEqual(c(self.old.arch + "-1", "0.1.13-1"), -1)
        self.assertEqual(c(self.old.arch + "-1", "0.1.12-1"), 1)
        self.assertEqual(c(self.old.arch + "-1", self.new.arch + "-1"), -1)
        self.assertEqual(c(self.same_day.arch + "-1", self.old.arch + "-1"), 1)

    def test_pacman_separator_would_be_wrong(self):
        # pacman has no tilde. A dot before the letters makes the nightly newer than the stable, which is why Arch
        # nightlies have no separator before "nightly".
        self.assertEqual(versions.pacman_compare("0.1.13.nightly1-1", "0.1.13-1"), 1)

    def test_known_vectors(self):
        self.assertEqual(versions.rpmvercmp("1.0~rc1", "1.0"), -1)
        self.assertEqual(versions.rpmvercmp("1.0^git1", "1.0"), 1)
        self.assertEqual(versions.rpmvercmp("5.5p1", "5.5p2"), -1)
        self.assertEqual(versions.rpmvercmp("1.05", "1.5"), 0)
        self.assertEqual(versions.dpkg_compare("1.0~rc1-1", "1.0-1"), -1)
        self.assertEqual(versions.dpkg_compare("1:0.1-1", "2.0-1"), 1)
        self.assertEqual(versions.pacman_compare("1.0a-1", "1.0-1"), -1)
        self.assertEqual(versions.pacman_compare("1:0.1-1", "2.0-1"), 1)


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class GitDescribe(unittest.TestCase):
    def repo(self):
        directory = Path(tempfile.mkdtemp(prefix="pkgrepo-git-"))
        self.addCleanup(shutil.rmtree, directory, True)
        subprocess.run(["git", "init", "-q", str(directory)], check=True)
        return directory

    def commit(self, directory):
        subprocess.run(
            ["git", "-C", str(directory), "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "x"],
            check=True,
        )

    def test_counts_commits_since_the_last_stable_tag(self):
        from pkgrepo import cli

        directory = self.repo()
        self.commit(directory)
        subprocess.run(["git", "-C", str(directory), "tag", "gnoblin-v0.1.12"], check=True)
        self.commit(directory)
        self.commit(directory)
        last, count, commit, date = cli.describe_git(directory, "gnoblin-v")
        self.assertEqual((last, count), ("0.1.12", 2))
        self.assertRegex(commit, "^[0-9a-f]{7}$")
        self.assertRegex(date, "^[0-9]{8}$")

    def test_without_a_tag_the_tree_version_names_the_coming_release(self):
        from pkgrepo import cli

        directory = self.repo()
        self.commit(directory)
        self.commit(directory)
        last, count, _, _ = cli.describe_git(directory, "v")
        self.assertEqual((last, count), ("0.0.0", 2))
        self.assertEqual(versions.next_stable(last, "0.1.0"), "0.1.0")


class Merge(unittest.TestCase):
    def entry(self, **kwargs):
        return tree.Entry(package(**kwargs), Path("/dev/null"), new=True)

    def test_stable_refuses_to_replace_a_published_version(self):
        existing = [tree.Entry(package(), Path("/dev/null"), new=False)]
        with self.assertRaisesRegex(tree.TreeError, "never replaced"):
            tree.merge(existing, [self.entry()], "stable", False)

    def test_stable_replace_is_an_explicit_choice(self):
        existing = [tree.Entry(package(), Path("/dev/null"), new=False)]
        merged = tree.merge(existing, [self.entry()], "stable", True)
        self.assertEqual(len(merged), 1)
        self.assertTrue(merged[0].new)

    def test_nightly_replaces_the_same_version(self):
        existing = [tree.Entry(package(version="0.1.13~nightly.1-1"), Path("/dev/null"), new=False)]
        merged = tree.merge(existing, [self.entry(version="0.1.13~nightly.1-1")], "nightly", False)
        self.assertEqual(len(merged), 1)
        self.assertTrue(merged[0].new)


class Retention(unittest.TestCase):
    def entries(self, count, name="bingux", fmt="rpm"):
        make = lambda i: tree.Entry(  # noqa: E731
            package(name=name, version=f"0.1.13~nightly.2026103{i}.{i}.gabc1234-1", fmt=fmt),
            Path("/dev/null"),
            new=False,
        )
        return [make(i) for i in range(count)]

    def test_nightly_keeps_the_newest_of_each_package(self):
        entries = self.entries(5) + self.entries(2, name="bingux-files")
        kept, dropped = tree.apply_retention(entries, "nightly", 3)
        self.assertEqual(len(kept), 5)
        self.assertEqual(len(dropped), 2)
        self.assertEqual(
            sorted(e.package.version for e in dropped),
            ["0.1.13~nightly.20261030.0.gabc1234-1", "0.1.13~nightly.20261031.1.gabc1234-1"],
        )

    def test_stable_is_never_pruned(self):
        kept, dropped = tree.apply_retention(self.entries(40), "stable", 3)
        self.assertEqual((len(kept), len(dropped)), (40, 0))

    def test_keep_must_be_positive(self):
        with self.assertRaises(tree.TreeError):
            tree.apply_retention(self.entries(2), "nightly", 0)


class Layout(unittest.TestCase):
    def test_architecture_independent_packages_follow_every_architecture(self):
        entries = [
            tree.Entry(package(name="a", arch="x86_64"), Path("/x"), True),
            tree.Entry(package(name="a", arch="aarch64"), Path("/x"), True),
            tree.Entry(package(name="docs", arch="noarch"), Path("/x"), True),
        ]
        dirs = tree._arch_dirs(entries, "x86_64")
        self.assertEqual(dirs[("fedora", "43")], ["aarch64", "x86_64"])
        paths = tree._destinations(entries[2].package, dirs)
        self.assertEqual(
            paths,
            [
                "fedora/43/aarch64/docs-0.1.12-1.noarch",
                "fedora/43/x86_64/docs-0.1.12-1.noarch",
            ],
        )

    def test_only_independent_packages_use_the_default_architecture(self):
        entries = [tree.Entry(package(name="docs", arch="noarch"), Path("/x"), True)]
        self.assertEqual(tree._arch_dirs(entries, "x86_64")[("fedora", "43")], ["x86_64"])

    def test_deb_has_one_pool_per_suite(self):
        deb = package(family="deb", release="trixie", arch="amd64", fmt="deb")
        self.assertEqual(tree._destinations(deb, {}), [f"deb/pool/trixie/{deb.filename}"])

    def test_pacman_repository_names(self):
        self.assertEqual(tree.pacman_repo_name("gnoblin", "stable"), "gnoblin")
        self.assertEqual(tree.pacman_repo_name("gnoblin", "nightly"), "gnoblin-nightly")


class ClientFiles(unittest.TestCase):
    def test_rpm_repo_file_checks_packages_and_metadata(self):
        text = index.rpm_repo_file(
            "https://repo.example", "gnoblin", "nightly", "fedora/$releasever/$basearch", "F" * 40
        )
        self.assertIn("[gnoblin-nightly]", text)
        self.assertIn("baseurl=https://repo.example/gnoblin/nightly/fedora/$releasever/$basearch", text)
        self.assertIn("gpgcheck=1\nrepo_gpgcheck=1", text)
        self.assertIn("gpgkey=https://repo.example/gpg.key", text)

    def test_pacman_snippet_requires_signatures(self):
        text = index.pacman_snippet("https://repo.example", "bingux", "stable", "bingux")
        self.assertIn("SigLevel = Required DatabaseRequired", text)
        self.assertIn("Server = https://repo.example/bingux/stable/arch/$arch", text)

    def test_fingerprint_groups(self):
        self.assertEqual(index.fingerprint_text("ABCD" * 3), "ABCD ABCD ABCD")


def build_tree(directory: Path, project="bingux", channel="nightly", packages=None):
    """A channel tree with just a manifest and package files, which is all publish reads."""
    scope = directory / project / channel
    scope.mkdir(parents=True)
    records = []
    for relative, content in (packages or {}).items():
        path = scope / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        import hashlib

        records.append({"filename": path.name, "path": relative, "sha256": hashlib.sha256(content).hexdigest()})
    (scope / "manifest.json").write_text(json.dumps({"schema": 1, "packages": records}))
    (scope / "index.html").write_text("<html></html>")
    (scope / "fedora").mkdir(exist_ok=True)
    (scope / "fedora" / "repo.repo").write_text("[x]\n")
    (directory / "gpg.key").write_text("key")
    (directory / "index.html").write_text("<html>root</html>")
    return scope


class PublishPlans(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="pkgrepo-test-"))
        self.addCleanup(shutil.rmtree, self.directory, True)
        self.packages = {
            "fedora/43/x86_64/a-1.rpm": b"one",
            "fedora/43/x86_64/b-1.rpm": b"two",
            "arch/x86_64/c-1-1-x86_64.pkg.tar.zst": b"three",
            "arch/x86_64/c-1-1-x86_64.pkg.tar.zst.sig": b"sig",
            "arch/x86_64/bingux-nightly.db.tar.zst": b"database",
        }
        build_tree(self.directory / "tree", packages=self.packages)

    def test_package_files_are_not_database_files(self):
        self.assertTrue(publish.is_package_file("arch/x86_64/c-1-1-x86_64.pkg.tar.zst"))
        self.assertTrue(publish.is_package_file("arch/x86_64/c-1-1-x86_64.pkg.tar.zst.sig"))
        self.assertFalse(publish.is_package_file("arch/x86_64/bingux-nightly.db.tar.zst"))
        self.assertTrue(publish.is_package_file("deb/pool/trixie/a_1_amd64.deb"))

    def test_everything_is_new_when_nothing_is_published(self):
        changed = publish.changed_packages(self.directory / "tree", "bingux/nightly", None)
        self.assertEqual(len(changed), 4)

    def test_only_changed_packages_are_uploaded(self):
        # The published copy lists a-1.rpm unchanged and b-1.rpm with other content.
        published = self.directory / "published"
        build_tree(published, packages={**self.packages, "fedora/43/x86_64/b-1.rpm": b"different"})
        previous = publish.previous_hashes(published, "bingux/nightly")
        changed = publish.changed_packages(self.directory / "tree", "bingux/nightly", previous)
        self.assertEqual(changed, ["bingux/nightly/fedora/43/x86_64/b-1.rpm"])

    def test_a_signature_is_uploaded_with_a_changed_package_only(self):
        published = self.directory / "published"
        build_tree(published, packages={**self.packages, "arch/x86_64/c-1-1-x86_64.pkg.tar.zst": b"old"})
        previous = publish.previous_hashes(published, "bingux/nightly")
        changed = publish.changed_packages(self.directory / "tree", "bingux/nightly", previous)
        self.assertIn("bingux/nightly/arch/x86_64/c-1-1-x86_64.pkg.tar.zst.sig", changed)

    def test_dry_run_runs_nothing(self):
        calls = []
        original = publish.run
        publish.run = lambda *a, **k: calls.append(a) or ""
        self.addCleanup(setattr, publish, "run", original)
        plan = publish.publish(
            "rsync",
            self.directory / "tree",
            "bingux",
            "nightly",
            "user@host:/srv/repo",
            previous=None,
            apply=False,
            environment={},
        )
        self.assertEqual(calls, [])
        self.assertEqual(plan.steps[-1].description, "remove files the new tree no longer has")

    def test_rsync_order_is_packages_then_metadata_then_removal(self):
        plan = publish.plan_rsync(self.directory / "tree", "bingux/nightly", "user@host:/srv/repo", [], env={})
        descriptions = [s.description for s in plan.steps]
        self.assertLess(
            descriptions.index("upload package files the destination lacks"),
            descriptions.index("upload metadata and pages"),
        )
        self.assertEqual(descriptions[-1], "remove files the new tree no longer has")
        self.addCleanup(lambda: [c() for c in plan.cleanup])

    def test_rsync_uses_the_deploy_key_for_remote_destinations(self):
        plan = publish.plan_rsync(
            self.directory / "tree",
            "bingux/nightly",
            "user@host:/srv/repo",
            [],
            env={"REPO_RSYNC_KEY": "PRIVATE", "REPO_RSYNC_KNOWN_HOSTS": "host ssh-ed25519 AAAA"},
        )
        self.addCleanup(lambda: [c() for c in plan.cleanup])
        command = plan.steps[0].command
        ssh = command[command.index("-e") + 1]
        self.assertIn("StrictHostKeyChecking=yes", ssh)
        self.assertIn("IdentitiesOnly=yes", ssh)

    def test_s3_plan_syncs_packages_before_metadata_and_never_prints_credentials(self):
        plan = publish.plan_s3(
            self.directory / "tree", "bingux/nightly", "s3://bucket/prefix", [], env={"AWS_SECRET_ACCESS_KEY": "s"}
        )
        text = plan.show()
        self.assertNotIn("AWS_SECRET_ACCESS_KEY", text)
        order = [s.description for s in plan.steps]
        self.assertLess(order.index("upload package files the bucket lacks"), order.index("upload metadata and pages"))
        with self.assertRaises(ValueError):
            publish.plan_s3(self.directory / "tree", "bingux/nightly", "bucket/prefix", [], env={})

    def test_unknown_backend_and_unbuilt_channel(self):
        with self.assertRaises(ValueError):
            publish.publish("ftp", self.directory / "tree", "bingux", "nightly", "x", previous=None, apply=False)
        with self.assertRaises(ValueError):
            publish.publish("rsync", self.directory / "tree", "gnoblin", "stable", "x", previous=None, apply=False)


class GnoblinNightlyRecipes(unittest.TestCase):
    """A nightly version reaches the Fedora spec and the Arch recipe in the spelling of each package manager."""

    def setUp(self):
        self.copy = Path(tempfile.mkdtemp(prefix="pkgrepo-gnoblin-"))
        self.addCleanup(shutil.rmtree, self.copy, True)
        for name in ("scripts", "versions", "packaging"):
            shutil.copytree(ROOT / name, self.copy / name, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))

    def run_script(self, *args):
        return subprocess.run(
            [sys.executable, str(self.copy / "scripts" / args[0]), *args[1:]], capture_output=True, text=True
        )

    def test_release_recipes_are_unchanged_by_the_source_version_split(self):
        result = self.run_script("sync-package-manifest.py", "check")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_nightly_version_in_the_recipes(self):
        semver = "0.1.13-nightly.20261031.142.gabc1234"
        result = self.run_script("apply-nightly-version.py", semver)
        self.assertEqual(result.returncode, 0, result.stderr)
        spec = (self.copy / "packaging/rpm/gnoblin.spec").read_text()
        self.assertIn("Version:        0.1.13~nightly.20261031.142.gabc1234\n", spec)
        self.assertIn(f"%global source_version {semver}\n", spec)
        self.assertIn("Source0:        gnoblin-%{source_version}-source.tar.xz", spec)
        self.assertIn("Epoch:          1\n", spec)
        arch = (self.copy / "packaging/arch/PKGBUILD").read_text()
        self.assertIn("pkgver=0.1.13nightly20261031.142\n", arch)
        self.assertIn(f"_srcver={semver}\n", arch)
        portal = (self.copy / "packaging/arch/portal/PKGBUILD").read_text()
        original = (ROOT / "packaging/arch/portal/PKGBUILD").read_text()
        version_line = lambda text: next(line for line in text.splitlines() if line.startswith("pkgver="))  # noqa: E731
        self.assertEqual(version_line(portal), version_line(original), "the portal keeps its own GNOME version")
        # The same copy still passes the consistency check.
        self.assertEqual(self.run_script("sync-package-manifest.py", "check").returncode, 0)

    def test_only_nightly_versions_are_accepted(self):
        self.assertEqual(self.run_script("apply-nightly-version.py", "0.1.13").returncode, 2)
        self.assertEqual(self.run_script("apply-nightly-version.py", "0.1.13-rc1").returncode, 2)

    def test_the_spellings_agree_with_the_tool(self):
        value = versions.nightly("0.1.12", "0.1.12", "20261031", 142, "abc1234")
        self.assertEqual(versions.semver_to_rpm(value.semver), value.rpm)
        self.assertEqual(versions.semver_to_arch(value.semver), value.arch)
        self.assertEqual(versions.semver_to_rpm("0.2.0"), "0.2.0")
        self.assertEqual(versions.semver_to_arch("0.2.0-rc.1"), "0.2.0rc1")


@unittest.skipUnless(shutil.which("rsync"), "rsync is not installed")
class RsyncApply(unittest.TestCase):
    def test_publishes_to_a_directory_and_removes_stale_files(self):
        directory = Path(tempfile.mkdtemp(prefix="pkgrepo-test-"))
        self.addCleanup(shutil.rmtree, directory, True)
        build_tree(directory / "tree", packages={"fedora/43/x86_64/a-1.rpm": b"one"})
        destination = directory / "site"
        stale = destination / "bingux" / "nightly" / "fedora" / "43" / "x86_64" / "old-0.rpm"
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b"old")
        other_project = destination / "gnoblin" / "stable" / "keep.txt"
        other_project.parent.mkdir(parents=True)
        other_project.write_text("keep")
        publish.publish(
            "rsync",
            directory / "tree",
            "bingux",
            "nightly",
            str(destination),
            previous=None,
            apply=True,
            environment={},
        )
        self.assertTrue((destination / "bingux/nightly/fedora/43/x86_64/a-1.rpm").exists())
        self.assertTrue((destination / "bingux/nightly/manifest.json").exists())
        self.assertTrue((destination / "gpg.key").exists())
        self.assertFalse(stale.exists(), "a file the new tree lacks is removed")
        self.assertTrue(other_project.exists(), "another project is untouched")


@unittest.skipUnless(shutil.which("git"), "git is not installed")
class PagesApply(unittest.TestCase):
    def git(self, *args, cwd=None):
        return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout

    def test_publishes_one_snapshot_commit_and_keeps_other_projects(self):
        directory = Path(tempfile.mkdtemp(prefix="pkgrepo-test-"))
        self.addCleanup(shutil.rmtree, directory, True)
        remote = directory / "remote.git"
        self.git("init", "--bare", "--initial-branch=main", str(remote))
        # Another project is already on the branch.
        seed = directory / "seed"
        self.git("init", "--initial-branch=gh-pages", str(seed))
        (seed / "gnoblin" / "stable").mkdir(parents=True)
        (seed / "gnoblin/stable/manifest.json").write_text("{}")
        self.git("add", "--all", cwd=seed)
        self.git("-c", "commit.gpgsign=false", "commit", "-m", "seed", cwd=seed)
        self.git("push", str(remote), "gh-pages", cwd=seed)

        build_tree(directory / "tree", packages={"fedora/43/x86_64/a-1.rpm": b"one"})
        for round_number in (1, 2):
            publish.publish(
                "pages", directory / "tree", "bingux", "nightly", str(remote), previous=None, apply=True, environment={}
            )
        clone = directory / "clone"
        self.git("clone", "--branch", "gh-pages", str(remote), str(clone))
        self.assertTrue((clone / "bingux/nightly/fedora/43/x86_64/a-1.rpm").exists())
        self.assertTrue((clone / "gnoblin/stable/manifest.json").exists(), "the other project is kept")
        self.assertTrue((clone / ".nojekyll").exists())
        self.assertEqual(
            self.git("rev-list", "--count", "HEAD", cwd=clone).strip(), "1", "one snapshot commit, not history"
        )


if __name__ == "__main__":
    unittest.main()

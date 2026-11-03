#!/usr/bin/env python3
"""Layer 4 of the package suite: put the packages of one target into a signed repository tree with the repository tool
and throw-away keys, as the repository stage does, and check the tree the way a client would.

usage: repository.py TARGET_JSON PACKAGE_DIR REPORT_DIR

Runs in the image of the repository tool (scripts/pkgrepo/container), which has createrepo_c, rpmsign, apt-ftparchive,
repo-add, gpg and openssl. scripts/run-package-tests.sh starts it. design/package-testing.md lists the failure modes
P1 to P4.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))

import targets  # noqa: E402
from formats import format_of  # noqa: E402
from report import Layer  # noqa: E402

PKGREPO = [sys.executable, "-m", "pkgrepo"]
RPM_ARCH = {"amd64": "x86_64", "arm64": "aarch64"}


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    environment = {**os.environ, "PYTHONPATH": str(ROOT / "scripts")}
    return subprocess.run(command, capture_output=True, text=True, env=environment, **kwargs)


def layout(target: dict, path: Path) -> str | None:
    """Where scripts/pkgrepo build expects a package: fedora/<release>/, opensuse/<release>/, deb/<suite>/, arch/,
    alpine/<branch>/<arch>/. A tar package is a release asset and no repository holds it."""
    fmt = format_of(path)
    if fmt == "rpm":
        fedora = re.search(r"\.fc(\d+)\.[a-z0-9_]+\.rpm$", path.name)
        return f"fedora/{fedora.group(1)}" if fedora else f"opensuse/{target['slug']}"
    if fmt == "deb":
        return f"deb/{target['slug']}"
    if fmt == "archlinux":
        return "arch"
    if fmt == "apk":
        return f"alpine/{target['slug']}/{RPM_ARCH[target['arch']]}"
    return None


def main() -> int:
    target = json.loads(Path(sys.argv[1]).read_text())
    package_dir, report_dir = Path(sys.argv[2]), Path(sys.argv[3])
    layer = Layer("repository")
    files = sorted(path for path in package_dir.iterdir() if path.is_file())
    work = Path(tempfile.mkdtemp(prefix="gnoblin-repository-"))
    log = report_dir / "evidence" / "repository"
    log.mkdir(parents=True, exist_ok=True)

    # P4: a target that is not published writes its packages to the testpkgs artifact, which no release takes.
    errors = []
    if targets.own_lane(target) and not target["published"] and target["install_artifact"] != "testpkgs":
        errors.append(f"{target['name']} is not published, and its packages are in {target['install_artifact']}")
    if target["install_artifact"] == "packages" and not targets.own_lane(target):
        for path in files:
            if re.search(r"-\d+~[^_]+_[a-z0-9]+\.deb$", path.name):
                errors.append(f"{path.name} is a per-release deb in a published artifact")
    layer.check("P4", "unpublished packages stay out of published artifacts", target["slug"], errors)

    start = time.time()
    packages = work / "packages"
    placed = []
    for path in files:
        where = layout(target, path)
        if where is None:
            continue
        (packages / where).mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, packages / where / path.name)
        placed.append((path, where))
    home = work / "gnupg"
    home.mkdir(mode=0o700)
    steps = [
        [
            "gpg",
            "--homedir",
            str(home),
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-generate-key",
            "package suite test key <noreply@example.invalid>",
            "rsa3072",
            "sign",
            "1d",
        ],
        ["openssl", "genrsa", "-out", str(work / "apk.pem"), "3072"],
    ]
    errors = []
    for step in steps:
        result = run(step)
        if result.returncode != 0:
            errors.append(f"{step[0]}: {result.stderr.strip()}")
    tree = work / "tree"
    if not errors:
        result = run(
            [
                *PKGREPO,
                "build",
                "--project",
                "gnoblin",
                "--channel",
                "nightly",
                "--packages",
                str(packages),
                "--output",
                str(tree),
                "--gnupg-home",
                str(home),
                "--apk-key",
                str(work / "apk.pem"),
                "--base-url",
                "https://repo.invalid",
                "--replace-stable",
            ]
        )
        (log / "build.log").write_text(result.stdout + result.stderr)
        if result.returncode != 0:
            errors.append(f"pkgrepo build failed: {(result.stdout + result.stderr).strip()[-2000:]}")
    layer.add(
        "P1",
        "the repository tool takes every package",
        "fail" if errors else "pass",
        "\n".join(errors),
        time.time() - start,
    )
    if errors:
        for case_id, title in (
            ("P2", "manifest and metadata match the files"),
            ("P3", "signatures verify with the test key"),
        ):
            layer.add(case_id, title, "fail", "not run: P1 failed")
        layer.write(report_dir)
        return 1

    # P2: every package is in manifest.json with its size and SHA-256, and pkgrepo verify accepts the tree.
    errors = []
    manifests = sorted(tree.rglob("manifest.json"))
    if len(manifests) != 1:
        layer.check(
            "P2",
            "manifest and metadata match the files",
            target["slug"],
            [f"{len(manifests)} manifest.json files in the tree"],
        )
        layer.add("P3", "signatures verify with the test key", "fail", "not run: no manifest")
        layer.write(report_dir)
        return 1
    channel_root = manifests[0].parent
    manifest = json.loads(manifests[0].read_text())
    entries = {item["filename"]: item for item in manifest["packages"]}
    for path, where in placed:
        item = entries.get(path.name)
        if item is None:
            errors.append(f"{path.name} is not in manifest.json")
            continue
        stored = channel_root / item["path"]
        if stored.is_file() and item["size"] != stored.stat().st_size:
            errors.append(f"{item['path']}: manifest size {item['size']}, file {stored.stat().st_size}")
        # pkgrepo signs rpms in place, so the stored rpm differs from the input. Its manifest entry must match the
        # file in the tree.
        if stored.is_file() and hashlib.sha256(stored.read_bytes()).hexdigest() != item["sha256"]:
            errors.append(f"{item['path']}: the SHA-256 of manifest.json does not match the file in the tree")
        if not stored.is_file():
            errors.append(f"{item['path']} is in manifest.json and not in the tree")
    verify = run([*PKGREPO, "verify", "--project", "gnoblin", "--channel", "nightly", "--tree", str(tree)])
    (log / "verify.log").write_text(verify.stdout + verify.stderr)
    if verify.returncode != 0:
        errors.append(f"pkgrepo verify: {(verify.stdout + verify.stderr).strip()[-2000:]}")
    layer.check("P2", "manifest and metadata match the files", target["slug"], errors)

    # P3: the signatures verify with the public half of the test key, as a client that imported it would check them.
    errors = []
    public = work / "key.asc"
    public.write_text(run(["gpg", "--homedir", str(home), "--armor", "--export"]).stdout)
    rpms = sorted(tree.rglob("*.rpm"))
    if rpms:
        rpmdb = work / "rpmdb"
        rpmdb.mkdir()
        run(["rpmkeys", "--dbpath", str(rpmdb), "--import", str(public)])
        for rpm in rpms:
            result = run(["rpmkeys", "--dbpath", str(rpmdb), "--checksig", "-v", str(rpm)])
            if result.returncode != 0 or "NOT OK" in result.stdout or "Signature" not in result.stdout:
                errors.append(f"{rpm.name}: {result.stdout.strip()} {result.stderr.strip()}")
    for release in tree.rglob("InRelease"):
        result = run(["gpg", "--homedir", str(home), "--batch", "--verify", str(release)])
        if result.returncode != 0:
            errors.append(f"{release.relative_to(tree)}: {result.stderr.strip()}")
    for signature in tree.rglob("*.pkg.tar.zst.sig"):
        result = run(["gpg", "--homedir", str(home), "--verify", str(signature), str(signature.with_suffix(""))])
        if result.returncode != 0:
            errors.append(f"{signature.relative_to(tree)}: {result.stderr.strip()}")
    checked = len(rpms) + len(list(tree.rglob("InRelease"))) + len(list(tree.rglob("*.sig")))
    if not checked and not list(tree.rglob("APKINDEX.tar.gz")):
        errors.append("the tree holds no signature to check")
    layer.check(
        "P3",
        f"signatures verify with the test key ({checked} checked, the Alpine index by pkgrepo verify)",
        target["slug"],
        errors,
    )
    layer.write(report_dir)
    shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

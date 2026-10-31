"""Upload a repository tree. The tree is plain static files, so any file host works; a back end only moves the files.

Every back end uploads in the same order, so a client never sees metadata that names a file that is not there yet:

  1. package files that are new or changed (a package file never changes in place, so most runs upload a few),
  2. metadata and pages (repodata/, dists/, the pacman databases, manifest.json, index pages, the key),
  3. removal of files the new tree no longer has.

The default is a dry run that prints the plan. --apply is the only way anything leaves the machine.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from .runner import quote, require, run

PACKAGE_SUFFIXES = (".rpm", ".deb")
PACKAGE_GLOBS = ("*.rpm", "*.deb", "*.pkg.tar.*")


def is_package_file(relative: str) -> bool:
    """Package files and their signatures. Database files (name.db.tar.zst) are metadata."""
    name = relative.rsplit("/", 1)[-1]
    if name.endswith(PACKAGE_SUFFIXES):
        return True
    return ".pkg.tar." in name


@dataclass
class Step:
    description: str
    command: list[str] | None = None
    action: Callable[[], None] | None = None
    env: dict[str, str] = field(default_factory=dict)


@dataclass
class Plan:
    steps: list[Step] = field(default_factory=list)
    cleanup: list[Callable[[], None]] = field(default_factory=list)

    def show(self) -> str:
        lines = []
        for number, step in enumerate(self.steps, 1):
            lines.append(f"{number}. {step.description}")
            if step.command:
                lines.append("   " + quote(step.command))
        return "\n".join(lines)

    def execute(self) -> None:
        try:
            for step in self.steps:
                print(f"== {step.description}", flush=True)
                if step.command:
                    run(step.command, env=step.env or None)
                if step.action:
                    step.action()
        finally:
            for clean in self.cleanup:
                clean()


def tree_files(root: Path, scope: str) -> list[str]:
    """Files of the channel (paths relative to root) and, separately handled, the files at the site root."""
    base = root / scope
    return sorted(str(path.relative_to(root)) for path in base.rglob("*") if path.is_file())


def site_files(root: Path) -> list[str]:
    return sorted(name for name in ("gpg.key", "index.html") if (root / name).is_file())


def changed_packages(root: Path, scope: str, previous: dict[str, str] | None) -> list[str]:
    """Package files (and their .sig files) that are new, or whose package differs from the published manifest.

    previous maps a package file name to its sha256 (see previous_hashes). Names are unique across a channel because
    they carry the release tag (.fc45) or the version; a signature counts as changed when its package does.
    """
    from .packages import sha256_of

    changed = []
    for relative in tree_files(root, scope):
        if not is_package_file(relative):
            continue
        main = relative[: -len(".sig")] if relative.endswith(".sig") else relative
        if previous is None or previous.get(main.rsplit("/", 1)[-1]) != sha256_of(root / main):
            changed.append(relative)
    return changed


def previous_hashes(existing: Path | None, scope: str) -> dict[str, str] | None:
    """File name -> sha256 of the published packages, from a pulled copy of the channel; None when nothing is published."""
    if existing is None:
        return None
    manifest = existing / scope / "manifest.json"
    if not manifest.exists():
        return None
    import json

    return {item["filename"]: item["sha256"] for item in json.loads(manifest.read_text())["packages"]}


# --- rsync -----------------------------------------------------------------------------------------------------------


def _write_secret(directory: Path, name: str, value: str) -> Path:
    path = directory / name
    path.write_text(value if value.endswith("\n") else value + "\n")
    path.chmod(stat.S_IRUSR | stat.S_IWUSR)
    return path


def plan_rsync(root: Path, scope: str, destination: str, changed: list[str], *, env: dict[str, str]) -> Plan:
    """rsync to a path on a server (user@host:/srv/repo) or a local directory.

    REPO_RSYNC_KEY holds an SSH private key and REPO_RSYNC_KNOWN_HOSTS the server's host key line; without them
    rsync uses the ambient SSH configuration.
    """
    plan = Plan()
    scratch = Path(tempfile.mkdtemp(prefix="pkgrepo-rsync-"))
    plan.cleanup.append(lambda: shutil.rmtree(scratch, ignore_errors=True))
    ssh_options: list[str] = []
    remote = ":" in destination.split("/", 1)[0]
    if remote and env.get("REPO_RSYNC_KEY"):
        key = _write_secret(scratch, "key", env["REPO_RSYNC_KEY"])
        hosts = _write_secret(scratch, "known_hosts", env.get("REPO_RSYNC_KNOWN_HOSTS", ""))
        ssh = f"ssh -i {key} -o IdentitiesOnly=yes -o UserKnownHostsFile={hosts} -o StrictHostKeyChecking=yes"
        ssh_options = ["-e", ssh]
    options = ["-rt", "--chmod=D755,F644", *ssh_options]
    source = f"{root}/"
    target = destination.rstrip("/") + "/"
    package_filter = ["--include=*/", *[f"--include={g}" for g in PACKAGE_GLOBS], "--exclude=*"]
    changed_list = scratch / "changed.txt"
    changed_list.write_text("".join(path + "\n" for path in changed))
    plan.steps.append(
        Step(
            f"upload {len(changed)} new or changed package files",
            ["rsync", *options, f"--files-from={changed_list}", source, target],
        )
    )
    plan.steps.append(
        Step(
            "upload package files the destination lacks",
            ["rsync", *options, "--size-only", *package_filter, f"{root / scope}/", f"{target}{scope}/"],
        )
    )
    metadata = [f"--exclude={g}" for g in PACKAGE_GLOBS]
    plan.steps.append(
        Step(
            "upload metadata and pages",
            ["rsync", *options, *metadata, f"{root / scope}/", f"{target}{scope}/"],
        )
    )
    for name in site_files(root):
        plan.steps.append(Step(f"upload {name}", ["rsync", *options, str(root / name), target + name]))
    plan.steps.append(
        Step(
            "remove files the new tree no longer has",
            [
                "rsync",
                "-r",
                "--delete",
                "--existing",
                "--ignore-existing",
                *ssh_options,
                f"{root / scope}/",
                f"{target}{scope}/",
            ],
        )
    )
    return plan


# --- S3 and compatible stores (Cloudflare R2, MinIO) -------------------------------------------------------------------


def plan_s3(root: Path, scope: str, destination: str, changed: list[str], *, env: dict[str, str]) -> Plan:
    """aws s3 to s3://bucket/prefix. AWS_ENDPOINT_URL (R2, MinIO) and the usual AWS_* credentials come from the environment."""
    if not destination.startswith("s3://"):
        raise ValueError("the s3 destination must look like s3://bucket/prefix")
    plan = Plan()
    base = destination.rstrip("/")
    package_filter = ["--exclude", "*", *[arg for g in PACKAGE_GLOBS for arg in ("--include", g)]]
    metadata_filter = [arg for g in PACKAGE_GLOBS for arg in ("--exclude", g)]
    for relative in changed:
        plan.steps.append(
            Step(
                f"upload changed package {relative}",
                ["aws", "s3", "cp", "--only-show-errors", str(root / relative), f"{base}/{relative}"],
            )
        )
    plan.steps.append(
        Step(
            "upload package files the bucket lacks",
            [
                "aws",
                "s3",
                "sync",
                "--only-show-errors",
                "--size-only",
                f"{root / scope}",
                f"{base}/{scope}",
                *package_filter,
            ],
        )
    )
    plan.steps.append(
        Step(
            "upload metadata and pages",
            ["aws", "s3", "sync", "--only-show-errors", f"{root / scope}", f"{base}/{scope}", *metadata_filter],
        )
    )
    for name in site_files(root):
        plan.steps.append(
            Step(f"upload {name}", ["aws", "s3", "cp", "--only-show-errors", str(root / name), f"{base}/{name}"])
        )
    plan.steps.append(
        Step(
            "remove package files the new tree no longer has",
            [
                "aws",
                "s3",
                "sync",
                "--only-show-errors",
                "--size-only",
                "--delete",
                f"{root / scope}",
                f"{base}/{scope}",
                *package_filter,
            ],
        )
    )
    plan.steps.append(
        Step(
            "remove metadata the new tree no longer has",
            [
                "aws",
                "s3",
                "sync",
                "--only-show-errors",
                "--delete",
                f"{root / scope}",
                f"{base}/{scope}",
                *metadata_filter,
            ],
        )
    )
    return plan


# --- GitHub Pages ------------------------------------------------------------------------------------------------------


def plan_pages(
    root: Path, scope: str, destination: str, changed: list[str], *, env: dict[str, str], branch: str = "gh-pages"
) -> Plan:
    """Replace this channel in a branch of a Git repository served by GitHub Pages, as one commit.

    The branch is rewritten to a single commit on every publication (force push), so the history of binaries never
    accumulates in the repository. REPO_PAGES_TOKEN, when set, authenticates over HTTPS.
    """
    plan = Plan()
    work = Path(tempfile.mkdtemp(prefix="pkgrepo-pages-"))
    plan.cleanup.append(lambda: shutil.rmtree(work, ignore_errors=True))
    git = ["git", "-C", str(work)]
    config: list[str] = []
    if env.get("REPO_PAGES_TOKEN"):
        import base64

        credentials = base64.b64encode(f"x-access-token:{env['REPO_PAGES_TOKEN']}".encode()).decode()
        config = ["-c", f"http.extraheader=AUTHORIZATION: basic {credentials}"]
    steps = plan.steps

    def fetch_branch() -> None:
        run(["git", "init", "--quiet", str(work)], quiet=True)
        run([*git, "remote", "add", "origin", destination], quiet=True)
        found = run([*git, *config, "ls-remote", "--heads", "origin", branch], capture=True, quiet=True).strip()
        if found:
            run([*git, *config, "fetch", "--quiet", "--depth", "1", "origin", branch], quiet=True)
            run([*git, "checkout", "--quiet", "-B", "snapshot", "FETCH_HEAD"], quiet=True)
        else:
            run([*git, "checkout", "--quiet", "--orphan", "snapshot"], quiet=True)

    def replace_channel() -> None:
        target = work / scope
        shutil.rmtree(target, ignore_errors=True)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(root / scope, target)
        for name in site_files(root):
            shutil.copyfile(root / name, work / name)
        (work / ".nojekyll").write_text("")

    def commit_and_push() -> None:
        run([*git, "add", "--all"], quiet=True)
        if not run([*git, "status", "--porcelain"], capture=True, quiet=True).strip():
            print("the branch already has exactly this content; nothing to push")
            return
        # A new root commit, so the branch never accumulates the history of old binaries.
        run([*git, "checkout", "--quiet", "--orphan", "next"], quiet=True)
        # A configured identity is used as it is; a CI container has none, so the commit gets a fixed one.
        configured = subprocess.run(
            [*git, "config", "--get", "user.email"], capture_output=True, text=True
        ).stdout.strip()
        identity = [] if configured else ["-c", "user.name=pkgrepo", "-c", "user.email=pkgrepo@localhost"]
        run(
            [*git, "-c", "commit.gpgsign=false", *identity, "commit", "--quiet", "--message", f"Publish {scope}"],
            quiet=True,
        )
        run([*git, *config, "push", "--force", "origin", f"next:{branch}"], quiet=True)

    steps.append(Step(f"fetch branch {branch} of {destination} (or start it)", action=fetch_branch))
    steps.append(Step(f"replace {scope}/ and the site files in the working copy", action=replace_channel))
    steps.append(Step(f"commit as one snapshot and force-push to {branch}", action=commit_and_push))
    return plan


BACKENDS = {"rsync": plan_rsync, "s3": plan_s3, "pages": plan_pages}


def publish(
    backend: str,
    root: Path,
    project: str,
    channel: str,
    destination: str,
    *,
    previous: Path | None,
    apply: bool,
    environment: dict[str, str] | None = None,
    branch: str | None = None,
) -> Plan:
    if backend not in BACKENDS:
        raise ValueError(f"unknown back end {backend!r}; choose one of {', '.join(BACKENDS)}")
    scope = f"{project}/{channel}"
    if not (root / scope / "manifest.json").is_file():
        raise ValueError(f"{root / scope} is not a built channel (no manifest.json)")
    env = dict(os.environ if environment is None else environment)
    changed = changed_packages(root, scope, previous_hashes(previous, scope))
    if apply:
        require({"rsync": "rsync", "s3": "aws", "pages": "git"}[backend])
    kwargs = {"branch": branch} if backend == "pages" and branch else {}
    plan = BACKENDS[backend](root, scope, destination, changed, env=env, **kwargs)
    print(f"{'Publishing' if apply else 'Dry run of publishing'} {scope} with {backend} to {destination}")
    print(plan.show())
    if apply:
        plan.execute()
        print("published")
    else:
        for clean in plan.cleanup:
            clean()
        print("dry run: nothing was uploaded. Pass --apply to publish.")
    return plan

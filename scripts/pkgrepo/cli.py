"""Command line: version, key-import, needs-build, pull, build, verify, publish, package."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
from datetime import datetime, timezone
from pathlib import Path

from . import check, download, package, publish as publisher, signing, tree, versions
from .runner import CommandError

DEFAULT_BASE_URL = "https://repo.gnoblin.org"


def _git(directory: Path, *arguments: str) -> str:
    result = subprocess.run(["git", "-C", str(directory), *arguments], capture_output=True, text=True)
    if result.returncode != 0:
        raise CommandError(f"git {' '.join(arguments)} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def describe_git(directory: Path, tag_prefix: str) -> tuple[str, int, str, str]:
    """(last stable version, commits since its tag, short commit, commit date as YYYYMMDD in UTC) of HEAD."""
    commit = _git(directory, "rev-parse", "--short=7", "HEAD")
    stamp = int(_git(directory, "log", "-1", "--format=%ct", "HEAD"))
    date = datetime.fromtimestamp(stamp, timezone.utc).strftime("%Y%m%d")
    try:
        tag = _git(directory, "describe", "--tags", "--abbrev=0", "--match", f"{tag_prefix}[0-9]*", "HEAD")
    except CommandError:
        # No release yet: every commit counts, and the version in the tree (which is ahead of 0.0.0) names the release
        # that is coming.
        return "0.0.0", int(_git(directory, "rev-list", "--count", "HEAD")), commit, date
    count = int(_git(directory, "rev-list", "--count", f"{tag}..HEAD"))
    return tag[len(tag_prefix) :], count, commit, date


def cmd_version(args: argparse.Namespace) -> int:
    if args.git:
        last, count, commit, date = describe_git(Path(args.git), args.tag_prefix)
        last = args.last_stable or last
        count = args.count if args.count is not None else count
        commit = args.commit or commit
        date = args.date or date
    else:
        missing = [name for name in ("last_stable", "count", "commit", "date") if getattr(args, name) is None]
        if missing:
            raise CommandError("without --git, give " + ", ".join("--" + m.replace("_", "-") for m in missing))
        last, count, commit, date = args.last_stable, args.count, args.commit, args.date
    value = versions.nightly(last, args.tree_version, date, count, commit)
    values = {
        "base": value.base,
        "rpm": value.rpm,
        "deb": value.deb,
        "arch": value.arch,
        "semver": value.semver,
        "date": value.date,
        "count": str(value.count),
        "commit": value.commit,
    }
    if args.format == "json":
        print(json.dumps(values, indent=2, sort_keys=True))
    elif args.format == "lines":
        for key, text in values.items():
            print(f"{key}={text}")
    else:
        print(values[args.format])
    return 0


def cmd_key_import(args: argparse.Namespace) -> int:
    armored = os.environ.get(args.key_env)
    if not armored:
        raise CommandError(f"the environment variable {args.key_env} is empty or not set")
    home = signing.gpg_home(Path(args.gnupg_home))
    print(signing.import_private_key(home, armored))
    return 0


def cmd_needs_build(args: argparse.Namespace) -> int:
    """Print needs_build=true when the published channel was built from another commit (or is not published yet)."""
    url = f"{args.url.rstrip('/')}/{args.project}/{args.channel}/manifest.json"
    try:
        with download.open_url(url) as response:
            published = json.loads(response.read())["source"]["commit"]
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        published = ""
    except urllib.error.URLError as error:
        if not args.allow_unreachable:
            raise CommandError(f"cannot reach {url}: {error.reason}") from error
        print(f"pkgrepo: cannot reach {url} ({error.reason}); assuming a build is needed", file=sys.stderr)
        published = ""
    needed = published != args.commit
    print(f"needs_build={'true' if needed else 'false'}")
    print(f"published_commit={published}", file=sys.stderr)
    return 0


def cmd_pull(args: argparse.Namespace) -> int:
    count = tree.pull(
        args.url, args.project, args.channel, Path(args.output), args.allow_missing, args.allow_unreachable
    )
    print(f"pulled {count} packages of {args.project}/{args.channel}")
    return 0


def _signer(args: argparse.Namespace) -> signing.Signer:
    if args.gnupg_home:
        home = signing.gpg_home(Path(args.gnupg_home))
    elif os.environ.get(args.key_env):
        home = signing.gpg_home()
        signing.import_private_key(home, os.environ[args.key_env])
    else:
        raise CommandError(f"give --gnupg-home, or put the private key in {args.key_env}")
    return signing.Signer.from_environment(home, args.key_id, args.passphrase_env)


def cmd_build(args: argparse.Namespace) -> int:
    settings = tree.Settings(
        project=args.project,
        channel=args.channel,
        input_dir=Path(args.packages),
        output=Path(args.output),
        existing=Path(args.existing) if args.existing else None,
        keep_nightlies=args.keep_nightlies,
        replace_stable=args.replace_stable,
        source_commit=args.source_commit,
        source_ref=args.source_ref,
        base_url=args.base_url.rstrip("/"),
        default_arch=args.default_arch,
    )
    with _signer(args) as signer:
        manifest = tree.build(settings, signer)
    print(
        f"built {args.project}/{args.channel}: {len(manifest['packages'])} packages, key {manifest['key']['fingerprint']}"
    )
    problems = check.verify(Path(args.output), args.project, args.channel)
    for problem in problems:
        print(f"verify: {problem}", file=sys.stderr)
    return 1 if problems else 0


def cmd_verify(args: argparse.Namespace) -> int:
    problems = check.verify(Path(args.tree), args.project, args.channel)
    for problem in problems:
        print(f"verify: {problem}", file=sys.stderr)
    print("verified" if not problems else f"{len(problems)} problems")
    return 1 if problems else 0


def cmd_package(args: argparse.Namespace) -> int:
    root = Path(args.root)
    manifests = {package.package_name(path): path for path in package.find_manifests(root)}
    if args.list:
        for name, path in sorted(manifests.items()):
            print(f"{name}\t{path.relative_to(root)}")
        return 0
    for option in ("stage", "version", "output"):
        if not getattr(args, option):
            raise CommandError(f"--{option} is required")
    if unknown := [name for name in args.package or [] if name not in manifests]:
        raise CommandError(f"no nfpm.yaml for: {', '.join(unknown)}")
    formats = args.format.split(",")
    if bad := [name for name in formats if name not in package.FORMATS]:
        raise CommandError(f"unknown format: {', '.join(bad)}")
    chosen = [path for name, path in sorted(manifests.items()) if not args.package or name in args.package]
    table = package.load_table(root / "packaging" / "dependencies.json", *(Path(p) for p in args.dependencies or []))
    built = package.build(
        chosen,
        stage=Path(args.stage),
        version=args.version,
        output=Path(args.output),
        formats=formats,
        arch=args.arch,
        table=table,
    )
    print(f"built {len(built)} packages: {', '.join(built)}")
    return 0


def cmd_publish(args: argparse.Namespace) -> int:
    publisher.publish(
        args.backend,
        Path(args.tree),
        args.project,
        args.channel,
        args.destination,
        previous=Path(args.previous) if args.previous else None,
        apply=args.apply,
        branch=args.branch,
    )
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="pkgrepo", description=__doc__)
    sub = root.add_subparsers(dest="command", required=True)

    def channel_arguments(p: argparse.ArgumentParser) -> None:
        p.add_argument("--project", required=True, choices=("gnoblin", "bingux"))
        p.add_argument("--channel", required=True, choices=versions.CHANNELS)

    p = sub.add_parser("version", help="print the nightly version of a commit in every package format")
    p.add_argument(
        "--git", help="read the last stable tag, the commit count, the commit and its date from this checkout"
    )
    p.add_argument("--tag-prefix", default="v", help="prefix of stable tags (gnoblin-v, v); default %(default)s")
    p.add_argument("--last-stable")
    p.add_argument(
        "--tree-version", help="the version in the tree; it wins when it is already newer than the last stable"
    )
    p.add_argument("--count", type=int)
    p.add_argument("--commit")
    p.add_argument("--date", help="YYYYMMDD; default the commit date")
    p.add_argument("--format", default="lines", choices=("lines", "json", "base", "rpm", "deb", "arch", "semver"))
    p.set_defaults(run=cmd_version)

    p = sub.add_parser("key-import", help="import the private key from an environment variable into a GNUPGHOME")
    p.add_argument("--gnupg-home", required=True)
    p.add_argument("--key-env", default=signing.PRIVATE_KEY_ENV)
    p.set_defaults(run=cmd_key_import)

    p = sub.add_parser("needs-build", help="has the main branch moved since the published nightly was built?")
    channel_arguments(p)
    p.add_argument("--url", default=DEFAULT_BASE_URL)
    p.add_argument("--commit", required=True)
    p.add_argument(
        "--allow-unreachable", action="store_true", help="treat a host that cannot be reached as not published"
    )
    p.set_defaults(run=cmd_needs_build)

    p = sub.add_parser("pull", help="download the published packages of a channel, to merge a build with them")
    channel_arguments(p)
    p.add_argument("--url", default=DEFAULT_BASE_URL)
    p.add_argument("--output", required=True)
    p.add_argument("--allow-missing", action="store_true", help="a channel that is not published yet is not an error")
    p.add_argument(
        "--allow-unreachable",
        action="store_true",
        help="a host that cannot be reached is not an error either. Only for runs that publish nothing: "
        "a publication built without the published packages would remove them",
    )
    p.set_defaults(run=cmd_pull)

    p = sub.add_parser("build", help="produce the signed repository tree of a channel")
    channel_arguments(p)
    p.add_argument(
        "--packages",
        required=True,
        help="directory of built packages: fedora/<release>/, opensuse/<release>/, deb/<suite>/, arch/",
    )
    p.add_argument("--output", required=True, help="directory that receives the tree")
    p.add_argument("--existing", help="a pulled copy of the published channel to merge with (pkgrepo pull --output)")
    p.add_argument(
        "--keep-nightlies", type=int, default=14, help="newest versions of each package kept in the nightly channel"
    )
    p.add_argument(
        "--replace-stable", action="store_true", help="allow a stable package that is already published to be replaced"
    )
    p.add_argument(
        "--source-commit", default="", help="the commit the packages were built from; recorded in the manifest"
    )
    p.add_argument("--source-ref", default="")
    p.add_argument("--base-url", default=DEFAULT_BASE_URL)
    p.add_argument(
        "--default-arch",
        default="x86_64",
        help="architecture directory for a release with only architecture-independent packages",
    )
    p.add_argument("--gnupg-home")
    p.add_argument("--key-id", help="fingerprint of the signing key; default: the only secret key")
    p.add_argument("--key-env", default=signing.PRIVATE_KEY_ENV)
    p.add_argument("--passphrase-env", default=signing.PASSPHRASE_ENV)
    p.set_defaults(run=cmd_build)

    p = sub.add_parser("verify", help="check checksums and signatures of a built tree like a client would")
    channel_arguments(p)
    p.add_argument("--tree", required=True)
    p.set_defaults(run=cmd_verify)

    p = sub.add_parser("publish", help="upload a built tree (a dry run unless --apply)")
    channel_arguments(p)
    p.add_argument("--tree", required=True)
    p.add_argument("--backend", required=True, choices=sorted(publisher.BACKENDS))
    p.add_argument(
        "--destination", required=True, help="rsync: user@host:/path; s3: s3://bucket/prefix; pages: git URL"
    )
    p.add_argument("--branch", help="pages: the branch GitHub Pages serves (default gh-pages)")
    p.add_argument("--previous", help="the pulled copy of the published channel, to upload only changed packages")
    p.add_argument("--apply", action="store_true", help="upload for real. Without it, only the plan is printed")
    p.set_defaults(run=cmd_publish)

    p = sub.add_parser("package", help="build packages from the nfpm.yaml files beside the code")
    p.add_argument("--root", default=".", help="source tree to search for nfpm.yaml; default the current directory")
    p.add_argument("--list", action="store_true", help="print the packages found and stop")
    p.add_argument("--stage", help="staged prefix the manifests read from (${STAGE})")
    p.add_argument("--version", help="semver version, for example 0.1.0 or 0.1.1-nightly.20261031.5.gabc1234")
    p.add_argument("--output", help="directory that receives the packages")
    p.add_argument("--format", default=",".join(package.FORMATS), help="comma list; default %(default)s")
    p.add_argument("--arch", default="amd64", help="Go architecture name, amd64 or arm64; default %(default)s")
    p.add_argument("--package", action="append", help="build only this package (repeatable)")
    p.add_argument("--dependencies", action="append", help="extra dependency table (JSON) on top of the shared one")
    p.set_defaults(run=cmd_package)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        return args.run(args)
    except (CommandError, tree.TreeError, ValueError) as error:
        print(f"pkgrepo: {error}", file=sys.stderr)
        return 1

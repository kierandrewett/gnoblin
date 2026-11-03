#!/usr/bin/env python3
"""The release gate: which packages may be published, the coverage table, and the verdict of every target.

usage: tests/packages/gate.py --targets FILE --reports DIR --artifacts DIR --release-tag TAG --out DIR
                              [--stages DIR] [--graphical DIR] [--today YYYY-MM-DD]

FILE holds the outputs of scripts/supported-targets.py --rows as one JSON object: matrix, rows, architectures and
graphical_packaged (the graphical jobs). The other folders are searched at any depth, so one folder can hold all of
them (one folder for each target, as the verdict artifacts of .github/workflows/target.yml are laid out):

  --stages     stage.json of each build stage: {"stage": "prefix" | "packages", "slug", "arch",
               "status": "pass" | "fail" | "not run", "detail", "check" (optional), "log" (optional)}
  --reports    report.json of the package suite (tests/packages/suite.py), one for each target
  --graphical  result.json of the graphical VM test (tests/graphical/harness.py), one for each target
  --artifacts  the package artifacts, one folder for each, named as the workflows name them

A target is one release on one architecture. It passes when every stage passed, in this order:

  prefix     its own prefix built. A target that installs the shared packages has no prefix of its own ("shared")
  packages   its packages were written (by its own lane, or by the shared packages job of its architecture)
  suite      every layer of the package suite passed
  graphical  the graphical VM test passed on that architecture

A stage after a failed stage is "not run: <stage> failed", never a pass. A stage without a record after a passed stage
is a failure for the suite and the graphical test (the job did not finish), and "not run" for a build stage. A target
on an architecture without a graphical job is "not run" there. A planned release (no job, rows of --rows) is a verdict
too: "not built", with its blocker.

A package of a published artifact (packages-<tag>-...) is in the publish set only when every target and architecture
that installs it passed every stage. A package that is the same file on several architectures needs all of them. The
gate writes:

  OUT/coverage.md     the table of every listed release: each stage of each target on each architecture
  OUT/coverage.json   the same, the verdict of every target ("targets"), the exclusions and the publish set
  OUT/publishable/    the publish set, one folder for each artifact, as publish and the repository stage read it

It exits 1 when a built target did not pass every stage, when a published artifact holds a file that no target
installs, when a per-release package would be published, or when the publish set is empty. A failed experimental
target is excluded and named, and does not change the exit status. The publish set and coverage.json are written in
every case: a target that passed is released even when another target failed, and the exit status keeps the run red.
coverage.json "left_out" names each target with a job that did not pass and the first stage that stopped it.
design/package-testing.md (G1 to G5) and design/release-pipeline.md.
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import report  # noqa: E402
import targets  # noqa: E402
from formats import format_of  # noqa: E402

PER_RELEASE_DEB = re.compile(r"-\d+~[^_]+_[a-z0-9]+\.deb$")
STAGES = ("prefix", "packages", "suite", "graphical")
PASS, FAIL, NOT_RUN, SHARED, NOT_BUILT = "pass", "fail", "not run", "shared", "not built"
BUILD_STAGES = ("prefix", "packages")


def by_target(directory: Path | None, name: str) -> dict[tuple[str, str], list[dict]]:
    """Every file called name under directory, read as JSON, by (slug, arch)."""
    found: dict[tuple[str, str], list[dict]] = {}
    if directory is None or not directory.is_dir():
        return found
    for path in sorted(directory.rglob(name)):
        data = json.loads(path.read_text())
        if name == "report.json":
            key = (data.get("target", {}).get("slug"), data.get("target", {}).get("arch"))
        else:
            key = (data.get("slug"), data.get("arch", "amd64"))
        found.setdefault(key, []).append(data)
    return found


def suite_stage(data: dict) -> dict:
    reasons = []
    for name in report.LAYERS:
        layer = data.get("layers", {}).get(name)
        if layer is None or layer.get("status") == "missing":
            reasons.append(f"{name}: missing")
        elif layer.get("status") != report.PASS:
            failed = [case["id"] for case in layer.get("cases", []) if case.get("status") == report.FAIL]
            reasons.append(f"{name}: failed {', '.join(dict.fromkeys(failed)) or '(no case)'}")
    passed = len(report.LAYERS) - len(reasons)
    if not reasons:
        return {"status": PASS, "check": "", "detail": f"{passed}/{len(report.LAYERS)} layers"}
    first = reasons[0].split(":")[0]
    return {"status": FAIL, "check": first, "detail": "; ".join(reasons)}


def graphical_stage(data: dict) -> dict:
    if data.get("result") == "pass":
        return {"status": PASS, "check": "", "detail": f"{len(data.get('checks', []))} checks"}
    if data.get("result") == "not packaged":
        return {"status": FAIL, "check": "packaged", "detail": data.get("reason") or "not packaged"}
    first = data.get("first_failure") or {}
    if not first:
        skipped = next((c for c in data.get("checks", []) if c.get("status") != "pass"), {})
        first = {"name": skipped.get("name", "unknown"), "detail": skipped.get("detail", "a check did not run")}
    return {"status": FAIL, "check": first.get("name", "unknown"), "detail": first.get("detail", "")}


def verdict(entry: dict, records: dict, graphical_jobs: dict) -> dict:
    """The stages of one target on one architecture, and whether it passed."""
    key = (entry["slug"], entry["arch"])
    raw: dict[str, dict | None] = {}
    for stage in BUILD_STAGES:
        found = [item for item in records["stages"].get(key, []) if item.get("stage") == stage]
        raw[stage] = (
            {"status": found[-1].get("status", FAIL), "check": found[-1].get("check", stage),
             "detail": found[-1].get("detail", ""), "log": found[-1].get("log", "")}
            if found else None
        )  # fmt: skip
    if entry["prefix"] != "own":
        raw["prefix"] = {"status": SHARED, "check": "", "detail": "installs the packages of the shared prefixes"}
    suite = records["reports"].get(key)
    raw["suite"] = suite_stage(suite[-1]) if suite else None
    graphical = records["graphical"].get(key)
    raw["graphical"] = graphical_stage(graphical[-1]) if graphical else None
    job = graphical_jobs.get(key)
    if job is not None and "image" in job and job["image"] is None:
        # No image to boot yet: the leg runs no graphical job, and nothing stands in for the result.
        cpu = (job.get("guest") or {}).get("cpu") or entry["arch"]
        missing = job.get("image_missing") or "no image data"
        detail = f"not run: no {cpu} image for {entry['name']} ({missing})"
        raw["graphical"] = {"status": NOT_RUN, "check": "image", "detail": detail}

    stages, blocked = {}, None
    for stage in STAGES:
        if blocked:
            stages[stage] = {"status": NOT_RUN, "check": "", "detail": f"not run: {blocked}"}
            continue
        record = raw[stage]
        if record is None:
            if stage in BUILD_STAGES:
                record = {"status": NOT_RUN, "check": "", "detail": f"not run: no record of the {stage} stage"}
            elif stage == "graphical" and key not in graphical_jobs:
                record = {"status": NOT_RUN, "check": "", "detail": f"not run: no graphical job on {entry['arch']}"}
            else:
                what = "report.json: the package suite" if stage == "suite" else "result.json: the graphical test"
                record = {"status": FAIL, "check": "no result", "detail": f"no {what} did not finish"}
        elif record["status"] == NOT_RUN and not record["detail"].startswith("not run"):
            record = {**record, "detail": f"not run: {record['detail'] or 'the job did not run'}"}
        stages[stage] = record
        if record["status"] == FAIL:
            blocked = f"{stage} failed"
        elif record["status"] == NOT_RUN:
            blocked = f"{stage} did not run"
    passed = all(stage["status"] in (PASS, SHARED) for stage in stages.values())
    first = next(((name, stage) for name, stage in stages.items() if stage["status"] not in (PASS, SHARED)), None)
    reason = f"{first[0]}: {first[1]['detail']}" if first else ""
    # A stage that did not run, with nothing failed before it, is a requirement that the run did not meet.
    unmet = first is not None and first[1]["status"] == NOT_RUN
    return {"stages": stages, "passed": passed, "reason": reason, "unmet": unmet}


def eol_of(name: str, rows: dict, graphical: dict, today: datetime.date) -> tuple[bool, str | None]:
    date = (rows.get(name) or {}).get("eol") or (graphical.get(name) or {}).get("eol_date")
    return (date is not None and datetime.date.fromisoformat(str(date)[:10]) <= today), date


def run(args) -> int:
    data = json.loads(args.targets.read_text())
    matrix = data["matrix"]
    architectures = [item["arch"] for item in data.get("architectures", [])] or sorted({e["arch"] for e in matrix})
    graphical_list = data.get("graphical_packaged")
    if graphical_list is None:
        graphical_list = [item for item in data.get("graphical", []) if item.get("packaged")]
    graphical_jobs = {(item["slug"], item["arch"]): item for item in graphical_list}
    graphical_by_name = {item["name"]: item for item in graphical_list}
    rows = data.get("rows") or [
        {"name": entry["name"], "state": "experimental" if entry["experimental"] else "built", "note": ""}
        for entry in matrix
        if entry["arch"] == architectures[0]
    ]
    rows_by_name = {row["name"]: row for row in rows}
    records = {
        "stages": by_target(args.stages, "stage.json"),
        "reports": by_target(args.reports, "report.json"),
        "graphical": by_target(args.graphical, "result.json"),
    }
    tag = args.release_tag
    status = {(entry["slug"], entry["arch"]): verdict(entry, records, graphical_jobs) for entry in matrix}

    errors, exclusions = [], []
    for entry in matrix:
        result = status[(entry["slug"], entry["arch"])]
        if result["passed"]:
            continue
        exclusions.append(f"excluded: {entry['name']} {entry['arch']}: {result['reason']}")
        if not entry["experimental"]:
            what = "unmet requirement of a built target" if result["unmet"] else "built target failed"
            errors.append(f"{what}: {entry['name']} {entry['arch']}: {result['reason']}")

    # The owners of every file of every artifact: the targets that install and test it.
    by_artifact: dict[str, list[dict]] = {}
    for entry in matrix:
        by_artifact.setdefault(targets.artifact_name(entry, tag), []).append(entry)
    groups: dict[tuple[str, str], list[tuple[Path, list[dict]]]] = {}
    for directory in sorted(path for path in args.artifacts.iterdir() if path.is_dir()):
        name = directory.name
        published = name.startswith(f"packages-{tag}-")
        if not published:
            continue
        arch = next((a for a in architectures if name.endswith(f"-{a}")), None)
        base = name[: -len(arch) - 1] if arch else name
        owners_here = by_artifact.get(name, [])
        if not owners_here:
            errors.append(f"{name}: no target installs this artifact (G3)")
        for path in sorted(directory.iterdir()):
            if not path.is_file() or format_of(path) is None:
                continue
            owners = [entry for entry in owners_here if targets.selects(entry, path.name)]
            if not owners:
                errors.append(f"{name}/{path.name}: no target installs and tests this file (G3)")
                continue
            if PER_RELEASE_DEB.search(path.name):
                errors.append(f"{name}/{path.name}: a per-release deb in a published artifact (G4)")
                continue
            for entry in owners:
                if targets.own_lane(entry) and not entry["published"]:
                    errors.append(f"{name}/{path.name}: {entry['name']} is not published, but its file is (G4)")
            groups.setdefault((base, path.name), []).append((path, owners))

    publish_set, excluded_files, published_keys = [], [], set()
    out = args.out
    if out.exists():
        shutil.rmtree(out)
    (out / "publishable").mkdir(parents=True)
    for (_, filename), copies in sorted(groups.items()):
        owners = [entry for _, owner_list in copies for entry in owner_list]
        failing = [entry for entry in owners if not status[(entry["slug"], entry["arch"])]["passed"]]
        if failing:
            names = ", ".join(sorted({f"{entry['name']} {entry['arch']}" for entry in failing}))
            excluded_files.append(f"{filename}: {names} did not pass every stage")
            continue
        for path, owner_list in copies:
            target_dir = out / "publishable" / path.parent.name
            target_dir.mkdir(exist_ok=True)
            shutil.copy2(path, target_dir / filename)
            publish_set.append(f"{path.parent.name}/{filename}")
            published_keys.update((entry["slug"], entry["arch"]) for entry in owner_list)

    verdicts = []
    for entry in matrix:
        key = (entry["slug"], entry["arch"])
        eol, eol_date = eol_of(entry["name"], rows_by_name, graphical_by_name, args.today)
        verdicts.append(
            {
                "name": entry["name"],
                "slug": entry["slug"],
                "arch": entry["arch"],
                "state": "experimental" if entry["experimental"] else "built",
                "eol": eol,
                "eol_date": eol_date,
                **status[key],
                "published": key in published_keys,
                "complete": True,
                "evidence": f"verdict-{tag}-{entry['slug']}-{entry['arch']}",
            }
        )
    for row in rows:
        if row["state"] != "planned":
            continue
        eol, eol_date = eol_of(row["name"], rows_by_name, graphical_by_name, args.today)
        blocker = row.get("note") or "no job builds this release"
        verdicts.append(
            {
                "name": row["name"],
                "slug": row.get("slug", ""),
                "arch": "-",
                "state": "planned",
                "eol": eol,
                "eol_date": eol_date,
                "stages": {"build": {"status": NOT_BUILT, "check": "build", "detail": blocker}},
                "passed": False,
                "reason": f"not built: {blocker}",
                "published": False,
                "complete": True,
                "evidence": "",
            }
        )

    published = [v for v in verdicts if v["passed"] and v["published"]]
    # The targets with a job that did not pass, and the first stage that stopped each, for the release notes.
    left_out = [
        f"{v['name']} {v['arch']}, {v['reason'].split(':', 1)[0]}"
        for v in verdicts
        if v["state"] != "planned" and not v["passed"]
    ]
    if not publish_set:
        errors.append("the publish set is empty: no package passed every stage on every target that installs it")
    result = {
        "release_tag": tag,
        "stages": list(STAGES),
        "suite_layers": list(report.LAYERS),
        "errors": errors,
        "exclusions": exclusions,
        "excluded_files": excluded_files,
        "publish_set": sorted(publish_set),
        "published_targets": names_with_arches(published),
        "experimental_published_targets": names_with_arches([v for v in published if v["state"] == "experimental"]),
        "left_out": left_out,
        "targets": verdicts,
    }
    (out / "coverage.json").write_text(json.dumps(result, indent=1) + "\n")
    markdown = render(result, rows)
    (out / "coverage.md").write_text(markdown)
    print(markdown)
    return 1 if errors else 0


def names_with_arches(verdicts: list[dict]) -> list[str]:
    """ "Fedora 44 (amd64, arm64)" for each release, in the order of the matrix."""
    arches: dict[str, list[str]] = {}
    for item in verdicts:
        arches.setdefault(item["name"], []).append(item["arch"])
    return [f"{name} ({', '.join(found)})" for name, found in arches.items()]


def cell(stage: dict) -> str:
    text = {PASS: "pass", SHARED: "shared", NOT_BUILT: "not built"}.get(stage["status"])
    if text is None:
        text = stage["detail"] if stage["status"] == NOT_RUN else f"FAIL {stage['check']}: {stage['detail']}"
    text = text.replace("|", "/").replace("\n", " ")
    return text if len(text) <= 90 else text[:87] + "..."


def render(result: dict, rows: list[dict]) -> str:
    lines = [
        f"## Release gate: {result['release_tag']}",
        "",
        "A target is published only when every stage passed on its architecture: prefix, packages, package suite "
        f"({', '.join(result['suite_layers'])}) and the graphical VM test.",
        "",
        "| Target | Arch | State | Prefix | Packages | Suite | Graphical | Result |",
        "| ------ | ---- | ----- | ------ | -------- | ----- | --------- | ------ |",
    ]
    order = {row["name"]: number for number, row in enumerate(rows)}
    for item in sorted(result["targets"], key=lambda item: (order.get(item["name"], len(order)), item["arch"])):
        if item["state"] == "planned":
            outcome = f"not built: {item['stages']['build']['detail']}"
            lines.append(f"| {item['name']} | - | planned | not built | not built | not run | not run | {outcome} |")
            continue
        stages = item["stages"]
        if item["passed"]:
            outcome = "passed, published" if item["published"] else "passed, not published (test packages)"
        else:
            outcome = ("excluded" if item["state"] == "experimental" else "FAILED") + ", not published"
        lines.append(
            f"| {item['name']} | {item['arch']} | {item['state']} | "
            + " | ".join(cell(stages[name]) for name in STAGES)
            + f" | {outcome} |"
        )
    if result["exclusions"]:
        lines += ["", "### Exclusions", ""] + [f"- {line}" for line in result["exclusions"]]
    if result["excluded_files"]:
        lines += ["", "### Packages left out of the publish set", ""] + [
            f"- {line}" for line in result["excluded_files"]
        ]
    if result["errors"]:
        lines += ["", "### Errors that fail the release", ""] + [f"- {line}" for line in result["errors"]]
    lines += ["", f"Publish set: {len(result['publish_set'])} files."]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--targets", type=Path, required=True)
    parser.add_argument("--stages", type=Path, help="the stage.json files of the build stages")
    parser.add_argument("--reports", type=Path, required=True)
    parser.add_argument("--graphical", type=Path, help="the result.json files of the graphical VM test")
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--today", type=datetime.date.fromisoformat, default=datetime.date.today())
    return run(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())

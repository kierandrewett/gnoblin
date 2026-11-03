#!/usr/bin/env python3
"""Write the stage.json records of the build stages that the release gate (tests/packages/gate.py) reads.

usage: tests/pipeline/stages.py record --stage STAGE --slug SLUG --arch ARCH --job-status STATUS --out DIR
                                       [--detail TEXT] [--run-url URL]
       tests/pipeline/stages.py shared --matrix JSON --arch ARCH --job-status STATUS --out DIR [--run-url URL]
       tests/pipeline/stages.py needs --target JSON --needs JSON --out DIR [--run-url URL]

record  one stage of one target, from the status of the job that ran it (job.status: success, failure or
        cancelled). The shared prefix job of .github/workflows/release.yml uses it.
shared  the packages stage of every target that installs the shared packages on ARCH (the targets of --matrix without
        a lane), from the status of the shared packages job.
needs   the prefix and packages stages of one target leg (.github/workflows/target.yml), from the needs context of its
        verdict job (toJSON(needs)). A job sets the output "status" from job.status in its last step, so a failed job
        with continue-on-error (an experimental target) still counts as a failure. A job without that output was
        skipped ("not run"), or ended before its last step (a failure: cancelled, timed out or the runner was lost).
        The prefix stage is written only when the leg builds its own prefix, and the packages stage only when it
        writes its own packages; the shared jobs write the others.

Each record goes to OUT/<stage>/stage.json (OUT/<slug>-<arch>/packages/stage.json for shared):
{"stage", "slug", "arch", "status": "pass" | "fail" | "not run", "check", "detail", "log"}.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def from_status(status: str, stage: str) -> tuple[str, str]:
    if status == "success":
        return "pass", ""
    if status == "cancelled":
        return "fail", f"the {stage} job was cancelled or timed out"
    return "fail", f"the {stage} job failed; its log names the step"


def write(out: Path, stage: str, slug: str, arch: str, status: str, detail: str, log: str) -> None:
    folder = out / stage
    folder.mkdir(parents=True, exist_ok=True)
    record = {"stage": stage, "slug": slug, "arch": arch, "status": status, "check": stage, "detail": detail,
              "log": log}  # fmt: skip
    (folder / "stage.json").write_text(json.dumps(record, indent=1) + "\n")
    print(f"{slug} {arch} {stage}: {status}{': ' + detail if detail else ''}")


def from_needs(job: dict | None, stage: str) -> tuple[str, str]:
    job = job or {}
    status = (job.get("outputs") or {}).get("status")
    if status:
        return from_status(status, stage)
    if job.get("result") in (None, "skipped"):
        return "not run", f"the {stage} job did not run"
    return "fail", f"the {stage} job ended before it recorded a status ({job.get('result')})"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    record = commands.add_parser("record")
    record.add_argument("--stage", required=True, choices=("prefix", "packages"))
    record.add_argument("--slug", required=True)
    record.add_argument("--arch", required=True)
    record.add_argument("--job-status", required=True)
    record.add_argument("--detail", default="")
    shared = commands.add_parser("shared")
    shared.add_argument("--matrix", required=True)
    shared.add_argument("--arch", required=True)
    shared.add_argument("--job-status", required=True)
    needs = commands.add_parser("needs")
    needs.add_argument("--target", required=True)
    needs.add_argument("--needs", required=True)
    for command in (record, shared, needs):
        command.add_argument("--out", type=Path, required=True)
        command.add_argument("--run-url", default="")
    arguments = parser.parse_args()

    log = arguments.run_url
    if arguments.command == "record":
        status, detail = from_status(arguments.job_status, arguments.stage)
        write(arguments.out, arguments.stage, arguments.slug, arguments.arch, status, arguments.detail or detail, log)
    elif arguments.command == "shared":
        status, detail = from_status(arguments.job_status, "shared packages")
        for item in json.loads(arguments.matrix):
            if item["arch"] == arguments.arch and not item["lane"]:
                out = arguments.out / f"{item['slug']}-{item['arch']}"
                write(out, "packages", item["slug"], item["arch"], status, detail, log)
    else:
        target, jobs = json.loads(arguments.target), json.loads(arguments.needs)
        if target["prefix"] == "own" and target["prefix_artifact"] == "prefix":
            write(
                arguments.out, "prefix", target["slug"], target["arch"], *from_needs(jobs.get("prefix"), "prefix"), log
            )
        if target["lane"]:
            status, detail = from_needs(jobs.get("packages"), "packages")
            write(arguments.out, "packages", target["slug"], target["arch"], status, detail, log)
    return 0


if __name__ == "__main__":
    sys.exit(main())

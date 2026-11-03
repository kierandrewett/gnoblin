"""The results of the package suite: one JSON file for each layer, then report.json, junit.xml and summary.md.

A case is passed, failed or skipped. A skipped case gives its reason. A layer passes when it has at least one case and
no case failed. A layer that wrote no result file did not run, and counts as failed.
"""

from __future__ import annotations

import datetime
import json
import time
import xml.etree.ElementTree as ET
from pathlib import Path

# The layers of the suite, in the order they run. The release gate requires every one of them for every target.
LAYERS = ("static", "install", "runtime", "repository")
PASS, FAIL, SKIP = "pass", "fail", "skip"


class Layer:
    """Collects the cases of one layer while it runs."""

    def __init__(self, name: str):
        if name not in LAYERS:
            raise ValueError(f"unknown layer {name}")
        self.name = name
        self.cases: list[dict] = []
        self.started = time.time()

    def add(self, case_id: str, title: str, status: str, message: str = "", seconds: float = 0.0, subject: str = ""):
        if status not in (PASS, FAIL, SKIP):
            raise ValueError(status)
        self.cases.append(
            {
                "id": case_id,
                "title": title,
                "subject": subject,
                "status": status,
                "message": message,
                "seconds": round(seconds, 3),
            }
        )

    def check(self, case_id: str, title: str, subject: str, errors: list[str], skip: str = ""):
        """Add a case from a list of errors: none is a pass. skip, when given and there are no errors, skips it."""
        if errors:
            self.add(
                case_id,
                title,
                FAIL,
                "\n".join(errors[:50]) + (f"\n... {len(errors) - 50} more" if len(errors) > 50 else ""),
                subject=subject,
            )
        elif skip:
            self.add(case_id, title, SKIP, skip, subject=subject)
        else:
            self.add(case_id, title, PASS, subject=subject)

    def result(self) -> dict:
        return layer_dict(self.name, self.cases, self.started, time.time())

    def write(self, report_dir: Path) -> dict:
        result = self.result()
        write_layer(report_dir, result)
        return result


def layer_status(cases: list[dict]) -> str:
    if not cases or any(case["status"] == FAIL for case in cases):
        return FAIL
    return PASS


def layer_dict(name: str, cases: list[dict], started: float, finished: float) -> dict:
    return {
        "layer": name,
        "status": layer_status(cases),
        "started": datetime.datetime.fromtimestamp(started, datetime.UTC).isoformat(timespec="seconds"),
        "seconds": round(finished - started, 1),
        "cases": cases,
    }


def write_layer(report_dir: Path, result: dict):
    (report_dir / "layers").mkdir(parents=True, exist_ok=True)
    (report_dir / "layers" / f"{result['layer']}.json").write_text(json.dumps(result, indent=1) + "\n")


def from_tsv(name: str, tsv: Path, started: float, finished: float) -> dict:
    """The results of a shell layer: one line for each case, id TAB title TAB status TAB seconds TAB message, with
    newlines in the message written as \\n."""
    cases = []
    if tsv.is_file():
        for line in tsv.read_text(errors="replace").replace("\r", " ").split("\n"):
            if not line.strip():
                continue
            case_id, title, status, seconds, message = (line.split("\t", 4) + ["", "", "", "0", ""])[:5]
            cases.append(
                {
                    "id": case_id,
                    "title": title,
                    "subject": "",
                    "status": status if status in (PASS, FAIL, SKIP) else FAIL,
                    "message": message.replace("\\n", "\n"),
                    "seconds": float(seconds or 0),
                }
            )
    return layer_dict(name, cases, started, finished)


def finish(report_dir: Path, target: dict, release_tag: str) -> dict:
    """Write report.json, junit.xml and summary.md from the layer files. A missing layer file is a layer that did not run."""
    layers = {}
    for name in LAYERS:
        path = report_dir / "layers" / f"{name}.json"
        if path.is_file():
            layers[name] = json.loads(path.read_text())
        else:
            layers[name] = {"layer": name, "status": "missing", "seconds": 0, "cases": []}
    passed = all(layer["status"] == PASS for layer in layers.values())
    report = {
        "schema": 1,
        "release_tag": release_tag,
        "target": target,
        "suite_layers": list(LAYERS),
        "layers": layers,
        "result": PASS if passed else FAIL,
    }
    (report_dir / "report.json").write_text(json.dumps(report, indent=1) + "\n")
    write_junit(report_dir / "junit.xml", report)
    (report_dir / "summary.md").write_text(summary(report))
    return report


def write_junit(path: Path, report: dict):
    target = report["target"]
    suites = ET.Element("testsuites", name=f"packages {target['slug']} {target['arch']}")
    for layer in report["layers"].values():
        cases = layer["cases"]
        suite = ET.SubElement(
            suites,
            "testsuite",
            name=f"{target['slug']}-{target['arch']}.{layer['layer']}",
            tests=str(max(len(cases), 1)),
            failures=str(sum(case["status"] == FAIL for case in cases) + (0 if cases else 1)),
            skipped=str(sum(case["status"] == SKIP for case in cases)),
            time=str(layer.get("seconds", 0)),
        )
        if not cases:
            case = ET.SubElement(suite, "testcase", classname=suite.get("name"), name="layer ran")
            ET.SubElement(case, "failure", message="the layer did not run").text = "no result file"
        for item in cases:
            name = f"{item['id']} {item['title']}" + (f": {item['subject']}" if item.get("subject") else "")
            case = ET.SubElement(suite, "testcase", classname=suite.get("name"), name=name, time=str(item["seconds"]))
            if item["status"] == FAIL:
                ET.SubElement(case, "failure", message=item["message"].split("\n", 1)[0][:200]).text = item["message"]
            elif item["status"] == SKIP:
                ET.SubElement(case, "skipped", message=item["message"][:200])
    ET.indent(suites)
    ET.ElementTree(suites).write(path, encoding="utf-8", xml_declaration=True)


def summary(report: dict) -> str:
    target = report["target"]
    state = "experimental" if target.get("experimental") else "built"
    lines = [
        f"### Package tests: {target['name']} ({target['arch']}, {state}): {report['result']}",
        "",
        "| Layer | Result | Passed | Failed | Skipped | Seconds |",
        "| ----- | ------ | ------ | ------ | ------- | ------- |",
    ]
    for layer in report["layers"].values():
        cases = layer["cases"]
        count = {status: sum(case["status"] == status for case in cases) for status in (PASS, FAIL, SKIP)}
        lines.append(
            f"| {layer['layer']} | {layer['status']} | {count[PASS]} | {count[FAIL]} | {count[SKIP]} | {layer.get('seconds', 0)} |"
        )
    problems = [
        (layer["layer"], case)
        for layer in report["layers"].values()
        for case in layer["cases"]
        if case["status"] in (FAIL, SKIP)
    ]
    if problems:
        lines += ["", "| Layer | Case | Result | Message |", "| ----- | ---- | ------ | ------- |"]
        for layer, case in problems:
            message = case["message"].split("\n", 1)[0].replace("|", "\\|")[:160]
            subject = f" ({case['subject']})" if case.get("subject") else ""
            lines.append(f"| {layer} | {case['id']} {case['title']}{subject} | {case['status']} | {message} |")
    return "\n".join(lines) + "\n"

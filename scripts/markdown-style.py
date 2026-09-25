#!/usr/bin/env python3
"""Audit and refine Gnoblin Markdown using the project's writing preferences."""

from __future__ import annotations

import argparse
import difflib
import json
import os
import re
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent.parent
PROFILE = ROOT / "MARKDOWN_STYLE.md"
SKIP_DIRS = {".git", "node_modules", ".vitepress", "dist", "build"}


def markdown_files(paths: list[str]) -> list[Path]:
    found: set[Path] = set()
    targets = [Path(p) if Path(p).is_absolute() else ROOT / p for p in paths]
    for target in targets:
        if target.is_file() and target.suffix.lower() in {".md", ".markdown"}:
            found.add(target)
        elif target.is_dir():
            for file in target.rglob("*"):
                if file.suffix.lower() not in {".md", ".markdown"}:
                    continue
                if any(part in SKIP_DIRS for part in file.parts):
                    continue
                found.add(file)
    return sorted(found)


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def paint(text: str, code: str, enabled: bool) -> str:
    if not enabled:
        return text
    return f"\033[{code}m{text}\033[0m"


def word_count(text: str) -> int:
    """Count prose words without inflating counts for Markdown identifiers."""
    text = re.sub(r"`[^`]*`", " ", text)
    text = re.sub(r"https?://\S+", " ", text)
    return len(re.findall(r"\b[\w'-]+\b", text))


def structural_issues(path: Path) -> list[tuple[int, str]]:
    """Catch cramped Markdown structures before the editorial review."""
    lines = path.read_text(encoding="utf-8").splitlines()
    issues: list[tuple[int, str]] = []
    blocks: list[tuple[str, int, str]] = []
    paragraph: list[str] = []
    paragraph_line = 1
    in_fence = False

    def flush_paragraph() -> None:
        nonlocal paragraph
        if paragraph:
            blocks.append(("prose", paragraph_line, " ".join(line.strip() for line in paragraph)))
            paragraph = []

    for number, line in enumerate(lines, 1):
        if re.match(r"^\s*(```|~~~)", line):
            flush_paragraph()
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if not line.strip():
            flush_paragraph()
            continue
        if line.lstrip().startswith("|"):
            flush_paragraph()
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            if all(re.fullmatch(r"\s*:?-{3,}:?\s*", cell) for cell in cells):
                continue
            blocks.append(("table", number, line))
            for cell in cells:
                words = word_count(cell)
                if words > 24:
                    issues.append(
                        (number, f"table cell has {words} prose words (limit 24); shorten it or use grouped bullets")
                    )
            continue
        if re.match(r"^\s*(#{1,6}\s|>|[-*+]\s|\d+[.)]\s|<!--)", line):
            flush_paragraph()
            blocks.append(("heading" if line.lstrip().startswith("#") else "other", number, line))
            continue
        if not paragraph:
            paragraph_line = number
        paragraph.append(line)
    flush_paragraph()

    for kind, number, text in blocks:
        if kind == "prose":
            words = word_count(text)
            if words > 60:
                issues.append(
                    (
                        number,
                        f"paragraph has {words} prose words (usual limit 60); split it or turn independent facts into a list",
                    )
                )

    for index, (kind, number, _text) in enumerate(blocks):
        if kind != "table" or (index and blocks[index - 1][0] == "table"):
            continue
        table_rows = []
        for row_kind, row_line, row_text in blocks[index:]:
            if row_kind != "table":
                break
            cells = [cell.strip() for cell in row_text.strip().strip("|").split("|")]
            if len(cells) < 2 or all(re.fullmatch(r"\s*:?-{3,}:?\s*", cell) for cell in cells):
                continue
            fields = re.findall(r"`([^`]+)`", cells[0])
            if not fields:
                fields = [cell.strip() for cell in cells[0].split(",")]
            descriptions = " ".join(cells[1:])
            row_tokens = set(re.findall(r"[a-z0-9]+", (cells[0] + " " + descriptions).lower()))
            for field in fields:
                if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_.-]*", field):
                    table_rows.append((field, row_tokens, row_line))
        if not table_rows:
            continue
        for next_kind, next_line, next_text in blocks[index + 1 :]:
            if next_kind == "heading":
                break
            if next_kind != "prose":
                continue
            prose_tokens = set(re.findall(r"[a-z0-9]+", next_text.lower()))
            repeated = 0
            for field, row_tokens, _row_line in table_rows:
                if not re.search(rf"(?<![\w.])`?{re.escape(field)}`?(?![\w.])", next_text):
                    continue
                shared = row_tokens & prose_tokens
                if len(shared) >= 3 and len(shared) / max(1, len(row_tokens)) >= 0.5:
                    repeated += 1
            if repeated >= 3:
                issues.append(
                    (
                        next_line,
                        "prose repeats three or more table field definitions; keep extra prose for interactions, limits, or selection guidance",
                    )
                )
                break

    return sorted(set(issues))


def lint_files(files: list[Path]) -> int:
    total = 0
    for path in files:
        found = structural_issues(path)
        print(display_path(path))
        if not found:
            print("  No structural flags")
        for line, issue in found:
            total += 1
            print(f"  {display_path(path)}:{line}: {issue}")
    print(f"Structural review: {len(files)} files, {total} flags.")
    return total


def style_sections(text: str) -> list[tuple[str, str, str]]:
    """Use each level-two section in the style guide as one Jev check."""
    sections: list[tuple[str, str, str]] = []
    title = ""
    body: list[str] = []
    for line in text.splitlines():
        heading = re.match(r"^##\s+(.+?)\s*$", line)
        if heading:
            if title:
                sections.append((title, "\n".join(body).strip()))
            title, body = heading.group(1), []
        elif title:
            body.append(line)
    if title:
        sections.append((title, "\n".join(body).strip()))
    return [
        (
            re.sub(r"[^a-z0-9]+", "-", heading.lower()).strip("-"),
            heading,
            f"{heading}\n{body}".strip(),
        )
        for heading, body in sections
    ]


def cmd_review(args: argparse.Namespace) -> int:
    load_local_env()
    color = args.color == "always" or (
        args.color == "auto"
        and sys.stdout.isatty()
        and "NO_COLOR" not in os.environ
        and os.environ.get("TERM") != "dumb"
    )
    provider = args.provider or os.environ.get("GNOBLIN_DOCS_JEV_PROVIDER")
    if provider not in {"typesafe", "openrouter"}:
        print(
            "Set GNOBLIN_DOCS_JEV_PROVIDER in the repo-root .env file or pass --provider.",
            file=sys.stderr,
        )
        return 2

    files = markdown_files(args.paths)
    if not files:
        print("No Markdown files found in the selected paths.", file=sys.stderr)
        return 2

    structural_flags = lint_files(files)

    env_name = "GNOBLIN_TYPESAFE_API_KEY" if provider == "typesafe" else "GNOBLIN_OPENROUTER_API_KEY"
    api_key = os.environ.get(env_name)
    if not api_key:
        print(
            f"--provider {provider} requires {env_name} in .env or the process environment.",
            file=sys.stderr,
        )
        return 2

    if provider == "typesafe":
        endpoint = "https://api.typesafe.ai/v1/systemone"
        selected_model = args.model or os.environ.get("GNOBLIN_DOCS_JEV_MODEL") or "jev-latest"
    else:
        endpoint = "https://openrouter.ai/api/alpha/decisions"
        selected_model = args.model or os.environ.get("GNOBLIN_DOCS_JEV_MODEL") or "~typesafe/jev-latest"
    style = PROFILE.read_text(encoding="utf-8")
    rules = style_sections(style)
    if not rules:
        print("No level-two rule sections found in MARKDOWN_STYLE.md.", file=sys.stderr)
        return 2
    print(paint(f"Jev Markdown review  ·  {provider}  ·  {selected_model}", "1;36", color))
    print(
        f"{len(files)} files  ·  {len(rules)} rules  ·  "
        f"{args.concurrency} concurrent requests  ·  flag at {args.threshold:.0%}\n"
    )
    total_flagged = 0

    def review_file(path: Path) -> tuple[Path, dict | None, str | None]:
        source = path.read_text(encoding="utf-8")
        questions = {
            rule_id: {
                "type": "noul",
                "instructions": (
                    f"Does this Markdown document clearly violate the following style-guide section?\n{rule_text}"
                ),
                "criteria": {
                    "true": (
                        "At least one specific passage in this file materially violates "
                        "this applicable rule and should be corrected before merge."
                    ),
                    "false": (
                        "There is no clear, material, fixable violation. Do not flag "
                        "rule clauses that do not apply to this document's purpose."
                    ),
                },
            }
            for rule_id, _heading, rule_text in rules
        }
        body = {
            "model": selected_model,
            "state": {
                "path": display_path(path),
                "style_rules": style,
                "markdown": source,
            },
            "questions": questions,
        }
        request = Request(
            endpoint,
            data=json.dumps(body).encode("utf-8"),
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urlopen(request, timeout=90) as response:
                payload = json.loads(response.read().decode("utf-8"))
            return path, payload["answers"], None
        except (HTTPError, URLError, TimeoutError, KeyError, TypeError, ValueError) as exc:
            detail = getattr(exc, "reason", str(exc))
            return path, None, str(detail)

    with ThreadPoolExecutor(max_workers=args.concurrency) as executor:
        results = list(executor.map(review_file, files))

    rule_names = {rule_id: heading for rule_id, heading, _ in rules}
    rule_texts = {rule_id: text for rule_id, _, text in rules}
    label_width = max(len(heading) for _, heading, _ in rules)
    reviewed = 0
    failures = 0
    for path, answers, error in results:
        print(paint(display_path(path), "1", color))
        if error:
            failures += 1
            print(paint(f"  Jev review failed: {error}\n", "31", color), file=sys.stderr)
            continue
        reviewed += 1
        flagged = 0
        flagged_rules = []
        print(paint(f"  {'Rule':<{label_width}}  {'Risk':>5}  Review", "2", color))
        for rule_id, answer in answers.items():
            probability = float(answer["noul"])
            is_flagged = probability >= args.threshold
            flagged += int(is_flagged)
            label = rule_names.get(rule_id, rule_id)
            review = "yes" if is_flagged else "—"
            risk = paint(f"{probability:>4.0%}", "31" if is_flagged else "2", color)
            review = paint("FIX", "1;31", color) if is_flagged else "—"
            print(f"  {label:<{label_width}}  {risk}  {review}")
            if is_flagged:
                flagged_rules.append((rule_id, probability))
        total_flagged += flagged
        print(f"  {flagged} rules to review")
        for rule_id, probability in flagged_rules:
            heading = f"  FIX REQUIRED · {rule_names.get(rule_id, rule_id)} ({probability:.0%})"
            print(f"\n{paint(heading, '1;31', color)}")
            print(paint("  Resolve this flag before completing the documentation change:", "33", color))
            print("\n".join(f"    {line}" for line in rule_texts[rule_id].splitlines()))
        print()

    summary = f"Complete: {reviewed}/{len(files)} files reviewed, {total_flagged} rule flags."
    print(paint(summary, "1;31" if total_flagged or failures else "1;32", color))
    return int(failures > 0 or (args.fail_on_flags and (total_flagged > 0 or structural_flags > 0)))


def cmd_lint(args: argparse.Namespace) -> int:
    files = markdown_files(args.paths)
    if not files:
        print("No Markdown files found in the selected paths.", file=sys.stderr)
        return 2
    total = lint_files(files)
    return int(args.fail_on_flags and total > 0)


def load_local_env() -> None:
    """Read simple KEY=value entries without overriding the process environment."""
    path = ROOT / ".env"
    if not path.is_file():
        return
    for line in path.read_text(encoding="utf-8").splitlines():
        entry = line.strip()
        if not entry or entry.startswith("#"):
            continue
        if entry.startswith("export "):
            entry = entry[7:].lstrip()
        key, separator, value = entry.partition("=")
        if not separator or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key.strip()):
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        os.environ.setdefault(key.strip(), value)


def probability_arg(value: str) -> float:
    try:
        probability = float(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("threshold must be a number from 0 to 1") from exc
    if not 0 <= probability <= 1:
        raise argparse.ArgumentTypeError("threshold must be between 0 and 1")
    return probability


def cmd_split(args: argparse.Namespace) -> int:
    path = Path(args.file)
    if not path.is_absolute():
        path = ROOT / path
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    idx = args.line - 1
    if idx < 0 or idx >= len(lines):
        print(f"{display_path(path)}:{args.line}: line is outside the file", file=sys.stderr)
        return 2
    line = lines[idx]
    needle = args.after
    pos = line.find(needle)
    if pos < 0:
        print(f"{display_path(path)}:{args.line}: --after text not found on that line", file=sys.stderr)
        return 2
    end = pos + len(needle)
    remainder = line[end:].lstrip()
    newline = "\n" if line.endswith("\n") else ""
    replacement = line[:end].rstrip() + "\n\n"
    if remainder:
        replacement += remainder + newline
    lines[idx] = replacement
    updated = "".join(lines)
    diff = difflib.unified_diff(
        path.read_text(encoding="utf-8").splitlines(keepends=True),
        updated.splitlines(keepends=True),
        fromfile=f"{display_path(path)} (before)",
        tofile=f"{display_path(path)} (after)",
    )
    print("".join(diff), end="")
    if args.write:
        path.write_text(updated, encoding="utf-8")
        print(f"\nApplied paragraph break to {display_path(path)}:{args.line}")
    else:
        print("\nPreview only. Add --write to apply this exact paragraph break.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Review Markdown using Gnoblin's documentation style.")
    sub = parser.add_subparsers(dest="command", required=True)
    review = sub.add_parser("review", help="classify style-rule violations with Jev")
    review.add_argument("paths", nargs="+", help="Markdown files or directories to send for review")
    review.add_argument("--provider", choices=("typesafe", "openrouter"))
    review.add_argument("--model", help="provider-specific Jev model name")
    review.add_argument(
        "--color",
        choices=("auto", "always", "never"),
        default="auto",
        help="color output (default: auto; respects NO_COLOR)",
    )
    review.add_argument(
        "--fail-on-flags",
        action="store_true",
        help="exit nonzero when any rule is flagged",
    )
    review.add_argument(
        "--concurrency",
        type=int,
        default=5,
        help="maximum simultaneous requests (default: 5)",
    )
    review.add_argument(
        "--threshold",
        type=probability_arg,
        default=0.7,
        help="probability at which to flag a rule for human review (default: 0.7)",
    )
    review.set_defaults(func=cmd_review)
    lint = sub.add_parser("lint", help="check Markdown structure for cramped prose and tables")
    lint.add_argument("paths", nargs="+", help="Markdown files or directories to check")
    lint.add_argument("--fail-on-flags", action="store_true", help="exit nonzero when structural issues are found")
    lint.set_defaults(func=cmd_lint)
    split = sub.add_parser("split", help="preview or apply a paragraph break after exact text")
    split.add_argument("file")
    split.add_argument("line", type=int, help="one-based source line number")
    split.add_argument("--after", required=True, help="exact sentence ending after which to insert a blank line")
    split.add_argument("--write", action="store_true", help="apply the shown edit; default is preview only")
    split.set_defaults(func=cmd_split)
    args = parser.parse_args()
    if getattr(args, "concurrency", 1) < 1:
        parser.error("--concurrency must be at least 1")
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())

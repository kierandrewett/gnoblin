#!/usr/bin/env python3
"""Run wlcs against the Gnoblin integration library and check the result against tests/wlcs/expected.toml.

`scripts/wlcs.sh` builds wlcs and the library and calls this. The runner is separate because a wlcs test can
crash the process it runs in (a compositor that aborts takes the whole gtest binary with it), and because the
suites are independent, so they run in parallel in processes of their own.

    wlcs-run.py --wlcs BIN --plugin LIB [--subset core|all] [--jobs N] [--update] [--results FILE] [PATTERN...]

Without patterns the run is the subset's patterns from expected.toml. With patterns it is those `Suite.test`
globs, and the expectation check is skipped unless `--check` says otherwise: a developer asking for one test wants
its output, not a verdict on the file.

Exit status: 0 when every test of the run matches the expectation file, 1 on a mismatch, 2 when the run itself
failed (no wlcs, no tests matched, bad file).
"""

import argparse
import concurrent.futures
import fnmatch
import json
import os
import re
import subprocess
import sys
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXPECTED = ROOT / "tests" / "wlcs" / "expected.toml"

RUN = re.compile(r"^\[ RUN      \] (\S+)")
# gmock chatter that says nothing about why a test failed.
NOISE = re.compile(
    r"^(GMOCK WARNING:|Uninteresting mock function call|/\S+:\d+:$|Function call:|NOTE: You can safely|Stack trace:|$)"
)
RESULT = re.compile(r"^\[ +(OK|FAILED|SKIP|SKIPPED) +\] (\S+?),?(?: where .*?)?(?: ?\((\d+) ?ms\))?$")


def list_tests(wlcs, plugin):
    """Every `Suite.test` wlcs knows for the plugin, in listing order."""
    out = subprocess.run([wlcs, plugin, "--gtest_list_tests"], capture_output=True, text=True, check=True).stdout
    tests, suite = [], None
    for line in out.splitlines():
        if not line.strip():
            continue
        if not line.startswith(" "):
            suite = line.split("#")[0].strip()
        else:
            name = line.strip().split("#")[0].strip()
            if suite and not name.startswith("DISABLED_"):
                tests.append(suite + name)
    return tests


def parse(output):
    """Turns gtest's console output into `{test: (status, ms, detail)}` and the test that was running last."""
    results, running, detail = {}, None, []
    for line in output.splitlines():
        started = RUN.match(line)
        if started:
            running, detail = started.group(1), []
            continue
        done = RESULT.match(line)
        if done and running == done.group(2):
            status = {"OK": "pass", "FAILED": "fail", "SKIP": "skip", "SKIPPED": "skip"}[done.group(1)]
            results[running] = (status, int(done.group(3) or 0), " | ".join(detail[:4]))
            running = None
        elif running and not NOISE.match(line.strip()):
            detail.append(line.strip())
    return results, running


def run_group(wlcs, plugin, tests, timeout):
    """Runs `tests` in as many wlcs processes as it takes: a crash is recorded and the rest run again."""
    results, remaining = {}, list(tests)
    while remaining:
        command = [wlcs, plugin, "--gtest_filter=" + ":".join(remaining)]
        try:
            done = subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=os.environ)
            output, crashed = done.stdout + done.stderr, done.returncode not in (0, 1)
        except subprocess.TimeoutExpired as expired:
            output = (expired.stdout or b"").decode(errors="replace") + (expired.stderr or b"").decode(errors="replace")
            crashed = True
        got, running = parse(output)
        results.update(got)
        if running and running not in got:
            results[running] = (
                "crash",
                0,
                "the process ended during the test: " + " | ".join(output.splitlines()[-3:]),
            )
        remaining = [t for t in remaining if t not in results]
        if not crashed and not running:
            for test in remaining:
                results[test] = ("crash", 0, "wlcs finished without running the test")
            break
    return results


def suites(tests):
    grouped = {}
    for test in tests:
        grouped.setdefault(test.split(".")[0], []).append(test)
    return grouped


def matches(test, patterns):
    return any(fnmatch.fnmatchcase(test, pattern) for pattern in patterns)


def load_expected():
    with open(EXPECTED, "rb") as handle:
        return tomllib.load(handle)


def check(results, expected, subset_tests):
    """Compares a run with the expectation file; returns the list of mismatches."""
    passing = set(expected.get("passing", []))
    known = expected.get("known_failure", [])
    problems = []
    for test in sorted(results):
        status = results[test][0]
        entry = next((k for k in known if fnmatch.fnmatchcase(test, k["pattern"])), None)
        if test in passing:
            if status != "pass":
                problems.append(f"regression: {test} is expected to pass and {status}ed: {results[test][2]}")
        elif entry is not None:
            if status == "pass":
                problems.append(f"fixed: {test} passes but is a known failure ({entry['issue']}); move it to `passing`")
        elif status == "skip":
            continue
        elif status == "pass":
            problems.append(f"new pass: {test} passes and is not in `passing`; run with --update")
        else:
            problems.append(f"unclassified: {test} {status}ed with no known_failure entry: {results[test][2]}")
    ran = set(results)
    for test in sorted(passing - ran):
        if test in subset_tests:
            problems.append(f"missing: {test} is in `passing` and wlcs did not report it")
    return problems


def update_passing(results, subset_tests):
    """Rewrites the `passing` array of the expectation file to the passes of this run."""
    text = EXPECTED.read_text()
    passed = sorted(test for test, (status, _, _) in results.items() if status == "pass")
    keep = [t for t in load_expected().get("passing", []) if t not in subset_tests]
    body = "passing = [\n" + "".join(f'    "{t}",\n' for t in sorted(set(keep) | set(passed))) + "]\n"
    new, count = re.subn(r"^passing = \[\n(?:    \".*\",\n)*\]\n", body, text, flags=re.M)
    if count != 1:
        sys.exit("expected.toml has no `passing` array in the generated shape")
    EXPECTED.write_text(new)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--wlcs", required=True)
    parser.add_argument("--plugin", required=True)
    parser.add_argument("--subset", choices=["core", "all"], default="core")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    parser.add_argument("--timeout", type=int, default=900, help="seconds one wlcs process may run")
    parser.add_argument("--update", action="store_true", help="rewrite `passing` from this run")
    parser.add_argument("--check", action="store_true", help="check against expected.toml even with patterns")
    parser.add_argument("--only-passing", action="store_true", help="run only the tests `passing` lists (the CI gate)")
    parser.add_argument("--results", help="write per-test results as JSON")
    parser.add_argument("patterns", nargs="*")
    args = parser.parse_args()

    expected = load_expected()
    known = list_tests(args.wlcs, args.plugin)
    if args.patterns:
        wanted = [t for t in known if matches(t, args.patterns)]
    elif args.subset == "all":
        wanted = known
    else:
        wanted = [t for t in known if matches(t, expected["core"])]
    if args.only_passing:
        passing = set(expected.get("passing", []))
        wanted = [t for t in wanted if t in passing]
    if not wanted:
        print("no wlcs test matched", file=sys.stderr)
        return 2

    started = time.monotonic()
    results = {}
    groups = suites(wanted)
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = [pool.submit(run_group, args.wlcs, args.plugin, tests, args.timeout) for tests in groups.values()]
        for future in concurrent.futures.as_completed(futures):
            results.update(future.result())

    counts = {"pass": 0, "fail": 0, "skip": 0, "crash": 0}
    per_suite = {}
    for test, (status, _, _) in results.items():
        counts[status] += 1
        per_suite.setdefault(test.split(".")[0], dict.fromkeys(counts, 0))[status] += 1
    print(f"{'suite':<58}{'pass':>6}{'fail':>6}{'skip':>6}{'crash':>6}")
    for suite in sorted(per_suite):
        row = per_suite[suite]
        print(f"{suite:<58}{row['pass']:>6}{row['fail']:>6}{row['skip']:>6}{row['crash']:>6}")
    print(f"{'total':<58}{counts['pass']:>6}{counts['fail']:>6}{counts['skip']:>6}{counts['crash']:>6}")
    print(f"{len(results)} tests in {time.monotonic() - started:.0f}s")

    if args.results:
        Path(args.results).write_text(
            json.dumps({t: {"status": s, "ms": ms, "detail": d} for t, (s, ms, d) in sorted(results.items())}, indent=1)
        )
    if args.update:
        update_passing(results, set(wanted))
        print(f"updated {EXPECTED.relative_to(ROOT)}")
        return 0
    if args.patterns and not args.check:
        for test, (status, ms, detail) in sorted(results.items()):
            print(f"{status:>5} {test} ({ms} ms) {detail}")
        return 0
    problems = check(results, expected, set(wanted))
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

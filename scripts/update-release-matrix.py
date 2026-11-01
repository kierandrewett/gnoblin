#!/usr/bin/env python3
"""Write the list of Fedora releases that Gnoblin could build for, and what changed since the last list.

usage: scripts/update-release-matrix.py [--floors-dir DIR] [--output FILE] [--diff FILE] [--github-output FILE]
                                        [--today YYYY-MM-DD]

This script only proposes. It writes scripts/release-candidates.json and never touches scripts/package-targets.json: a
person reads the diff and promotes a candidate by adding or editing its inventory entry. The workflows build what the
inventory lists (scripts/fedora-releases.py), and scripts/check-packaging-targets.py warns when the two disagree.

A release is observed three ways, and the three must agree before it is a candidate:

  endoflife.date  https://endoflife.date/api/fedora.json: the cycles and their end-of-life dates
  official        Fedora's own release data: Bodhi, https://bodhi.fedoraproject.org/releases/, gives the state, the
                  release date (released_on) and the end-of-life date (eol) that the Fedora project set, and
                  dl.fedoraproject.org/pub/fedora/linux/releases/<release>/ is where the project publishes a release
                  once it is out. Bodhi has no release date for some older releases, so the published directory
                  stands in for it
  image           the fedora:<release> tag on Docker Hub, which the build container uses

  released     endoflife.date lists it, Fedora says it is out and not past its end of life, and the image exists. The
               newest four are candidates.
  pre-release  Fedora's release date is still ahead and the release is not published, endoflife.date does not list it,
               and the image exists. It is a candidate with "prerelease": true, because a tag can exist early and only
               Fedora's own date says when the release is out.

Anything else that looks like a release goes to "needs_review" with the three observations, never to the candidates. A
release that Bodhi says is still rawhide (state pending) is not a candidate, whatever its image tag says.

--floors-dir holds the result of the build-floor check of each release, as fedora-<release>.json
({"floors_met": true, "evidence": "..."}, written by .github/workflows/release-matrix.yml from
scripts/install-fedora-build-deps.sh and scripts/check-build-deps.py in the release's image) or as fedora-<release>.txt
(the PASS/FAIL/MISSING lines of a pkg-config probe). A release without a result has "floors_met": null and is not
eligible. A release that fails its floors is listed but is not eligible.

--github-output appends changed=true or changed=false for a workflow.
"""

import argparse
import datetime
import json
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "scripts/release-candidates.json"
ENDOFLIFE_URL = "https://endoflife.date/api/fedora.json"
BODHI_URL = "https://bodhi.fedoraproject.org/releases/?rows_per_page=100&exclude_archived=true"
IMAGE_URL = "https://hub.docker.com/v2/repositories/library/fedora/tags/{release}"
PUBLISHED_URL = "https://dl.fedoraproject.org/pub/fedora/linux/releases/{release}/"
KEEP_NEWEST = 4
FAMILY = "fedora"
# What a change of each field is called in the diff, in the order they are listed.
FIELDS = ("released", "eol", "prerelease", "floors_met", "evidence")


def fetch_json(url):
    request = urllib.request.Request(
        url, headers={"Accept": "application/json", "User-Agent": "gnoblin-release-matrix"}
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def image_exists(release):
    try:
        fetch_json(IMAGE_URL.format(release=release))
    except urllib.error.HTTPError as exception:
        if exception.code == 404:
            return False
        raise
    return True


def is_published(release):
    """True when the Fedora project has published the release. An unpublished one answers 403 or 404."""
    request = urllib.request.Request(PUBLISHED_URL.format(release=release), method="HEAD")
    try:
        urllib.request.urlopen(request, timeout=60).close()
    except urllib.error.HTTPError as exception:
        if exception.code in (403, 404):
            return False
        raise
    return True


def date_or_none(value):
    if not value or value is True:
        return None
    return datetime.date.fromisoformat(str(value)[:10])


def endoflife_observations(cycles):
    """release -> {"released": date, "eol": date or None} for every cycle that endoflife.date lists."""
    result = {}
    for cycle in cycles:
        if str(cycle.get("cycle", "")).isdigit():
            result[str(cycle["cycle"])] = {
                "released": date_or_none(cycle.get("releaseDate")),
                "eol": date_or_none(cycle.get("eol")),
            }
    return result


def official_observations(releases):
    """release -> {"state", "released", "eol"} from Bodhi. The F43C and F43F entries are other content types."""
    result = {}
    for entry in releases:
        match = re.fullmatch(r"F(\d+)", entry.get("name", ""))
        if match:
            result[match.group(1)] = {
                "state": entry.get("state"),
                "released": date_or_none(entry.get("released_on")),
                "eol": date_or_none(entry.get("eol")),
            }
    return result


def describe(value):
    return "absent" if value is None else value


def observations(release, api, official, image, published):
    """The three observations of one release, as the short strings that the file shows."""
    if release in api:
        entry = api[release]
        listed = f"listed, released {describe(entry['released'])}, eol {describe(entry['eol'])}"
    else:
        listed = "absent"
    if release in official:
        entry = official[release]
        bodhi = f"state {entry['state']}, released_on {describe(entry['released'])}, eol {describe(entry['eol'])}"
    else:
        bodhi = "absent"
    return {
        "endoflife_date": listed,
        "official": f"Bodhi: {bodhi}; releases/{release}/ {'published' if published else 'not published'}",
        "image": f"fedora:{release} {'exists' if image else 'does not exist'}",
    }


def classify(release, api, official, image, published, today):
    """Return "released", "prerelease", "ignore" (not a release yet, or over) or "review" (the sources disagree)."""
    entry = official.get(release)
    listed = api.get(release)
    listed_over = listed is not None and listed["eol"] is not None and listed["eol"] <= today
    if entry is None:
        return "ignore" if listed is None or listed_over else "review"
    if entry["state"] == "pending":
        return "ignore"
    dated = entry["released"]
    out = dated <= today if dated else published
    over = entry["eol"] is not None and entry["eol"] <= today
    if out:
        if over:
            return "ignore" if listed is None or listed_over else "review"
        listed_out = listed is not None and listed["released"] is not None and listed["released"] <= today
        return "released" if listed_out and not listed_over and image and published else "review"
    # Fedora's release date is still ahead.
    return "prerelease" if dated and listed is None and image and not published else "review"


def read_floors(floors_dir, release):
    """Return (floors_met, evidence) from the result of the floor check of one release, or (None, reason)."""
    if floors_dir is None:
        return None, "not checked"
    path = floors_dir / f"fedora-{release}.json"
    if path.is_file():
        result = json.loads(path.read_text())
        return bool(result["floors_met"]), str(result["evidence"])
    path = floors_dir / f"fedora-{release}.txt"
    if path.is_file():
        rows = [
            line.split("\t")
            for line in path.read_text().splitlines()
            if line.split("\t")[0] in ("PASS", "FAIL", "MISSING")
        ]
        failed = [row for row in rows if row[0] != "PASS"]
        if not rows:
            return None, f"{path.name} has no PASS, FAIL or MISSING lines"
        if failed:
            shown = "; ".join(
                f"{row[0]} {row[1]} {row[2]} found {row[3] if len(row) > 3 else 'nothing'}" for row in failed[:3]
            )
            return False, f"{len(failed)} of {len(rows)} pkg-config floors not met: {shown}"
        found = {row[1]: row for row in rows}
        samples = ", ".join(
            f"{name} {found[name][2]} found {found[name][3]}"
            for name in ("libinput", "glib-2.0", "gtk4")
            if name in found
        )
        return True, f"all {len(rows)} pkg-config floors met ({samples})"
    return None, "no floor result for this release"


def build(today, endoflife, bodhi, images, published, floors_dir=None):
    """Return the candidate file as a dictionary.

    images is the set of releases whose fedora:<release> tag exists, and published the set that Fedora has published.
    """
    api = endoflife_observations(endoflife)
    official = official_observations(bodhi)
    candidates, review, ignored = [], [], []
    for release in sorted(set(api) | set(official), key=int):
        image = release in images
        state = classify(release, api, official, image, release in published, today)
        shown = observations(release, api, official, image, release in published)
        if state == "ignore":
            ignored.append(release)
            continue
        if state == "review":
            review.append({"family": FAMILY, "release": release, "observations": shown})
            continue
        entry = official[release]
        floors_met, evidence = read_floors(floors_dir, release)
        candidates.append(
            {
                "family": FAMILY,
                "release": release,
                "image": f"fedora:{release}",
                "released": str(entry["released"] or api[release]["released"]),
                "eol": str(entry["eol"]) if entry["eol"] else None,
                "prerelease": state == "prerelease",
                "floors_met": floors_met,
                "evidence": evidence,
                "eligible": floors_met is True,
                "observations": shown,
            }
        )
    released = [item for item in candidates if not item["prerelease"]]
    keep = {item["release"] for item in released[-KEEP_NEWEST:]}
    candidates = [item for item in candidates if item["prerelease"] or item["release"] in keep]
    return {
        "schema_version": 1,
        "generated": str(today),
        "sources": {"endoflife_date": ENDOFLIFE_URL, "official": [BODHI_URL, PUBLISHED_URL], "image": IMAGE_URL},
        "candidates": candidates,
        "needs_review": review,
    }


def diff(previous, current):
    """A Markdown list of what changed between two candidate files; empty when nothing did."""
    lines = []
    before = {item["release"]: item for item in previous.get("candidates", [])}
    after = {item["release"]: item for item in current["candidates"]}
    for release in sorted(after.keys() - before.keys(), key=int):
        item = after[release]
        kind = "pre-release" if item["prerelease"] else "release"
        lines.append(f"- New candidate: Fedora {release} ({kind}, eol {item['eol']}). Floors: {item['evidence']}")
    for release in sorted(before.keys() - after.keys(), key=int):
        lines.append(
            f"- No longer a candidate: Fedora {release}. Review its inventory entry in scripts/package-targets.json."
        )
    for release in sorted(after.keys() & before.keys(), key=int):
        for field in FIELDS:
            if before[release].get(field) != after[release][field]:
                lines.append(
                    f"- Fedora {release} {field}: {before[release].get(field)!r} became {after[release][field]!r}"
                )
    old_review = {item["release"]: item for item in previous.get("needs_review", [])}
    new_review = {item["release"]: item for item in current["needs_review"]}
    for release in sorted(new_review.keys() - old_review.keys(), key=int):
        lines.append(f"- Needs review: Fedora {release}. The sources disagree.")
        for source, text in new_review[release]["observations"].items():
            lines.append(f"  - {source}: {text}")
    for release in sorted(old_review.keys() - new_review.keys(), key=int):
        lines.append(f"- Fedora {release} no longer needs review.")
    return "\n".join(lines)


def collect(today):
    endoflife = fetch_json(ENDOFLIFE_URL)
    bodhi = fetch_json(BODHI_URL)["releases"]
    wanted = set(endoflife_observations(endoflife)) | set(official_observations(bodhi))
    # Only the releases that the two sources name need a look-up, and the old ones are over.
    wanted = {release for release in wanted if int(release) >= lowest_live(endoflife, today)}
    return (
        endoflife,
        bodhi,
        {release for release in wanted if image_exists(release)},
        {release for release in wanted if is_published(release)},
    )


def lowest_live(endoflife, today):
    """The lowest release number that can still be a candidate: the oldest cycle that has not reached end of life."""
    live = [
        int(cycle["cycle"])
        for cycle in endoflife
        if str(cycle.get("cycle", "")).isdigit() and (date_or_none(cycle.get("eol")) or datetime.date.max) > today
    ]
    return min(live) if live else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--floors-dir", type=Path, help="directory of fedora-<release>.json or .txt floor results")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--diff", type=Path, help="write the Markdown diff to this file")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--today", type=datetime.date.fromisoformat, default=datetime.date.today())
    arguments = parser.parse_args()

    endoflife, bodhi, images, published = collect(arguments.today)
    current = build(arguments.today, endoflife, bodhi, images, published, arguments.floors_dir)
    previous = json.loads(arguments.output.read_text()) if arguments.output.is_file() else {}
    changes = diff(previous, current)
    text = changes or "No changes."
    print(text)
    if arguments.diff:
        arguments.diff.write_text(text + "\n")
    if arguments.github_output:
        with arguments.github_output.open("a") as output:
            output.write(f"changed={'true' if changes else 'false'}\n")
    arguments.output.write_text(json.dumps(current, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

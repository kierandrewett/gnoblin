#!/usr/bin/env python3
"""Build the live 500 + 300 Gnoblin desktop-app compatibility catalog.

The first source is Flathub's public Popular collection. The second is Fedora
AppStream's RPM catalogue, selected deterministically across its app categories
and excluding desktop IDs already present in the Flathub sample.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
from datetime import datetime, timezone
import gzip
import hashlib
import json
from pathlib import Path
import sys
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET


FLATHUB_COLLECTION = "https://flathub.org/api/v2/collection/popular"
FLATHUB_PAGE_SIZE = 250
FEDORA_APPSTREAM_PACKAGE = "appstream-data"


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def child_text(element: ET.Element, name: str) -> str | None:
    for child in element:
        if local_name(child.tag) == name and child.text:
            return child.text.strip()
    return None


def fetch_flathub(limit: int) -> list[dict]:
    apps: list[dict] = []
    seen: set[str] = set()
    page = 1
    while len(apps) < limit:
        query = urllib.parse.urlencode({"page": page, "per_page": FLATHUB_PAGE_SIZE})
        request = urllib.request.Request(
            f"{FLATHUB_COLLECTION}?{query}",
            headers={"User-Agent": "gnoblin-e2e-catalog/1.0"},
        )
        with urllib.request.urlopen(request, timeout=45) as response:
            result = json.load(response)
        hits = result.get("hits")
        if not isinstance(hits, list) or not hits:
            raise RuntimeError(f"Flathub Popular ended at page {page}; found {len(apps)}/{limit}")
        for rank, hit in enumerate(hits, start=(page - 1) * FLATHUB_PAGE_SIZE + 1):
            app_id = hit.get("app_id")
            if hit.get("type") != "desktop-application" or not isinstance(app_id, str):
                continue
            if app_id in seen:
                continue
            seen.add(app_id)
            apps.append({
                "source": "flathub-popular",
                "source_rank": rank,
                "app_id": app_id,
                "name": hit.get("name") or app_id,
                "summary": hit.get("summary") or "",
                "category": hit.get("main_categories") or "Other",
                "install": app_id,
                "launch": app_id,
            })
            if len(apps) == limit:
                break
        page += 1
        if page > int(result.get("totalPages", page)) + 1 and len(apps) < limit:
            raise RuntimeError(f"Flathub Popular has only {len(apps)} unique desktop apps")
    return apps


def parse_component(component: ET.Element) -> dict | None:
    if component.attrib.get("type") not in {"desktop", "desktop-application"}:
        return None
    app_id = child_text(component, "id")
    package = child_text(component, "pkgname")
    desktop_id = None
    categories: list[str] = []
    for element in component.iter():
        name = local_name(element.tag)
        if name == "launchable" and element.attrib.get("type") == "desktop-id" and element.text:
            desktop_id = element.text.strip()
        elif name == "category" and element.text:
            categories.append(element.text.strip())
    if not app_id or not package:
        return None
    if not desktop_id and app_id.endswith(".desktop"):
        desktop_id = app_id
        app_id = app_id.removesuffix(".desktop")
    if not desktop_id:
        return None
    if not desktop_id.endswith(".desktop"):
        desktop_id += ".desktop"
    return {
        "source": "fedora-appstream",
        "app_id": app_id,
        "desktop_id": desktop_id,
        "name": child_text(component, "name") or app_id,
        "summary": child_text(component, "summary") or "",
        "category": categories[0] if categories else "Other",
        "install": package,
        "launch": desktop_id.removesuffix(".desktop"),
    }


def fetch_fedora(path: Path, limit: int, excluded_ids: set[str]) -> list[dict]:
    candidates: dict[str, dict] = {}
    with gzip.open(path, "rb") as source:
        for _, element in ET.iterparse(source, events=("end",)):
            if local_name(element.tag) != "component":
                continue
            record = parse_component(element)
            if record and record["app_id"] not in excluded_ids and record["desktop_id"] not in excluded_ids:
                candidates.setdefault(record["app_id"], record)
            element.clear()

    by_category: dict[str, list[dict]] = defaultdict(list)
    for app in candidates.values():
        by_category[app["category"]].append(app)
    for apps in by_category.values():
        apps.sort(key=lambda app: hashlib.sha256(app["app_id"].encode()).digest())

    categories = sorted(by_category)
    selected: list[dict] = []
    while len(selected) < limit:
        advanced = False
        for category in categories:
            apps = by_category[category]
            if apps:
                selected.append(apps.pop(0))
                advanced = True
                if len(selected) == limit:
                    break
        if not advanced:
            raise RuntimeError(
                f"Fedora AppStream has only {len(selected)} unique non-Flathub launchable apps; "
                f"need {limit}"
            )
    return selected


def make_catalog(fedora_xml: Path, flathub_count: int, fedora_count: int) -> dict:
    flathub = fetch_flathub(flathub_count)
    excluded = {app_id for app in flathub for app_id in (app["app_id"], app["app_id"] + ".desktop")}
    fedora = fetch_fedora(fedora_xml, fedora_count, excluded)
    return {
        "schema": 1,
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "fedora_appstream_sha256": hashlib.sha256(fedora_xml.read_bytes()).hexdigest(),
        "sources": {
            "store": FLATHUB_COLLECTION,
            "fedora": str(fedora_xml),
            "fedora_package": FEDORA_APPSTREAM_PACKAGE,
        },
        "requested_counts": {"flathub_popular": flathub_count, "fedora_appstream": fedora_count},
        "counts": {"flathub_popular": len(flathub), "fedora_appstream": len(fedora)},
        "apps": flathub + fedora,
    }


def shard_catalog(catalog: dict, index: int, count: int) -> dict:
    if count < 1 or not 0 <= index < count:
        raise ValueError("shard must satisfy 0 <= index < count")
    groups: dict[str, list[dict]] = defaultdict(list)
    for app in catalog["apps"]:
        groups[app["source"]].append(app)
    selected: list[dict] = []
    for apps in groups.values():
        selected.extend(apps[index::count])
    return {
        "schema": catalog["schema"],
        "catalog_generated_utc": catalog["generated_utc"],
        "fedora_appstream_sha256": catalog.get("fedora_appstream_sha256"),
        "shard": {"index": index, "count": count},
        "expected_counts": catalog["counts"],
        "apps": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fedora-appstream", type=Path, default=Path("/usr/share/swcatalog/xml/fedora.xml.gz"))
    parser.add_argument("--catalog-in", type=Path, help="shard an existing catalog without refetching it")
    parser.add_argument("--flathub-count", type=int, default=500)
    parser.add_argument("--fedora-count", type=int, default=300)
    parser.add_argument("--output", type=Path, help="write the full catalog here")
    parser.add_argument("--shard-index", type=int)
    parser.add_argument("--shard-count", type=int, default=40)
    parser.add_argument("--shard-output", type=Path)
    args = parser.parse_args()
    if args.catalog_in:
        catalog = json.loads(args.catalog_in.read_text())
    else:
        if not args.fedora_appstream.is_file():
            raise SystemExit(f"Fedora AppStream metadata is missing: {args.fedora_appstream}")
        catalog = make_catalog(args.fedora_appstream, args.flathub_count, args.fedora_count)
    actual_counts = defaultdict(int)
    for app in catalog.get("apps", []):
        actual_counts[app.get("source")] += 1
    expected = {"flathub-popular": args.flathub_count, "fedora-appstream": args.fedora_count}
    if not args.catalog_in and any(actual_counts[source] != count for source, count in expected.items()):
        raise RuntimeError(f"catalog minimums not met: found {dict(actual_counts)}, expected {expected}")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
        print("Catalog counts:", json.dumps(catalog["counts"], sort_keys=True))
        print(f"Catalog written to {args.output}")
    if args.shard_index is not None:
        if args.shard_output is None:
            raise SystemExit("--shard-output is required with --shard-index")
        shard = shard_catalog(catalog, args.shard_index, args.shard_count)
        args.shard_output.parent.mkdir(parents=True, exist_ok=True)
        args.shard_output.write_text(json.dumps(shard, indent=2, sort_keys=True) + "\n")
        print(f"Shard {args.shard_index}/{args.shard_count}: {len(shard['apps'])} apps")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"catalog generation failed: {error}", file=sys.stderr)
        raise SystemExit(1)

#!/usr/bin/env python3
"""List the systems that Gnoblin lists, builds and packages for, from live release data.

usage: scripts/supported-targets.py [--table] [--github-output FILE] [--today YYYY-MM-DD]

The workflows ask this script which jobs to start. Nothing it prints is kept in the repository, and no workflow names a
release, an image or a family. It reads two inputs:

  scripts/targets.json  The only file that a person edits. At the top level:
                          newest         how many of the newest releases of a family are listed
                          window_years   a release first published in this many years is listed as well
                          architectures  the architectures of every job: {"arch": "amd64", "runner": "ubuntu-latest",
                                         "guest": {...}}. "guest" is the QEMU guest of the graphical VM test on that
                                         architecture: cpu and boot (the names in the cloud image files, x86_64 and
                                         bios, aarch64 and uefi), qemu (the emulator), machine, tcg_cpu (the CPU
                                         model without KVM), gpu, firmware (UEFI code and vars files to try in order,
                                         empty for the default BIOS), host_packages (the apt packages of the runner)
                                         and timeout_minutes (the job)
                          graphical_newest         optional. The graphical VM test covers this many of the newest
                                                   releases of each family, with or without packages (below)
                          graphical_architectures  the architectures of the graphical VM test, ["amd64", "arm64"].
                                                   A target on another architecture has no graphical result, so the
                                                   release gate never publishes its packages
                        Each family names:
                          product           the endoflife.date product (https://endoflife.date/api/<product>.json)
                          image             the container image of a release, with {release} in it. It builds the
                                            prefix and runs the install check, unless install_image says otherwise
                          install_image     optional. The image of the install check, when it is not image
                          install_prepare   optional. A command that the install check runs in its image first, for
                                            a tool that the image lacks and the check needs: bash on Alpine,
                                            update-alternatives on a minimal openSUSE Leap image
                          <arch>_image      optional, for example arm64_image. The install image on that architecture,
                                            for an image that has no build for it
                          slug              the name of the release in job and artifact names and in the revision of a
                                            deb, with {release} in it: fc43, deb13, ubuntu26.04
                          kind              the package format of the system: rpm, deb, apk or pacman
                          prefix            "own": the release builds a prefix on its own image (scripts/build-prefix.sh).
                                            "shared": it installs the packages that the shared prefix made
                          bootstrap         own prefix only. The command that installs git, tar, xz and zstd in the image
                          deps_script       own prefix only. The script in scripts/ that installs the build requirements
                          lane              own prefix only. The package lane that writes the packages of this release
                                            (scripts/build-packages.sh --lanes): fedora (the rpm of the release), deb
                                            (the debs, with the slug in their revision), opensuse (the openSUSE rpm,
                                            with the slug in its release) or other (the apks, from a musl prefix).
                                            Without a lane, the prefix only feeds the shared packages
                          published         optional. The packages of this release are release assets and go into the
                                            repository. The others are install checks only
                          lts_only          optional. List only the releases that endoflife.date marks as long-term
                                            support. Ubuntu uses it: an interim release lives 9 months, so it is not
                                            a compatibility target
                          build_from        the oldest release that a CI job covers, or null when no release has a job
                          verified_from     the oldest and the newest release that a full run on GitHub has passed
                          verified_through  on, or both null when none has. The releases between them are built. The
                                            other releases from build_from up are experimental
                          planned_reason    why each listed release without a job has none: an object that maps the
                                            release to the reason, such as the libraries that are below the build
                                            floors. A listed planned release without an entry is an error
                          experimental_reason
                                            optional. An object that maps an experimental release to a note that
                                            replaces the default one, for a known cause of a failing job
                        Exactly one family names source_release. Its prefix builds the source bundle, and makes the
                        shared packages (the openSUSE rpm, deb, Arch and tar packages) too, because the systems that
                        install them are newer than that release. It may name "upcoming": "bodhi" (below).
                        The file also lists the rolling targets: the same data without a release, and without
                        build_from, verified_through and a state. A rolling target with its own prefix and
                        "shared_source": true feeds the shared packages too (Alpine edge makes the apk).
                        With graphical_newest, each family and rolling target names "graphical": the cloud image
                        of the graphical VM test (tests/graphical/harness.py). "index" lists the folders that hold
                        it, the first that has it wins; "file" and "checksum" are regular expressions of the image
                        and its checksum file there ({file} in checksum is the image name). {release},
                        {codename} (the endoflife.date codename, in lower case, first word), {arch} (amd64, arm64)
                        and the guest's {cpu} and {boot} are replaced. A key named after an architecture ("arm64")
                        replaces index, file or checksum on that architecture, or names "none": the reason that the
                        system publishes no image for it. A rolling target without its own image names "base", a
                        family whose newest image it boots, and "upgrade", the command that moves that guest to the
                        rolling release.
  endoflife.date        The release date and the end-of-life date of each release.

Which releases are listed: a release of a family is listed when it is among the newest "newest" releases of the family
by release date, or when it was first published in the last "window_years" years. End of life does not remove a
release. It is shown, nothing more. A family that lists only long-term support releases is filtered first.

The state of a listed release:

  built         verified_from <= release <= verified_through. A CI job builds or install-checks it, and a failure
                fails the run.
  experimental  build_from <= release, and the release is not built: no full run has passed on it yet. Its job runs,
                and a failure does not fail the run. A release that is not out yet is experimental too.
  planned       release < build_from, or the family has no build_from. It has no job. planned_reason says why.

Fedora names a release before endoflife.date lists it. For "upcoming": "bodhi" the script reads Fedora's release data
(Bodhi) and adds a release that is newer than every cycle that endoflife.date lists, that Bodhi dates and does not call
pending, and whose container image exists.

Output, as one JSON object. The jobs get only built and experimental releases. One target is one entry for each
architecture, so every list below is a release x architecture matrix:

  matrix         every entry. The install check of each entry runs on install_image, with the packages of
                 install_artifact-<tag>install_suffix-<arch>
  prefixes       the entries with an own prefix. Each builds the prefix on prefix_image: bootstrap, then deps_script,
                 then scripts/build-prefix.sh. The artifact is <prefix_artifact>-<tag>-<slug>-<arch>. The artifacts of
                 the shared sources are called prefix-shared-...
  packages       the entries with a lane. Each writes its packages with scripts/build-packages.sh package_args from its
                 own prefix, into <packages_artifact>-<tag>-<slug>-<arch>. packages_artifact is "packages" for a
                 published release and "testpkgs" for an install check only, which no repository and no GitHub
                 release takes
  architectures  the architectures of the jobs that do not belong to one release (the shared packages)
  compile        one entry for each family that has a floor: the release at build_from, on the first architecture. The
                 compile gate of the oldest release of each family
  source         the release that builds the source bundle (source_release)
  source_image   its image
  graphical      with graphical_newest only. One entry for each release of the graphical VM test
                 (.github/workflows/graphical.yml): the newest graphical_newest releases of each family by release
                 date, packaged or not, the rolling targets, and every other target with packages, on
                 graphical_architectures. Each entry has the family, release, slug, end-of-life date (eol_date) and
                 eol (true when that date is today or earlier; a rolling target is never at its end of life), state,
                 packaged (a package job builds the release), reason (the planned_reason of a release without
                 packages), the guest data of the architecture, the resolved image data (null, with image_missing,
                 when the system publishes no image for the architecture) and the package artifact of the release
                 (install_artifact, install_suffix)
  graphical_packaged  the graphical entries with packages: the graphical jobs of the release workflow

--table prints every listed release of every family with its state, the planned ones too, a summary for each family,
the rolling targets, and how many jobs each stage starts.
--github-output appends one NAME=VALUE line for each output to FILE, which is how a workflow reads them.
--rows adds "rows": every listed release of every family with its state and note, the planned ones too. The coverage
gate of the package suite (tests/packages/gate.py) lists the planned releases from it.
"""

import argparse
import datetime
import json
from pathlib import Path
import re
import sys
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
POLICY = ROOT / "scripts/targets.json"
ENDOFLIFE_URL = "https://endoflife.date/api/{product}.json"
BODHI_URL = "https://bodhi.fedoraproject.org/releases/?rows_per_page=100&exclude_archived=true"
DOCKER_HUB_URL = "https://hub.docker.com/v2/repositories/library/{name}/tags/{tag}"
BUILT, EXPERIMENTAL, PLANNED = "built", "experimental", "planned"
STATES = (BUILT, EXPERIMENTAL, PLANNED)
KINDS = ("rpm", "deb", "apk", "pacman")
PREFIXES = ("own", "shared")
# The package lanes of scripts/build-packages.sh: the kind of package each one writes, and its arguments. {release} is the
# release number and {slug} the name of the release (deb13), which the deb revision carries.
LANES = {
    "fedora": ("rpm", "--lanes fedora --fedora {release}"),
    "deb": ("deb", "--lanes deb --deb-release {slug}"),
    "opensuse": ("rpm", "--lanes opensuse --opensuse-release {slug}"),
    "other": ("apk", "--lanes other"),
}


def fetch_json(url):
    """GET a JSON document. endoflife.date and Bodhi both fail now and then, so a request is tried three times."""
    request = urllib.request.Request(url, headers={"Accept": "application/json", "User-Agent": "gnoblin-targets"})
    for attempt in (1, 2, 3):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.load(response)
        except (urllib.error.URLError, TimeoutError):
            if attempt == 3:
                raise
            time.sleep(5 * attempt)


def image_exists(image):
    """True when the tag exists on Docker Hub. Only the official images ("name:tag") can be checked this way."""
    name, _, tag = image.partition(":")
    if "/" in name:
        return True
    try:
        fetch_json(DOCKER_HUB_URL.format(name=name, tag=tag))
    except urllib.error.HTTPError as exception:
        if exception.code == 404:
            return False
        raise
    return True


def version_key(release):
    """ "26.10" sorts after "26.4" and before "27"."""
    return tuple(int(part) for part in release.split("."))


def is_version(release):
    return re.fullmatch(r"\d+(\.\d+)*", release) is not None


def date_or_none(value):
    if not value or value is True:
        return None
    return datetime.date.fromisoformat(str(value)[:10])


def years_before(today, years):
    try:
        return today.replace(year=today.year - years)
    except ValueError:  # 29 February
        return today.replace(year=today.year - years, day=28)


def released_cycles(family, cycles, today):
    """The numbered releases that endoflife.date lists as out, as {release: (release date, end of life or None)}."""
    result = {}
    for cycle in cycles:
        release = str(cycle.get("cycle", ""))
        released, eol = date_or_none(cycle.get("releaseDate")), date_or_none(cycle.get("eol"))
        if not is_version(release) or released is None or released > today:
            continue
        if family.get("lts_only") and not cycle.get("lts"):
            continue
        result[release] = (released, eol)
    return result


def upcoming_releases(family, cycles, bodhi, exists, today):
    """Fedora releases newer than every cycle that endoflife.date lists, that Bodhi dates and does not count as over,
    and whose image exists, as {release: (release date, end of life)}. Bodhi calls rawhide and a branch before its
    freeze "pending"; those are not releases yet. The F43C and F43F entries are other content types."""
    newest = max((version_key(str(c["cycle"])) for c in cycles if is_version(str(c["cycle"]))), default=())
    result = {}
    for item in bodhi:
        match = re.fullmatch(r"F(\d+)", item.get("name", ""))
        if not match or item.get("state") == "pending" or not item.get("released_on"):
            continue
        release, eol = match.group(1), date_or_none(item.get("eol"))
        if version_key(release) > newest and not (eol and eol <= today):
            if exists(family["image"].format(release=release)):
                result[release] = (date_or_none(item["released_on"]), eol)
    return result


def listed(releases, count, since):
    """The releases among the newest count by release date, and the releases first published on or after since."""
    newest = sorted(releases, key=lambda release: (releases[release][0], version_key(release)), reverse=True)[:count]
    return {release for release in releases if release in newest or releases[release][0] >= since}


def state(family, release, upcoming):
    floor, oldest, newest = family["build_from"], family["verified_from"], family["verified_through"]
    if floor is None or version_key(release) < version_key(floor):
        return PLANNED
    if upcoming or newest is None or not version_key(oldest) <= version_key(release) <= version_key(newest):
        return EXPERIMENTAL
    return BUILT


def note(family, release, release_state, upcoming):
    if release_state == PLANNED:
        reason = family["planned_reason"].get(release)
        if not reason:
            raise ValueError(f"{family['name']} {release} is planned, and planned_reason has no entry for it")
        return reason
    if release_state == EXPERIMENTAL:
        if release in family.get("experimental_reason", {}):
            return family["experimental_reason"][release]
        if upcoming:
            return "not out yet"
        if family["verified_through"] is None or version_key(release) < version_key(family["verified_from"]):
            return "no full run has verified it yet"
        return f"newer than {family['verified_through']}, the last verified release"
    return ""


def check_target(item):
    """Check the data of a family or a rolling target that the jobs need."""
    name = item["name"]
    if item.get("kind") not in KINDS:
        raise ValueError(f"{name}: kind is {item.get('kind')!r}, not one of {', '.join(KINDS)}")
    if not item.get("slug"):
        raise ValueError(f"{name}: slug is missing")
    if item.get("prefix") not in PREFIXES:
        raise ValueError(f"{name}: prefix is {item.get('prefix')!r}, not own or shared")
    lane = item.get("lane")
    if not isinstance(item.get("install_prepare", ""), str):
        raise ValueError(f"{name}: install_prepare is a command, as one string")
    if item["prefix"] == "shared":
        extra = [key for key in ("bootstrap", "deps_script", "lane", "published", "shared_source") if key in item]
        if extra:
            raise ValueError(f"{name}: a shared prefix has no {', '.join(extra)}")
        return
    for key in ("bootstrap", "deps_script"):
        if not item.get(key):
            raise ValueError(f"{name}: an own prefix needs {key}")
    if not (ROOT / "scripts" / item["deps_script"]).is_file():
        raise ValueError(f"{name}: scripts/{item['deps_script']} does not exist")
    if lane is not None and lane not in LANES:
        raise ValueError(f"{name}: lane is {lane!r}, not one of {', '.join(LANES)}")
    if lane is not None and LANES[lane][0] != item["kind"]:
        raise ValueError(f"{name}: the lane {lane} writes {LANES[lane][0]} packages, not {item['kind']}")
    if lane is None and not item.get("shared_source") and "source_release" not in item:
        raise ValueError(f"{name}: an own prefix needs a lane, or has to feed the shared packages")
    if item.get("published") and lane is None:
        raise ValueError(f"{name}: published needs a lane")


def check_family(family):
    name, floor = family["name"], family["build_from"]
    oldest, newest = family["verified_from"], family["verified_through"]
    check_target(family)
    if "{release}" not in family["image"] or "{release}" not in family["slug"]:
        raise ValueError(f"{name}: image and slug need {{release}}")
    for key in ("planned_reason", "experimental_reason"):
        if not isinstance(family.get(key, {}), dict):
            raise ValueError(f"{name}: {key} maps each release to its reason")
    if (oldest is None) != (newest is None):
        raise ValueError(f"{name}: verified_from and verified_through are both set or both null")
    if newest is None:
        return
    if floor is None:
        raise ValueError(f"{name}: verified_from and verified_through need a build_from")
    if version_key(oldest) < version_key(floor):
        raise ValueError(f"{name}: verified_from {oldest} is older than build_from {floor}")
    if version_key(newest) < version_key(oldest):
        raise ValueError(f"{name}: verified_through {newest} is older than verified_from {oldest}")


def target_row(item, **fields):
    """The fields of a family release or a rolling target that the entries are made of."""
    release = fields["release"]
    images = {key: value.format(release=release) for key, value in item.items() if key.endswith("image")}
    return {
        "name": fields["name"],
        "family": fields["family"],
        "release": release,
        "slug": item["slug"].format(release=release),
        "kind": item["kind"],
        "prefix": item["prefix"],
        "images": images,
        "bootstrap": item.get("bootstrap", ""),
        "deps_script": item.get("deps_script", ""),
        "lane": item.get("lane") or "",
        "install_prepare": item.get("install_prepare", ""),
        "published": bool(item.get("published")),
        "shared_source": fields["shared_source"],
        "floor": fields["floor"],
        "released": fields["released"],
        "eol": fields["eol"],
        "state": fields["state"],
        "note": fields["note"],
    }


def entry(row, architecture):
    """One job target: a row on one architecture. Everything a job needs is a field, so a workflow names nothing."""
    arch = architecture["arch"]
    images = row["images"]
    install_image = images.get(f"{arch}_image") or images.get("install_image") or images["image"]
    own_set = row["lane"] != ""
    packages_artifact = "packages" if row["published"] or not own_set else "testpkgs"
    package_args = ""
    if own_set:
        package_args = LANES[row["lane"]][1].format(release=row["release"], slug=row["slug"])
    return {
        "name": row["name"],
        "family": row["family"],
        "release": row["release"],
        "slug": row["slug"],
        "kind": row["kind"],
        "arch": arch,
        "runner": architecture["runner"],
        "experimental": row["state"] == EXPERIMENTAL,
        "floor": row["floor"],
        "prefix": row["prefix"],
        "prefix_image": images["image"] if row["prefix"] == "own" else "",
        "prefix_artifact": "prefix-shared" if row["shared_source"] else "prefix",
        "bootstrap": row["bootstrap"],
        "deps_script": row["deps_script"],
        "lane": row["lane"],
        "package_args": package_args,
        "published": row["published"],
        "packages_artifact": packages_artifact,
        "install_image": install_image,
        "install_prepare": row["install_prepare"],
        "install_artifact": packages_artifact,
        "install_suffix": f"-{row['slug']}" if own_set else "",
    }


def compute(policy, cycles, bodhi, exists, today):
    """The targets, from the policy, the endoflife.date cycles of each product, Bodhi's releases and an image check.

    cycles maps a product name to its list of cycles. exists(image) says whether a container image exists.
    """
    since = years_before(today, policy["window_years"])
    rows = []
    for family in policy["families"]:
        check_family(family)
        product_cycles = cycles[family["product"]]
        found = {release: (*dates, False) for release, dates in released_cycles(family, product_cycles, today).items()}
        if family.get("upcoming") == "bodhi":
            upcoming = upcoming_releases(family, product_cycles, bodhi, exists, today)
            found.update({release: (*dates, True) for release, dates in upcoming.items()})
        shown = listed({release: (value[0],) for release, value in found.items()}, policy["newest"], since)
        for release in sorted(shown, key=version_key, reverse=True):
            released, eol, upcoming = found[release]
            release_state = state(family, release, upcoming)
            rows.append(
                target_row(
                    family,
                    name=f"{family['name']} {release}",
                    family=family["name"],
                    release=release,
                    shared_source=release == family.get("source_release"),
                    floor=release == family["build_from"],
                    released=released.isoformat(),
                    eol=eol.isoformat() if eol else None,
                    state=release_state,
                    note=note(family, release, release_state, upcoming),
                )
            )
    for item in policy["rolling"]:
        check_target(item)
        rows.append(
            target_row(
                item,
                name=item["name"],
                family="Rolling",
                release="rolling",
                shared_source=bool(item.get("shared_source")),
                floor=False,
                released=None,
                eol=None,
                state=BUILT,
                note="",
            )
        )
    targets = [row for row in rows if row["state"] != PLANNED]

    sources = [family for family in policy["families"] if "source_release" in family]
    if len(sources) != 1:
        raise ValueError(f"the policy needs one family with a source_release, not {len(sources)}")
    source = next((row for row in targets if row["shared_source"] and row["family"] == sources[0]["name"]), None)
    if source is None:
        raise ValueError(f"{sources[0]['name']} {sources[0]['source_release']} (source_release) is not a target")
    if source["state"] != BUILT:
        raise ValueError(f"{source['name']} (source_release) is {source['state']} and cannot make the shared packages")
    if any(row["shared_source"] and row["state"] != BUILT for row in targets):
        raise ValueError("a shared source is not built, and the shared packages come from every shared source")

    order = {name: number for number, name in enumerate([*(f["name"] for f in policy["families"]), "Rolling"])}
    ordered = sorted(
        targets,
        key=lambda row: (order[row["family"]], () if row["release"] == "rolling" else version_key(row["release"])),
    )
    matrix = [entry(row, architecture) for row in ordered for architecture in policy["architectures"]]
    first = policy["architectures"][0]["arch"]
    graphical_entries = graphical(policy, cycles, targets, today)
    release_graphical = [item for item in graphical_entries if item["packaged"]]
    graphical_of = {(item["slug"], item["arch"]): item for item in release_graphical}
    legs = [{**item, "graphical": graphical_of.get((item["slug"], item["arch"]))} for item in matrix]
    return {
        "matrix": matrix,
        "prefixes": [item for item in matrix if item["prefix"] == "own"],
        "packages": [item for item in matrix if item["lane"]],
        "shared_prefixes": [item for item in matrix if item["prefix"] == "own" and needs_shared(item)],
        "legs_own": [leg for leg in legs if not needs_shared(leg)],
        "legs_shared": [leg for leg in legs if needs_shared(leg)],
        "architectures": policy["architectures"],
        "compile": [item for item in matrix if item["floor"] and item["arch"] == first and item["prefix"] == "own"],
        "source": source["release"],
        "source_image": source["images"]["image"],
        "rows": [{**row, "image": row["images"]["image"]} for row in rows],
        "rule": rule(policy),
        "jobs": jobs(policy, matrix, legs),
        "graphical": graphical_entries,
        "graphical_packaged": release_graphical,
    }


def needs_shared(item):
    """True when the target leg waits for the shared stage of the release workflow: its prefix is a shared prefix,
    which the shared stage builds, or it installs the shared packages."""
    return item["prefix_artifact"] == "prefix-shared" or not item["lane"]


def graphical_image(item, release, codename, families, architecture):
    """The image data of one graphical target on one architecture, with {release}, {codename}, {arch}, {cpu} and {boot}
    replaced, or None with the reason when the system publishes no image for that architecture."""
    data = item.get("graphical")
    if not isinstance(data, dict):
        raise ValueError(f"{item['name']}: graphical names the cloud image of the graphical VM test")
    upgrade = ""
    if "base" in data:
        if data["base"] not in families or not data.get("upgrade"):
            raise ValueError(f"{item['name']}: graphical base names a family, and needs an upgrade command")
        upgrade = data["upgrade"]
        data = families[data["base"]]["graphical"]
    arch = architecture["arch"]
    override = data.get(arch, {})
    if "none" in override:
        return None, override["none"]
    if "build" in override:
        # The system publishes no image for this architecture: the harness builds one with this script.
        build = override["build"]
        for key in ("script", "file"):
            if not build.get(key):
                raise ValueError(f"{item['name']}: graphical {arch} build needs {key}")
        return {"build": build, "upgrade": upgrade}, ""
    data = {**data, **override}
    if not data.get("index") or not data.get("file") or not data.get("checksum"):
        raise ValueError(f"{item['name']}: graphical needs index, file and checksum")
    guest = architecture.get("guest") or {}

    def fill(text, pattern):
        values = {"release": release, "codename": codename, "arch": arch}
        values.update({key: guest.get(key) for key in ("cpu", "boot")})
        for key, value in values.items():
            if f"{{{key}}}" in text:
                if value is None:
                    raise ValueError(f"{item['name']}: graphical names {{{key}}}, and {arch} has no guest {key}")
                text = text.replace(f"{{{key}}}", re.escape(value) if pattern else value)
        return text

    image = {
        "index": [fill(url, False) for url in data["index"]],
        "file": fill(data["file"], True),
        "checksum": fill(data["checksum"], True),
        "upgrade": upgrade,
    }
    return image, ""


def graphical(policy, cycles, targets, today):
    """The entries of the graphical VM test: the newest graphical_newest releases of each family and the rolling
    targets, and every other target with packages, on each graphical architecture. A release without a package job
    is an entry too: it fails as "not packaged", with its planned_reason as the blocker. A target with packages is
    always an entry, so the release gate has a graphical result to read for every target and architecture."""
    if "graphical_newest" not in policy:
        return []
    count = policy["graphical_newest"]
    architectures = [item for item in policy["architectures"] if item["arch"] in policy["graphical_architectures"]]
    families = {family["name"]: family for family in policy["families"]}
    built = {row["name"]: row for row in targets}
    newest = {}
    added = set()
    result = []

    def add(item, row, image_release, codename, **fields):
        added.add(fields["name"])
        for architecture in architectures:
            job = entry(row, architecture) if row else {}
            image, missing = graphical_image(item, image_release, codename, families, architecture)
            result.append(
                {
                    **fields,
                    "slug": item["slug"].format(release=fields["release"]),
                    "kind": item["kind"],
                    "arch": architecture["arch"],
                    "runner": architecture["runner"],
                    "guest": architecture.get("guest"),
                    "state": row["state"] if row else PLANNED,
                    "packaged": row is not None,
                    "image": image,
                    "image_missing": missing,
                    "install_artifact": job.get("install_artifact", ""),
                    "install_suffix": job.get("install_suffix", ""),
                }
            )

    codenames = {}
    for family in policy["families"]:
        product_cycles = cycles[family["product"]]
        found = released_cycles(family, product_cycles, today)
        codenames[family["name"]] = {
            str(c.get("cycle")): (str(c.get("codename") or "").split() or [""])[0].lower() for c in product_cycles
        }
        releases = sorted(found, key=lambda release: (found[release][0], version_key(release)), reverse=True)[:count]
        newest[family["name"]] = (releases[0], codenames[family["name"]].get(releases[0], "")) if releases else None
        for release in sorted(releases, key=version_key, reverse=True):
            released, eol = found[release]
            name = f"{family['name']} {release}"
            row = built.get(name)
            reason = ""
            if row is None:
                reason = family["planned_reason"].get(release) or (
                    f"no package job builds it: it is older than {family['build_from']}, the oldest release with one"
                    if family["build_from"]
                    else "no package job builds it"
                )
            add(
                family,
                row,
                release,
                codenames[family["name"]].get(release, ""),
                name=name,
                family=family["name"],
                release=release,
                released=released.isoformat(),
                eol_date=eol.isoformat() if eol else None,
                eol=eol is not None and eol <= today,
                reason=reason,
            )
    # The targets outside the window: an older release that is still built, or one that is not out yet.
    for row in targets:
        if row["family"] == "Rolling" or row["name"] in added:
            continue
        eol = date_or_none(row["eol"])
        add(
            families[row["family"]],
            row,
            row["release"],
            codenames[row["family"]].get(row["release"], ""),
            name=row["name"],
            family=row["family"],
            release=row["release"],
            released=row["released"],
            eol_date=row["eol"],
            eol=eol is not None and eol <= today,
            reason="",
        )
    for item in policy["rolling"]:
        base = item.get("graphical", {}).get("base")
        release, codename = "", ""
        if base:
            if not newest.get(base):
                raise ValueError(f"{item['name']}: graphical base names a family with a release, not {base}")
            release, codename = newest[base]
        row = built.get(item["name"])
        add(
            item,
            row,
            release,
            codename,
            name=item["name"],
            family="Rolling",
            release="rolling",
            released=None,
            eol_date=None,
            eol=False,
            reason="" if row else "no package job builds it",
        )
    order = {name: number for number, name in enumerate([*(f["name"] for f in policy["families"]), "Rolling"])}
    return sorted(
        result,
        key=lambda item: (
            order[item["family"]],
            () if item["release"] == "rolling" else tuple(-part for part in version_key(item["release"])),
        ),
    )


def jobs(policy, matrix, legs):
    """How many jobs the release workflow starts: the shared stage, then one leg (.github/workflows/target.yml) for each
    target on each architecture, and the jobs of each stage inside the legs. compile is verify.yml."""
    shared_prefixes = [item for item in matrix if item["prefix"] == "own" and needs_shared(item)]
    first = policy["architectures"][0]["arch"]
    return {
        "shared prefix": len(shared_prefixes),
        "shared packages": len(policy["architectures"]) if shared_prefixes else 0,
        "target legs": len(legs),
        "prefix": sum(leg["prefix"] == "own" and not needs_shared(leg) for leg in legs),
        "packages": sum(bool(leg["lane"]) for leg in legs),
        "package suite": len(legs),
        "graphical": sum(leg["graphical"] is not None and leg["graphical"]["image"] is not None for leg in legs),
        "verdict": len(legs),
        "compile": sum(1 for item in matrix if item["floor"] and item["prefix"] == "own" and item["arch"] == first),
    }


def rule(policy):
    """The listing rule, in words, for the table."""
    text = (
        f"Listed: the newest {policy['newest']} releases of each family by release date, and every release first "
        f"published in the last {policy['window_years']} years. End of life does not remove a release."
    )
    lts = [family["name"] for family in policy["families"] if family.get("lts_only")]
    if lts:
        text += f" {', '.join(lts)}: long-term support releases only, because an interim release lives 9 months."
    return text


def summary(rows):
    """One line for each family: how many listed releases are built, experimental and planned."""
    lines = []
    for family in dict.fromkeys(row["family"] for row in rows):
        states = [row["state"] for row in rows if row["family"] == family]
        counts = ", ".join(f"{states.count(name)} {name}" for name in STATES if name in states)
        lines.append(f"{family}: {counts}")
    return lines


def table(result):
    lines = [("System", "Release", "Released", "End of life", "State", "Note")]
    for row in result["rows"]:
        lines.append(
            (
                row["family"] if row["family"] != "Rolling" else row["name"],
                row["release"],
                row["released"] or "none",
                row["eol"] or "none",
                row["state"],
                row["note"],
            )
        )
    widths = [max(len(line[column]) for line in lines) for column in range(len(lines[0]))]
    body = "\n".join("  ".join(cell.ljust(width) for cell, width in zip(line, widths)).rstrip() for line in lines)
    count = result["jobs"]
    legs = ", ".join(
        f"{count[stage]} {stage}" for stage in ("prefix", "packages", "package suite", "graphical", "verdict")
    )
    jobs_line = (
        f"Jobs: {count['shared prefix']} shared prefix, {count['shared packages']} shared packages, "
        f"{count['target legs']} target legs ({legs}), {count['compile']} compile"
    )
    lines = [result["rule"], "", body, "", *summary(result["rows"]), "", jobs_line]
    if result["graphical"]:
        rows = result["graphical"]
        eol = sum(item["eol"] for item in rows)
        packaged = result["graphical_packaged"]
        architectures = ", ".join(dict.fromkeys(item["arch"] for item in rows))
        lines.append(
            f"Graphical VM test by hand (graphical.yml): {len(rows)} entries, planned releases too, on "
            f"{architectures}, {eol} at their end of life"
        )
        missing = [f"{item['name']} {item['arch']}" for item in packaged if item["image"] is None]
        if missing:
            lines.append(f"No graphical image yet, so the graphical stage is not run and the packages are not published: "
                         f"{', '.join(missing)}")  # fmt: skip
    return "\n".join(lines)


def collect(policy):
    products = {family["product"] for family in policy["families"]}
    cycles = {product: fetch_json(ENDOFLIFE_URL.format(product=product)) for product in products}
    needs_bodhi = any(family.get("upcoming") == "bodhi" for family in policy["families"])
    bodhi = fetch_json(BODHI_URL)["releases"] if needs_bodhi else []
    return cycles, bodhi


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--table", action="store_true", help="print every listed release and its state as a table")
    parser.add_argument("--github-output", type=Path, help="append the outputs to this GITHUB_OUTPUT file")
    parser.add_argument("--rows", action="store_true", help="add every listed release (rows), planned ones too")
    parser.add_argument("--today", type=datetime.date.fromisoformat, default=datetime.date.today())
    arguments = parser.parse_args()
    try:
        policy = json.loads(POLICY.read_text())
        cycles, bodhi = collect(policy)
        result = compute(policy, cycles, bodhi, image_exists, arguments.today)
    except (OSError, KeyError, ValueError, urllib.error.URLError) as exception:
        print(f"supported targets: {exception!r}", file=sys.stderr)
        return 1
    if arguments.table:
        print(table(result))
        return 0
    hidden = ("rule", "jobs") if arguments.rows else ("rows", "rule", "jobs")
    output = {key: value for key, value in result.items() if key not in hidden}
    print(json.dumps(output, indent=2))
    if arguments.github_output:
        with arguments.github_output.open("a") as handle:
            for key, value in output.items():
                text = value if isinstance(value, str) else json.dumps(value, separators=(",", ":"))
                handle.write(f"{key}={text}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

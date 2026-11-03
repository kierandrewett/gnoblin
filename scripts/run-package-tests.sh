#!/usr/bin/env bash
# Run the package suite (tests/packages) for one target: the static, install, runtime and repository layers.
#
# usage: scripts/run-package-tests.sh --target SLUG --arch ARCH --artifacts DIR [options]
#
#   --target SLUG, --arch ARCH  the target, as scripts/supported-targets.py names it (fc43 amd64, debian-testing arm64)
#   --targets FILE              the output of scripts/supported-targets.py. Default: run it now
#   --artifacts DIR             a folder that holds the package artifacts of the run, one folder for each, with the
#                               names that .github/workflows/packages.yml gives them (gh run download writes this)
#   --release-tag TAG           the release tag in the artifact names. Default: local
#   --report DIR                where the report goes. Default: build/package-tests/<slug>-<arch>
#   --layers LIST               a part of static,install,runtime,repository. Default: all four
#   --version SEMVER            the version the packages were written with. Default: the version of the tree
#   --prefixes DIR              the prefix archives of the run (prefix-*/prefix.tar.zst). With them, the static layer
#                               writes the packages a second time with the pinned pkgsmith and compares the bytes
#   --mutate NAME               a negative test: break a copy of the packages with tests/packages/mutate.py NAME,
#                               run the layer that must detect it, and pass only when that layer fails
#
# The install and runtime layers start clean containers of the install image of the target, and the repository layer a
# container of scripts/pkgrepo/container. CONTAINER_ENGINE names the engine and its options (default: docker, else
# podman). Every run writes report.json, junit.xml, summary.md and evidence/ (the command, the environment, the image
# digests, the package hashes and the logs of every layer). design/package-testing.md describes the layers.
set -euo pipefail

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
suite="$root/tests/packages"
slug=""
arch=""
targets_file=""
artifacts=""
release_tag=local
report=""
layers=static,install,runtime,repository
version=""
prefixes=""
mutation=""
command_line="$0 $*"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --target)
            slug="${2:?}"
            shift 2
            ;;
        --arch)
            arch="${2:?}"
            shift 2
            ;;
        --targets)
            targets_file="${2:?}"
            shift 2
            ;;
        --artifacts)
            artifacts="${2:?}"
            shift 2
            ;;
        --release-tag)
            release_tag="${2:?}"
            shift 2
            ;;
        --report)
            report="${2:?}"
            shift 2
            ;;
        --layers)
            layers="${2:?}"
            shift 2
            ;;
        --version)
            version="${2:?}"
            shift 2
            ;;
        --prefixes)
            prefixes="${2:?}"
            shift 2
            ;;
        --mutate)
            mutation="${2:?}"
            shift 2
            ;;
        *)
            sed -n '2,25p' "$0" >&2
            exit 2
            ;;
    esac
done
[ -n "$slug" ] && [ -n "$arch" ] && [ -n "$artifacts" ] || {
    echo "usage: $0 --target SLUG --arch ARCH --artifacts DIR [options]" >&2
    exit 2
}
if [ -z "${CONTAINER_ENGINE:-}" ]; then
    CONTAINER_ENGINE=docker
    command -v docker >/dev/null || CONTAINER_ENGINE=podman
fi
read -r -a engine <<<"$CONTAINER_ENGINE"
report="$(realpath -m -- "${report:-$root/build/package-tests/$slug-$arch}")"
artifacts="$(realpath -- "$artifacts")"
rm -rf -- "$report"
mkdir -p "$report/evidence"
work="$(mktemp -d "${TMPDIR:-/tmp}/gnoblin-package-tests.XXXXXX")"
cleanup() { "${engine[@]}" unshare rm -rf -- "$work" 2>/dev/null || rm -rf -- "$work" 2>/dev/null || true; }
trap cleanup EXIT

if [ -z "$targets_file" ]; then
    targets_file="$work/targets.json"
    python3 "$root/scripts/supported-targets.py" >"$targets_file"
fi
python3 "$suite/suite.py" target "$targets_file" "$slug" "$arch" >"$work/target.json"
cp "$work/target.json" "$report/target.json"
field() { python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$work/target.json" "$1"; }
artifact="$(python3 -c 'import json, sys; sys.path.insert(0, sys.argv[2]); import targets; print(targets.artifact_name(json.load(open(sys.argv[1])), sys.argv[3]))' "$work/target.json" "$suite" "$release_tag")"
image="$(field install_image)"
install_prepare="$(field install_prepare)"
[ -d "$artifacts/$artifact" ] || {
    echo "[package-tests] the artifact $artifact is not in $artifacts" >&2
    exit 1
}
echo "[package-tests] $(field name) ($arch): $artifact, image $image"
python3 "$suite/suite.py" select "$work/target.json" "$artifacts/$artifact" "$work/packages" >"$report/evidence/packages.txt"

expect_layer=""
if [ -n "$mutation" ]; then
    expect_layer="$(python3 "$suite/mutate.py" layer "$mutation")"
    layers="$expect_layer"
    python3 "$suite/mutate.py" apply "$mutation" "$work/packages" | tee "$report/evidence/mutation.txt"
fi

# The evidence: how to repeat the run, and what it ran on.
{
    echo "cd $root"
    echo "CONTAINER_ENGINE='$CONTAINER_ENGINE' $command_line"
} >"$report/evidence/command"
{
    echo "date: $(date -u +%FT%TZ)"
    echo "host: $(uname -srm)"
    echo "commit: $(git -C "$root" rev-parse HEAD 2>/dev/null || echo none)$(git -C "$root" diff --quiet HEAD 2>/dev/null || echo ' with uncommitted changes')"
    echo "engine: $("${engine[@]}" --version 2>&1 | head -n1)"
    echo "python: $(python3 --version 2>&1)"
    echo "pkgsmith pin: $(cat "$root/scripts/pkgsmith.ref")"
    echo "pkgsmith used: ${GNOBLIN_PKGSMITH_REF:-the pin}"
    for tool in rpm dpkg-deb desktop-file-validate glib-compile-schemas systemd-analyze readelf zstd; do
        echo "$tool: $(command -v "$tool" >/dev/null && "$tool" --version 2>&1 | head -n1 || echo missing)"
    done
    echo "target: $(cat "$work/target.json")"
    echo "packages:"
    (cd "$work/packages" && sha256sum -- *)
} >"$report/evidence/env.txt"

has_layer() { [[ ",$layers," == *",$1,"* ]]; }
container_run() {
    # container_run IMAGE COMMAND: the suite read-only at /suite, the repository at /repo, the packages at /packages.
    local image="$1"
    shift
    "${engine[@]}" run --rm --security-opt label=disable --network host \
        -v "$root:/repo:ro" -v "$work/packages:/packages:ro" -v "$work/out:/out" -v "$work/published:/published:ro" \
        -e INSTALL_PREPARE="$install_prepare" "$image" "$@"
}
image_digest() {
    "${engine[@]}" pull -q "$1" >/dev/null 2>&1 || true
    echo "image $1: $("${engine[@]}" image inspect --format '{{.Id}} {{json .RepoDigests}}' "$1" 2>/dev/null || echo unknown)" \
        >>"$report/evidence/env.txt"
}
mkdir -p "$work/out" "$work/published"
chmod 0777 "$work/out"

if has_layer static; then
    started="$(date +%s)"
    rebuilt_args=()
    if [ -n "$prefixes" ]; then
        # S11: write the packages of this target again from the prefixes of the run, with the pinned pkgsmith.
        mkdir -p "$work/rebuilt"
        PATH="$("$root/scripts/install-pkgsmith.sh"):$PATH"
        package_args="$(field package_args)"
        rebuild_version="${version:-$(python3 "$root/scripts/gnoblin-version.py" get version)}"
        for archive in "$prefixes"/*/prefix.tar.zst; do
            name="$(basename -- "$(dirname -- "$archive")")"
            case "$name" in
                *"-$slug-$arch" | prefix-shared-*"-$arch") ;;
                *) continue ;;
            esac
            rm -rf "$work/prefix" && mkdir "$work/prefix"
            tar --zstd -xf "$archive" -C "$work/prefix"
            if [[ $name == *"-$slug-$arch" ]] && [ -n "$package_args" ]; then
                # shellcheck disable=SC2086
                "$root/scripts/build-packages.sh" --prefix "$work/prefix" --output "$work/rebuilt" $package_args \
                    --version "$rebuild_version" >>"$report/evidence/rebuild.log" 2>&1
            fi
            if [[ $name == prefix-shared-* ]]; then
                "$root/scripts/build-packages.sh" --prefix "$work/prefix" --output "$work/rebuilt" --lanes other \
                    --version "$rebuild_version" >>"$report/evidence/rebuild.log" 2>&1
            fi
        done
        # The artifacts carry the Arch packages with a dot for the colon of the epoch.
        for file in "$work/rebuilt"/*:*; do
            [ -e "$file" ] || continue
            mv -- "$file" "$(dirname -- "$file")/$(basename -- "$file" | tr : .)"
        done
        rebuilt_args=(--rebuilt "$work/rebuilt")
        rm -rf "$work/prefix"
    fi
    python3 "$suite/static.py" --target "$work/target.json" --packages "$work/packages" --report "$report" \
        ${version:+--version "$version"} "${rebuilt_args[@]}" 2>&1 | tee "$report/evidence/static.log" || true
    echo "static layer: $(($(date +%s) - started)) s" >>"$report/evidence/timings.txt"
fi

if has_layer install; then
    started="$(date +%s)"
    python3 "$suite/suite.py" published "$work/target.json" "$targets_file" "$work/published" \
        >"$report/evidence/published.txt" 2>&1 || echo "published packages: download failed" >>"$work/published/SKIP"
    image_digest "$image"
    container_run "$image" sh -euc 'eval "$INSTALL_PREPARE"; exec bash /repo/tests/packages/install.sh /packages /out/install /published' \
        2>&1 | tee "$report/evidence/install.log" || true
    python3 "$suite/suite.py" layer install "$work/out/install/results.tsv" "$report" "$started"
    cp -r "$work/out/install/cases" "$report/evidence/install-cases" 2>/dev/null || true
    echo "install layer: $(($(date +%s) - started)) s" >>"$report/evidence/timings.txt"
fi

if has_layer runtime; then
    started="$(date +%s)"
    image_digest "$image"
    container_run "$image" sh -euc 'eval "$INSTALL_PREPARE"; exec bash /repo/tests/packages/runtime.sh /packages /out/runtime' \
        2>&1 | tee "$report/evidence/runtime.log" || true
    python3 "$suite/suite.py" layer runtime "$work/out/runtime/results.tsv" "$report" "$started"
    mkdir -p "$report/evidence/runtime"
    cp -r "$work/out/runtime/." "$report/evidence/runtime/" 2>/dev/null || true
    echo "runtime layer: $(($(date +%s) - started)) s" >>"$report/evidence/timings.txt"
fi

if has_layer repository; then
    started="$(date +%s)"
    # The image of the repository tool, as the repository stage uses it. Built once and reused.
    tool_image=localhost/gnoblin-pkgrepo-builder
    if ! "${engine[@]}" image inspect "$tool_image" >/dev/null 2>&1; then
        "${engine[@]}" build -q -t "$tool_image" -f "$root/scripts/pkgrepo/container/Containerfile" \
            "$root/scripts/pkgrepo/container" >"$report/evidence/repository-image.log" 2>&1
    fi
    cp "$work/target.json" "$work/out/target.json"
    container_run "$tool_image" python3 /repo/tests/packages/repository.py /out/target.json /packages /out/repository \
        2>&1 | tee "$report/evidence/repository.log" || true
    if [ -f "$work/out/repository/layers/repository.json" ]; then
        mkdir -p "$report/layers"
        cp "$work/out/repository/layers/repository.json" "$report/layers/"
        cp -r "$work/out/repository/evidence/repository" "$report/evidence/" 2>/dev/null || true
    fi
    echo "repository layer: $(($(date +%s) - started)) s" >>"$report/evidence/timings.txt"
fi

status=0
python3 "$suite/suite.py" finish "$work/target.json" "$report" "$release_tag" || status=$?
if [ -n "$mutation" ]; then
    # A negative test passes when the layer that must detect the broken package failed.
    expect_case="$(python3 "$suite/mutate.py" case "$mutation")"
    failed="$(python3 -c 'import json, sys; print(" ".join(c["id"] for c in json.load(open(sys.argv[1]))["layers"][sys.argv[2]]["cases"] if c["status"] == "fail"))' \
        "$report/report.json" "$expect_layer")"
    if [[ " $failed " == *" $expect_case "* ]]; then
        echo "[package-tests] negative test $mutation: the $expect_layer layer failed $expect_case on the broken package, as it must (failed: $failed)"
        exit 0
    fi
    echo "[package-tests] negative test $mutation: $expect_case of the $expect_layer layer did NOT fail (failed: ${failed:-none})" >&2
    exit 1
fi
echo "[package-tests] report: $report"
exit "$status"

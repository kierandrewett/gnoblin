#!/usr/bin/env bash
# Publish a Gnoblin GitHub release from a directory of finished assets.
#
# usage: publish-github-release.sh TAG ASSET_DIR NOTES_FILE
#
# Needs the gh CLI, GH_TOKEN and GITHUB_REPOSITORY. The release workflow calls this after every package job and
# install gate has passed, so a release never appears with a missing package. ASSET_DIR must already hold SHA256SUMS
# covering every other file; the workflow attests the assets after it writes that file.
#
# The script checks the assets and uploads everything to a draft release, and only then makes the release public. A
# reader of the releases page never sees a half-filled release. A release that is already public is never changed:
# gh release upload --clobber deletes the old asset before it uploads the new one, so a failed upload would damage a
# live release. To repair a public release, turn it back into a draft first.
set -euo pipefail
export PATH="$PATH"

if [ "$#" -ne 3 ]; then
    echo "usage: $0 TAG ASSET_DIR NOTES_FILE" >&2
    exit 2
fi
tag="$1"
asset_dir="$2"
notes_file="$3"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

[[ "$tag" =~ ^gnoblin-v[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "[release] not a release tag: $tag" >&2
    exit 2
}
[ -d "$asset_dir" ] || {
    echo "[release] missing asset directory: $asset_dir" >&2
    exit 1
}
[ -f "$notes_file" ] || {
    echo "[release] missing notes file: $notes_file" >&2
    exit 1
}

# Every release carries the source bundle and one build of each packaging route. An empty route means a job was
# skipped or uploaded nothing, which must stop the release rather than publish less than the page promises.
required_patterns=(
    'gnoblin-*-source.tar.xz'
    'gnoblin-[0-9]*.PKGBUILD'
    'gnoblin-[0-9]*-x86_64.pkg.tar.zst'
    'xdg-desktop-portal-gnoblin-[0-9]*-x86_64.pkg.tar.zst'
    'gnoblin-[0-9]*.src.rpm'
    'xdg-desktop-portal-gnoblin-[0-9]*.src.rpm'
    'gnoblin-[0-9]*.fc[0-9]*.x86_64.rpm'
    'xdg-desktop-portal-gnoblin-[0-9]*.fc[0-9]*.x86_64.rpm'
    'gnoblin-[0-9]*.tumbleweed.x86_64.rpm'
    'xdg-desktop-portal-gnoblin-[0-9]*.tumbleweed.x86_64.rpm'
)
shopt -s nullglob
for pattern in "${required_patterns[@]}"; do
    if ! compgen -G "$asset_dir/$pattern" >/dev/null; then
        echo "[release] no asset matches $pattern in $asset_dir" >&2
        exit 1
    fi
done
source_bundles=("$asset_dir"/gnoblin-*-source.tar.xz)
if [ "${#source_bundles[@]}" -ne 1 ]; then
    echo "[release] expected exactly one source bundle, found ${#source_bundles[@]}" >&2
    exit 1
fi
expected_version="${tag#gnoblin-v}"
case "$(basename "${source_bundles[0]}")" in
    gnoblin-"$expected_version"-*-source.tar.xz) ;;
    *)
        echo "[release] $(basename "${source_bundles[0]}") does not belong to $tag" >&2
        exit 1
        ;;
esac

# SHA256SUMS must list every other asset, and every listed file must match. A file added or lost after the checksum
# step, or after the attestation, would otherwise reach users unchecked.
(
    cd "$asset_dir"
    [ -f SHA256SUMS ] || {
        echo "[release] missing SHA256SUMS in $asset_dir" >&2
        exit 1
    }
    sha256sum -c SHA256SUMS >/dev/null
    diff <(find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\n' | LC_ALL=C sort) \
        <(awk '{print $2}' SHA256SUMS | sed 's/^\*//' | LC_ALL=C sort) || {
        echo "[release] SHA256SUMS does not list exactly the other assets" >&2
        exit 1
    }
)

assets=("$asset_dir"/*)
echo "[release] ${#assets[@]} assets for $tag"

state="$(gh release list --repo "$repo" --limit 200 --json tagName,isDraft \
    --jq ".[] | select(.tagName == \"$tag\") | .isDraft")"
case "$state" in
    false)
        echo "[release] $tag is already published, so this run will not replace its assets." >&2
        echo "[release] To repair it, turn it back into a draft first: gh release edit $tag --draft" >&2
        exit 1
        ;;
    true)
        echo "[release] reusing the existing draft for $tag"
        ;;
    *)
        gh release create "$tag" --repo "$repo" --draft --verify-tag --generate-notes \
            --title "Gnoblin $expected_version" --notes-file "$notes_file"
        ;;
esac

gh release upload "$tag" "${assets[@]}" --repo "$repo" --clobber
gh release edit "$tag" --repo "$repo" --draft=false
echo "[release] published $tag"

#!/usr/bin/env bash
# Run the repository tool on real packages and install from the result in a clean container.
#
# usage: tests/pkgrepo/real-packages.sh PROJECT CHANNEL FAMILY PACKAGES-DIR "PACKAGE ..."
#
#   PACKAGES-DIR   the input of `pkgrepo build`: fedora/<release>/*.rpm, deb/<suite>/*.deb or arch/*.pkg.tar.zst
#   FAMILY         fedora, debian or arch: the distribution the clean container runs
#   "PACKAGE ..."  the packages to install from the repository
#
# It uses a throw-away key in $PKGREPO_WORK/gnupg (created by containers.sh packages, or made here) and the same
# environment variables as containers.sh. Nothing leaves the machine.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
src="$(cd -- "$here/../.." && pwd)"
engine="${PKGREPO_ENGINE:-podman}"
work="${PKGREPO_WORK:-/var/tmp/pkgrepo-test}"
port="${PKGREPO_PORT:-18090}"
project="$1" channel="$2" family="$3" packages="$4" names="$5"
mkdir -p "$work"

image=docker.io/library/fedora:43
suite=trixie
[[ $family == debian ]] && image=docker.io/library/debian:trixie
[[ $family == arch ]] && image=docker.io/library/archlinux:latest

tool() {
    $engine run --rm --network=host -v "$src:/src:ro,Z" -v "$work:/work:Z" -v "$packages:/packages:ro,Z" -e GNUPGHOME=/work/gnupg \
        -e PYTHONDONTWRITEBYTECODE=1 -w /src/scripts localhost/pkgrepo-builder python3 -m pkgrepo "$@"
}
if [[ ! -d "$work/gnupg" ]]; then
    mkdir -m 0700 "$work/gnupg"
    $engine run --rm -v "$work:/work:Z" -e GNUPGHOME=/work/gnupg localhost/pkgrepo-builder gpg --batch --pinentry-mode loopback \
        --passphrase '' --quick-generate-key 'pkgrepo test key (throw-away) <pkgrepo-test@example.invalid>' rsa3072 sign never
fi
rm -rf "$work/real-tree" "$work/real-site"
mkdir -p "$work/real-site"
tool build --project "$project" --channel "$channel" --packages /packages --output /work/real-tree --gnupg-home /work/gnupg \
    --base-url "http://127.0.0.1:$port"
tool publish --project "$project" --channel "$channel" --tree /work/real-tree --backend rsync --destination /work/real-site --apply >/dev/null
(cd "$work/real-site" && exec python3 -m http.server "$port" --bind 127.0.0.1 >"$work/logs/http-real.log" 2>&1) &
server=$!
trap 'kill $server 2>/dev/null || true' EXIT
for _ in $(seq 50); do
    curl -fsS -o /dev/null "http://127.0.0.1:$port/" 2>/dev/null && break
    sleep 0.2
done
find "$work/real-site" -type f | sort | sed "s#$work/real-site/##"
$engine run --rm --network=host -v "$here/clients:/clients:ro,Z" -e FAMILY="$family" -e PROJECT="$project" -e CHANNEL="$channel" \
    -e URL="http://127.0.0.1:$port" -e SUITE="$suite" -e PACKAGES="$names" "$image" bash /clients/real.sh

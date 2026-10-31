#!/usr/bin/env bash
# End-to-end test of scripts/pkgrepo with the real tools, in containers, with throw-away packages and a throw-away key.
#
# What it does:
#   1. builds dummy RPM, deb and Arch packages (stable 0.1.12, stable 0.1.13, two nightlies)
#   2. builds the signed repositories with the tool and publishes them with the rsync back end into two site
#      directories: state 1 (stable 0.1.12, nightly of 0.1.13) and state 2 (adds stable 0.1.13 and a nightly of 0.1.14,
#      built by pulling state 1 over HTTP and merging)
#   3. serves both states and, in clean Fedora, Debian, Ubuntu and Arch containers, adds the repositories the way the
#      documentation says, installs, upgrades between channels and downgrades
#   4. checks that a tampered package and a tampered signature are refused
#   5. compares the version orderings of the tool with rpm, dpkg and pacman's vercmp
#
# usage: tests/pkgrepo/containers.sh [step ...]      steps: packages repos clients tamper versions (default: all)
#
# Environment:
#   PKGREPO_ENGINE   the container command, default podman (use `podman --root DIR --runroot DIR2` to keep images off /home)
#   PKGREPO_WORK     scratch directory, default /var/tmp/pkgrepo-test; the logs of the run are in $PKGREPO_WORK/logs
#   PKGREPO_PORT     first of two ports for the local servers, default 18081
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
src="$(cd -- "$here/../.." && pwd)"
engine="${PKGREPO_ENGINE:-podman}"
work="${PKGREPO_WORK:-/var/tmp/pkgrepo-test}"
port1="${PKGREPO_PORT:-18081}"
port2=$((port1 + 1))
port3=$((port1 + 2))
project=bingux
package=bingux-dummy
mkdir -p "$work/logs"

steps=("$@")
((${#steps[@]})) || steps=(packages repos clients tamper versions)
wanted() {
    local s
    for s in "${steps[@]}"; do [[ $s == "$1" ]] && return 0; done
    return 1
}
say() { printf '\n==== %s\n' "$*"; }

builder() {
    $engine run --rm --network=host -v "$src:/src:ro,Z" -v "$work:/work:Z" -e GNUPGHOME=/work/gnupg \
        -e PYTHONDONTWRITEBYTECODE=1 -w /src/scripts "$@"
}
tool() { builder localhost/pkgrepo-builder python3 -m pkgrepo "$@"; }

if ! $engine image exists localhost/pkgrepo-builder 2>/dev/null; then
    say "building the tool image"
    $engine build -q -t pkgrepo-builder -f "$src/scripts/pkgrepo/container/Containerfile" "$src/scripts/pkgrepo/container"
fi

# --- 1. packages -------------------------------------------------------------------------------------------------------
if wanted packages; then
    say "throw-away signing key"
    if [[ ! -d "$work/gnupg" ]]; then
        mkdir -m 0700 "$work/gnupg"
        builder localhost/pkgrepo-builder gpg --batch --pinentry-mode loopback --passphrase '' \
            --quick-generate-key 'pkgrepo test key (throw-away) <pkgrepo-test@example.invalid>' rsa3072 sign never 2>&1 | tail -2
    fi

    say "nightly version strings from the tool"
    v1="$(tool version --last-stable 0.1.12 --tree-version 0.1.12 --count 142 --commit abc1234 --date 20261031 --format json)"
    v2="$(tool version --last-stable 0.1.13 --tree-version 0.1.13 --count 20 --commit def5678 --date 20261105 --format json)"
    field() { python3 -c 'import json,sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"; }
    echo "nightly 1: rpm/deb $(field "$v1" rpm), arch $(field "$v1" arch)"
    echo "nightly 2: rpm/deb $(field "$v2" rpm), arch $(field "$v2" arch)"

    $engine unshare rm -rf "$work/pkgs"
    mkdir -p "$work/pkgs"
    cat >"$work/make-rpm-deb.sh" <<EOF
set -euo pipefail
cd /src/tests/pkgrepo
mk() { # label rpm-version deb-version release
    local out=/work/pkgs/\$1
    ./make-dummy-package.py rpm $package \$2 \$4 \$out/fedora/43 --dist .fc43 2>/dev/null
    ./make-dummy-package.py rpm $package-data \$2 \$4 \$out/fedora/43 --dist .fc43 --arch noarch 2>/dev/null
    for suite in trixie noble; do
        ./make-dummy-package.py deb $package \$3 \$4 \$out/deb/\$suite
        ./make-dummy-package.py deb $package-data \$3 \$4 \$out/deb/\$suite --arch all
    done
}
mk stable-1 0.1.12 0.1.12 1
mk stable-2 0.1.13 0.1.13 1
mk nightly-1 $(field "$v1" rpm) $(field "$v1" deb) 1
mk nightly-2 $(field "$v2" rpm) $(field "$v2" deb) 1
EOF
    builder localhost/pkgrepo-builder bash /work/make-rpm-deb.sh
    cat >"$work/make-arch.sh" <<EOF
set -euo pipefail
pacman -Sy --noconfirm --needed base-devel >/dev/null 2>&1
useradd -m builder
cp /src/tests/pkgrepo/make-dummy-package.py /home/builder/
chmod -R a+rwX /work/pkgs
mk() { # label pkgver
    su builder -c "python3 /home/builder/make-dummy-package.py arch $package \$2 1 /work/pkgs/\$1/arch"
    su builder -c "python3 /home/builder/make-dummy-package.py arch $package-data \$2 1 /work/pkgs/\$1/arch --arch any"
}
pacman -S --noconfirm --needed python >/dev/null 2>&1
mk stable-1 0.1.12
mk stable-2 0.1.13
mk nightly-1 $(field "$v1" arch)
mk nightly-2 $(field "$v2" arch)
chown -R 0:0 /work/pkgs
EOF
    $engine run --rm --network=host -v "$src:/src:ro,Z" -v "$work:/work:Z" docker.io/library/archlinux:latest bash /work/make-arch.sh
    find "$work/pkgs" -type f | sort | sed "s#$work/##"
fi

# --- 2. repositories -------------------------------------------------------------------------------------------------
site() { echo "/work/site$1"; }
build_channel() { # state channel label [extra args]
    local state=$1 channel=$2 label=$3
    shift 3
    tool build --project $project --channel "$channel" --packages "/work/pkgs/$label" --output "/work/trees/$state-$channel" \
        --existing "/work/existing/$state-$channel" --gnupg-home /work/gnupg --base-url "http://127.0.0.1:$PORT" \
        --source-commit "$label" "$@"
}

serve() { # directory port
    (cd "$1" && exec python3 -m http.server "$2" --bind 127.0.0.1 >"$work/logs/http-$2.log" 2>&1) &
    server_pids+=($!)
    for _ in $(seq 50); do
        curl -fsS -o /dev/null "http://127.0.0.1:$2/" 2>/dev/null && return 0
        sleep 0.2
    done
    echo "server on port $2 did not start" >&2
    return 1
}
server_pids=()
stop_servers() { ((${#server_pids[@]})) && kill "${server_pids[@]}" 2>/dev/null || true; }
trap stop_servers EXIT

if wanted repos; then
    say "state 1: stable 0.1.12 and nightly 1, published with the rsync back end into site1/"
    rm -rf "$work/trees" "$work/existing" "$work/site1" "$work/site2" "$work/site3"
    mkdir -p "$work/trees" "$work/existing" "$work/site1" "$work/site2"
    PORT=$port1
    build_channel s1 stable stable-1
    build_channel s1 nightly nightly-1
    for channel in stable nightly; do
        tool publish --project $project --channel $channel --tree "/work/trees/s1-$channel" --backend rsync \
            --destination /work/site1 | tail -12
        tool publish --project $project --channel $channel --tree "/work/trees/s1-$channel" --backend rsync \
            --destination /work/site1 --apply >/dev/null
    done
    find "$work/site1" -type f | sort | sed "s#$work/##"

    say "state 2: pull state 1 over HTTP, add stable 0.1.13 and nightly 2, publish into site2/"
    serve "$work/site1" "$port1"
    PORT=$port2
    for channel in stable nightly; do
        tool pull --project $project --channel $channel --url "http://127.0.0.1:$port1" --output "/work/existing/s2-$channel"
    done
    build_channel s2 stable stable-2
    build_channel s2 nightly nightly-2
    for channel in stable nightly; do
        tool publish --project $project --channel $channel --tree "/work/trees/s2-$channel" --backend rsync \
            --destination /work/site2 --previous "/work/existing/s2-$channel" --apply | tail -8
    done
    # The channels of state 2 already hold the packages of state 1 (merged); site2 also needs the root files.
    echo "--- stable manifest of state 2"
    python3 -c 'import json,sys; [print(p["family"], p["release"], p["name"], p["version"]) for p in json.load(open(sys.argv[1]))["packages"]]' "$work/site2/$project/stable/manifest.json"
    echo "--- nightly manifest of state 2 (retention keeps the newest 14 per package; here both nightlies fit)"
    python3 -c 'import json,sys; [print(p["family"], p["release"], p["name"], p["version"]) for p in json.load(open(sys.argv[1]))["packages"]]' "$work/site2/$project/nightly/manifest.json"
    stop_servers
    server_pids=()
fi

# --- 3. clients --------------------------------------------------------------------------------------------------------
client() { # name image script [extra env]
    local name=$1 image=$2 script=$3
    shift 3
    say "client: $name"
    $engine run --rm --network=host -v "$here/clients:/clients:ro,Z" -e PROJECT=$project -e PACKAGE=$package \
        -e "URL1=http://127.0.0.1:$port1" -e "URL2=http://127.0.0.1:$port2" "$@" "$image" bash "/clients/$script" \
        2>&1 | tee "$work/logs/client-$name.log" | sed 's/^/  /'
    grep -q "CLIENT-OK" "$work/logs/client-$name.log"
}

if wanted clients; then
    serve "$work/site1" "$port1"
    serve "$work/site2" "$port2"
    client fedora-43 docker.io/library/fedora:43 fedora.sh
    client debian-trixie docker.io/library/debian:trixie debian.sh -e SUITE=trixie
    client ubuntu-noble docker.io/library/ubuntu:24.04 debian.sh -e SUITE=noble
    client archlinux docker.io/library/archlinux:latest arch.sh
    stop_servers
    server_pids=()
fi

# --- 4. tampering ------------------------------------------------------------------------------------------------------
tamper_case() { # name port
    local name=$1 url="http://127.0.0.1:$2" family
    for family in fedora debian arch; do
        local image=docker.io/library/$family
        [[ $family == arch ]] && image=docker.io/library/archlinux:latest
        [[ $family == debian ]] && image=docker.io/library/debian:trixie
        [[ $family == fedora ]] && image=docker.io/library/fedora:43
        $engine run --rm --network=host -v "$here/clients:/clients:ro,Z" -e FAMILY=$family -e URL="$url" -e PROJECT=$project \
            -e PACKAGE=$package "$image" bash /clients/tamper.sh 2>&1 | sed "s/^/  [$name] /" | tee -a "$work/logs/tamper.log"
        [[ ${PIPESTATUS[0]} -eq 0 ]] || {
            echo "tamper case $name failed for $family" >&2
            return 1
        }
    done
}

if wanted tamper; then
    : >"$work/logs/tamper.log"
    say "tampered copies of state 2 must be refused: one flipped byte in every package (port $port3)"
    rm -rf "$work/site3" "$work/site4"
    cp -r "$work/site2" "$work/site3"
    cp -r "$work/site2" "$work/site4"
    # The client files name the port of state 2; the copies are served on other ports.
    find "$work/site3" \( -name '*.repo' -o -name '*.sources' -o -name '*.conf' \) -exec sed -i "s/:$port2/:$port3/" {} +
    find "$work/site4" \( -name '*.repo' -o -name '*.sources' -o -name '*.conf' \) -exec sed -i "s/:$port2/:$((port3 + 1))/" {} +
    find "$work/site3" \( -name '*.rpm' -o -name '*.deb' -o -name '*.pkg.tar.zst' \) -exec sh -c 'printf X >> "$1"' _ {} \;
    serve "$work/site3" "$port3"
    tamper_case packages "$port3"
    say "tampered metadata: a changed line in repomd.xml and Release, a flipped byte in the pacman database (port $((port3 + 1)))"
    find "$work/site4" -name repomd.xml -exec sh -c 'echo "<!-- tampered -->" >> "$1"' _ {} \;
    find "$work/site4" -name Release -exec sh -c 'echo "X-Tampered: yes" >> "$1"' _ {} \;
    find "$work/site4" -name InRelease -exec sh -c 'sed -i "s/^Origin: .*/Origin: Evil/" "$1"' _ {} \;
    find "$work/site4" \( -name '*.db' -o -name '*.db.tar.zst' \) -exec sh -c 'printf X >> "$1"' _ {} \;
    serve "$work/site4" "$((port3 + 1))"
    tamper_case metadata "$((port3 + 1))"
    stop_servers
    server_pids=()
fi

# --- 5. version orderings --------------------------------------------------------------------------------------------
if wanted versions; then
    say "version orderings against rpm, dpkg and vercmp"
    $engine run --rm -v "$src:/src:ro,Z" -e PYTHONDONTWRITEBYTECODE=1 localhost/pkgrepo-builder python3 /src/tests/pkgrepo/compare-versions.py rpm dpkg \
        2>&1 | tee "$work/logs/versions-rpm-dpkg.log"
    $engine run --rm -v "$src:/src:ro,Z" -e PYTHONDONTWRITEBYTECODE=1 docker.io/library/archlinux:latest bash -c \
        'pacman -Sy --noconfirm --needed python >/dev/null 2>&1; python3 /src/tests/pkgrepo/compare-versions.py pacman' \
        2>&1 | tee "$work/logs/versions-pacman.log"
fi

say "all selected steps passed"

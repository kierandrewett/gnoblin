#!/usr/bin/env bash
# Run the graphical VM test for one release: boot its cloud image in QEMU, install Gnoblin, start a real session on
# the virtio GPU and check that it renders and takes input. See design/graphical-testing.md.
#
# usage: scripts/run-graphical-test.sh RELEASE [PACKAGES] [harness options]
#
#   RELEASE   a graphical target of scripts/supported-targets.py: "Fedora 42", "Debian 13", "Arch Linux", or a slug
#             such as fc42 or deb13. The list comes from endoflife.date at run time.
#   PACKAGES  a folder of package files (the build artifacts), or nightly or stable for that channel of
#             repo.gnoblin.org. Default: nightly.
#
# The evidence goes to $GNOBLIN_GRAPHICAL_OUT, default build/graphical/<release>. The cloud image is cached in
# $TMPDIR/gnoblin-graphical-images and deleted after the run unless --keep-image is given. Other options go to
# tests/graphical/harness.py (--accel tcg, --break CHECK, --min-free-gb N, ...; see its --help).
#
# It needs qemu-system-x86_64, qemu-img, ssh, ssh-keygen, tar and genisoimage (or mkisofs or xorriso), and /dev/kvm
# for a run of minutes rather than an hour.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if (($# < 1)) || [[ "$1" == -h || "$1" == --help ]]; then
    sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
    exit 2
fi
release="$1"
shift
packages=nightly
if (($#)) && [[ "$1" != -* ]]; then
    packages="$1"
    shift
fi
for tool in qemu-system-x86_64 qemu-img ssh ssh-keygen tar; do
    command -v "$tool" >/dev/null || {
        echo "run-graphical-test: $tool is needed" >&2
        exit 2
    }
done
out="${GNOBLIN_GRAPHICAL_OUT:-$root/build/graphical/${release// /-}}"
exec python3 "$root/tests/graphical/harness.py" --release "$release" --packages "$packages" --out "$out" "$@"

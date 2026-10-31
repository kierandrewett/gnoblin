#!/bin/bash
# Runs inside a clean Arch container.
set -euo pipefail
say() { printf '\n### %s\n' "$*"; }
installed() { pacman -Q "$PACKAGE"; }
point() { sed -i "s#$1#$2#" /etc/pacman.conf; }
setup_keys() {
    pacman-key --init >/dev/null 2>&1
    curl -fsS -o /tmp/gpg.key "$URL1/gpg.key"
    pacman-key --add /tmp/gpg.key >/dev/null
    fingerprint="$(gpg --show-keys --with-colons /tmp/gpg.key | awk -F: '/^fpr/ {print $10; exit}')"
    pacman-key --lsign-key "$fingerprint" >/dev/null 2>&1
    echo "trusted key $fingerprint"
}
section() { # channel, base URL
    curl -fsS "$2/$PROJECT/$1/arch/$([ $1 = stable ] && echo $PROJECT || echo $PROJECT-$1).conf" | tee -a /etc/pacman.conf
}
unsection() {
    python3 - "$1" <<'PY' || true
import re, sys
text = open("/etc/pacman.conf").read()
text = re.sub(r"\n\[%s\]\nSigLevel = [^\n]*\nServer = [^\n]*\n" % re.escape(sys.argv[1]), "\n", text)
open("/etc/pacman.conf", "w").write(text)
PY
}
pacman -Sy --noconfirm --needed curl python >/dev/null 2>&1 || true

say "Channel stable: trust the key, add the section from the generated snippet"
setup_keys
echo >>/etc/pacman.conf
section stable "$URL1"
pacman -Sy --noconfirm $PACKAGE 2>&1 | tail -12
installed
$PACKAGE
say "Stable receives 0.1.13: upgrade"
point "$URL1" "$URL2"
pacman -Sy --noconfirm $PACKAGE 2>&1 | tail -6
installed

say "Channel nightly only, clean system"
pacman -R --noconfirm $PACKAGE >/dev/null
unsection $PROJECT
echo >>/etc/pacman.conf
section nightly "$URL1"
pacman -Sy --noconfirm $PACKAGE 2>&1 | tail -6
installed
say "Nightly -> stable: the nightly of 0.1.13 gives way to the stable 0.1.13"
unsection $PROJECT-nightly
echo >>/etc/pacman.conf
section stable "$URL2"
pacman -Sy --noconfirm $PACKAGE 2>&1 | tail -6
installed
say "Stable -> nightly: nightly section placed first wins; both sections listed"
unsection $PROJECT
echo >>/etc/pacman.conf
section nightly "$URL2"
section stable "$URL2"
pacman -Sy >/dev/null 2>&1
echo "nightly section first:"
pacman -Si $PACKAGE | grep -E '^(Repository|Version)' | head -2
unsection $PROJECT-nightly
unsection $PROJECT
echo >>/etc/pacman.conf
section stable "$URL2" >/dev/null
section nightly "$URL2" >/dev/null
pacman -Sy >/dev/null 2>&1
echo "stable section first (pacman takes the first repository that has the package, not the highest version):"
pacman -Si $PACKAGE | grep -E '^(Repository|Version)' | head -2
unsection $PROJECT-nightly
unsection $PROJECT
echo >>/etc/pacman.conf
section nightly "$URL2" >/dev/null
section stable "$URL2" >/dev/null
pacman -Sy >/dev/null 2>&1
pacman -Sy --noconfirm $PACKAGE 2>&1 | tail -6
installed
say "Back to stable: delete the nightly section, then reinstall from the stable section (pacman does not downgrade by itself)"
unsection $PROJECT-nightly
pacman -Sy >/dev/null 2>&1
pacman -S --noconfirm $PACKAGE 2>&1 | tail -4
installed
echo ARCH-CLIENT-OK

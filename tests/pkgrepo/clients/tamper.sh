#!/bin/bash
# Runs inside a clean container of the distribution named by FAMILY (fedora, debian or arch). The server at $URL has
# been tampered with; installing PACKAGE must fail, and the failure must be about the content, not about the network.
set -uo pipefail
out=/tmp/tamper.out
case "$FAMILY" in
    fedora)
        curl -fsS -o /etc/yum.repos.d/r.repo "$URL/$PROJECT/stable/fedora/$PROJECT-stable.repo" || exit 3
        dnf -y --repo $PROJECT-stable install $PACKAGE >$out 2>&1
        ;;
    debian)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null
        install -d /etc/apt/keyrings
        curl -fsS -o /etc/apt/keyrings/$PROJECT.asc "$URL/gpg.key" || exit 3
        curl -fsS -o /etc/apt/sources.list.d/r.sources "$URL/$PROJECT/stable/deb/$PROJECT-stable-trixie.sources" || exit 3
        { apt-get update && apt-get install -y $PACKAGE; } >$out 2>&1
        ;;
    arch)
        pacman -Sy --noconfirm --needed curl >/dev/null 2>&1
        pacman-key --init >/dev/null 2>&1
        curl -fsS -o /tmp/k "$URL/gpg.key" || exit 3
        pacman-key --add /tmp/k >/dev/null 2>&1
        fp="$(gpg --show-keys --with-colons /tmp/k 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')"
        pacman-key --lsign-key "$fp" >/dev/null 2>&1
        printf '\n' >>/etc/pacman.conf
        curl -fsS "$URL/$PROJECT/stable/arch/$PROJECT.conf" >>/etc/pacman.conf || exit 3
        pacman -Sy --noconfirm $PACKAGE >$out 2>&1
        ;;
esac
status=$?
if [ $status -eq 0 ]; then
    echo "$FAMILY: INSTALLED FROM A TAMPERED REPOSITORY"
    tail -5 $out
    exit 1
fi
if grep -qiE "could not connect|curl error \(7\)|failed to connect|Connection refused" $out; then
    echo "$FAMILY: failed for a network reason, so the test proves nothing"
    tail -5 $out
    exit 2
fi
echo "$FAMILY: refused (exit status $status). The errors:"
grep -iE "error|fail|checksum|signature|mismatch|corrupt|invalid|^E:|NO_PUBKEY|BAD" $out | grep -v debconf | cut -c1-200 | sort -u | head -6
exit 0

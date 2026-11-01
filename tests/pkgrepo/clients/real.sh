#!/bin/bash
# Runs inside a clean container of the distribution named by FAMILY (fedora, debian, arch or alpine): adds the repository at
# $URL (channel $CHANNEL of project $PROJECT), installs the packages named in $PACKAGES, and verifies their files.
set -euo pipefail
say() { printf '\n### %s\n' "$*"; }
# shellcheck disable=SC2086
case "$FAMILY" in
    fedora)
        say "add the repository file and install"
        curl -fsS -o "/etc/yum.repos.d/$PROJECT-$CHANNEL.repo" "$URL/$PROJECT/$CHANNEL/fedora/$PROJECT-$CHANNEL.repo"
        dnf -y --repo "$PROJECT-$CHANNEL" --repo fedora --repo updates install $PACKAGES 2>&1 | tail -25
        say "verify"
        rpm -q $PACKAGES
        rpm -V $PACKAGES && echo "rpm -V: all files match"
        rpm -qa 'gpg-pubkey*' --queryformat '%{NAME}  %{SUMMARY}\n'
        ;;
    debian)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq >/dev/null
        apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null
        install -d -m 0755 /etc/apt/keyrings
        say "add the key and the sources file, install"
        curl -fsS -o "/etc/apt/keyrings/$PROJECT.asc" "$URL/gpg.key"
        curl -fsS -o "/etc/apt/sources.list.d/$PROJECT.sources" "$URL/$PROJECT/$CHANNEL/deb/$PROJECT-$CHANNEL-$SUITE.sources"
        apt-get update 2>&1 | grep -E "127.0.0.1|^E:|^W:" || true
        apt-get install -y $PACKAGES 2>&1 | tail -15
        say "verify"
        dpkg-query -W -f '${Package} ${Version}\n' $PACKAGES
        dpkg --verify $PACKAGES && echo "dpkg --verify: all files match"
        ;;
    arch)
        pacman -Sy --noconfirm --needed curl >/dev/null 2>&1
        say "trust the key, add the section, install"
        pacman-key --init >/dev/null 2>&1
        curl -fsS -o /tmp/key "$URL/gpg.key"
        pacman-key --add /tmp/key >/dev/null 2>&1
        fingerprint="$(gpg --show-keys --with-colons /tmp/key 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')"
        pacman-key --lsign-key "$fingerprint" >/dev/null 2>&1
        name="$PROJECT"
        [ "$CHANNEL" = stable ] || name="$PROJECT-$CHANNEL"
        printf '\n' >>/etc/pacman.conf
        curl -fsS "$URL/$PROJECT/$CHANNEL/arch/$name.conf" | tee -a /etc/pacman.conf
        pacman -Sy --noconfirm $PACKAGES 2>&1 | tail -20
        say "verify"
        pacman -Q $PACKAGES
        pacman -Qkq $PACKAGES && echo "pacman -Qk: all files match"
        ;;
    alpine)
        say "trust the key, add the repository, install"
        wget -q -O "/etc/apk/keys/$PROJECT.rsa.pub" "$URL/$PROJECT.rsa.pub"
        wget -q -O - "$URL/$PROJECT/$CHANNEL/alpine/edge/$PROJECT-$CHANNEL.list" | tee -a /etc/apk/repositories
        apk update
        apk add $PACKAGES 2>&1 | tail -20
        say "verify"
        apk list -I $PACKAGES
        for name in $PACKAGES; do
            apk info -L "$name" | tail -n +2 | while read -r file; do
                [ -n "$file" ] || continue
                test -f "/$file" || {
                    echo "missing /$file"
                    exit 1
                }
            done
        done
        echo "every file the packages list is installed"
        ;;
esac
echo REAL-CLIENT-OK

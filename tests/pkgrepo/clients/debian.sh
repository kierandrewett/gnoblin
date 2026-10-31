#!/bin/bash
# Runs inside a clean Debian or Ubuntu container; SUITE is the codename (trixie, noble).
set -euo pipefail
say() { printf '\n### %s\n' "$*"; }
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null
install -d -m 0755 /etc/apt/keyrings
installed() { dpkg-query -W -f '${Package} ${Version}\n' "$PACKAGE"; }
point() { sed -i "s#$1#$2#" /etc/apt/sources.list.d/*.sources; }

say "Channel stable: key and sources file as documented"
curl -fsS -o /etc/apt/keyrings/$PROJECT.asc "$URL1/gpg.key"
curl -fsS -o /etc/apt/sources.list.d/$PROJECT.sources "$URL1/$PROJECT/stable/deb/$PROJECT-stable-$SUITE.sources"
cat /etc/apt/sources.list.d/$PROJECT.sources
apt-get update 2>&1 | grep -i "$PROJECT\|127.0.0.1\|err\|warn" || true
apt-get install -y -qq $PACKAGE >/dev/null
installed
$PACKAGE
say "Stable receives 0.1.13: upgrade"
point "$URL1" "$URL2"
apt-get update 2>&1 | grep -i "err\|warn\|Get:" || true
apt-get install -y -qq --only-upgrade $PACKAGE 2>&1 | tail -3
installed

say "Channel nightly only"
apt-get remove -y -qq $PACKAGE >/dev/null
rm -f /etc/apt/sources.list.d/$PROJECT.sources
curl -fsS -o /etc/apt/sources.list.d/$PROJECT.sources "$URL1/$PROJECT/nightly/deb/$PROJECT-nightly-$SUITE.sources"
apt-get update -qq 2>&1 | grep -i "err\|warn" || true
apt-get install -y -qq $PACKAGE >/dev/null
installed
say "Nightly -> stable: the stable 0.1.13 replaces the nightly 0.1.13~nightly"
rm -f /etc/apt/sources.list.d/$PROJECT.sources
curl -fsS -o /etc/apt/sources.list.d/$PROJECT.sources "$URL2/$PROJECT/stable/deb/$PROJECT-stable-$SUITE.sources"
apt-get update -qq 2>&1 | grep -i "err\|warn" || true
apt-get install -y -qq --only-upgrade $PACKAGE 2>&1 | tail -3
installed
say "Stable -> nightly: both channels enabled, the nightly of the next version wins"
curl -fsS -o /etc/apt/sources.list.d/$PROJECT-nightly.sources "$URL2/$PROJECT/nightly/deb/$PROJECT-nightly-$SUITE.sources"
apt-get update -qq 2>&1 | grep -i "err\|warn" || true
apt-cache policy $PACKAGE | sed -n '1,12p'
apt-get install -y -qq --only-upgrade $PACKAGE 2>&1 | tail -3
installed
say "Back to stable: remove the nightly sources and downgrade"
rm -f /etc/apt/sources.list.d/$PROJECT-nightly.sources
apt-get update -qq 2>&1 | grep -i "err\|warn" || true
apt-get install -y -qq --allow-downgrades $PACKAGE=0.1.13-1 2>&1 | tail -3
installed
echo DEBIAN-CLIENT-OK

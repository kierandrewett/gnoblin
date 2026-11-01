#!/bin/sh
# Runs inside a clean Alpine edge container. apk checks the signature of the index against /etc/apk/keys, and the
# packages against the index, so the key is the only thing to trust.
set -eu
say() { printf '\n### %s\n' "$*"; }
installed() { apk list -I "$1" 2>/dev/null | cut -d' ' -f1; }
repo() { # base URL, channel: the line the repository publishes for /etc/apk/repositories
    wget -q -O - "$1/$PROJECT/$2/alpine/edge/$PROJECT-$2.list"
}
reset_repositories() { sed -i "\#/$PROJECT/#d" /etc/apk/repositories; }

say "Trust the key, add the stable channel as documented"
wget -q -O "/etc/apk/keys/$PROJECT.rsa.pub" "$URL1/$PROJECT.rsa.pub"
repo "$URL1" stable | tee -a /etc/apk/repositories
apk update | grep -i "$PROJECT\|error"
apk add -q "$PACKAGE" "$PACKAGE-data"
installed "$PACKAGE"
installed "$PACKAGE-data"
"$PACKAGE"

say "Stable receives 0.1.13: upgrade"
reset_repositories
repo "$URL2" stable >>/etc/apk/repositories
apk update | grep -i "error" || true
apk upgrade -q
installed "$PACKAGE"

say "Channel nightly only, clean system"
apk del -q "$PACKAGE" "$PACKAGE-data"
reset_repositories
repo "$URL1" nightly >>/etc/apk/repositories
apk update | grep -i "error" || true
apk add -q "$PACKAGE"
installed "$PACKAGE"
"$PACKAGE"

say "Nightly -> stable: the stable 0.1.13 is newer than the nightly 0.1.13_pre"
repo "$URL2" stable >>/etc/apk/repositories
apk update | grep -i "error" || true
apk upgrade -q
installed "$PACKAGE"

say "Stable -> nightly: both channels enabled, the nightly of the next version is newest"
reset_repositories
repo "$URL2" stable >>/etc/apk/repositories
repo "$URL2" nightly >>/etc/apk/repositories
apk update | grep -i "error" || true
apk upgrade -q
installed "$PACKAGE"
"$PACKAGE"

say "Back to stable: apk does not downgrade by itself; ask for the stable version"
reset_repositories
repo "$URL2" stable >>/etc/apk/repositories
apk update | grep -i "error" || true
apk add -q "$PACKAGE=0.1.13-r1"
installed "$PACKAGE"
echo ALPINE-CLIENT-OK

#!/bin/bash
# Runs inside a clean Fedora container. The repositories are served at $URL1 (state 1) and $URL2 (state 2: a stable
# release and a newer nightly were published). PROJECT=bingux, PACKAGE=bingux-dummy.
set -euo pipefail
say() { printf '\n### %s\n' "$*"; }
repo() { dnf repoquery --repo "$1" --queryformat '%{name} %{version}-%{release}\n' "$PACKAGE" 2>/dev/null | sort -u; }
installed() { rpm -q --qf '%{name}-%{version}-%{release}\n' "$PACKAGE"; }
point() { sed -i "s#$1#$2#" /etc/yum.repos.d/*.repo; }

say "Channel stable: add the repository file the tool generated"
curl -fsS -o /etc/yum.repos.d/$PROJECT-stable.repo "$URL1/$PROJECT/stable/fedora/$PROJECT-stable.repo"
cat /etc/yum.repos.d/$PROJECT-stable.repo
say "Install from stable (dnf imports the key and checks the repository and package signatures)"
dnf -y --repo $PROJECT-stable install $PACKAGE 2>&1 | tail -25
installed
$PACKAGE
say "Stable receives 0.1.13: upgrade"
point "$URL1" "$URL2"
dnf -y --repo $PROJECT-stable upgrade 2>&1 | tail -8
installed
$PACKAGE
rpm -K /var/cache/libdnf5/*/packages/*.rpm 2>/dev/null | head -3 || true
rpm -qa 'gpg-pubkey*' --qf '%{name}-%{version}-%{release} %{summary}\n'

say "Channel nightly only, clean system"
dnf -y remove $PACKAGE >/dev/null 2>&1
rm -f /etc/yum.repos.d/$PROJECT-stable.repo
curl -fsS -o /etc/yum.repos.d/$PROJECT-nightly.repo "$URL1/$PROJECT/nightly/fedora/$PROJECT-nightly.repo"
dnf -y --repo $PROJECT-nightly install $PACKAGE 2>&1 | tail -6
installed
say "Nightly -> stable: the nightly of 0.1.13 gives way to the stable 0.1.13 (stable repository only)"
rm -f /etc/yum.repos.d/$PROJECT-nightly.repo
curl -fsS -o /etc/yum.repos.d/$PROJECT-stable.repo "$URL2/$PROJECT/stable/fedora/$PROJECT-stable.repo"
dnf -y --repo $PROJECT-stable upgrade 2>&1 | tail -6
installed
say "Stable -> nightly: enabling the nightly channel moves to 0.1.14~nightly"
curl -fsS -o /etc/yum.repos.d/$PROJECT-nightly.repo "$URL2/$PROJECT/nightly/fedora/$PROJECT-nightly.repo"
dnf -y --repo $PROJECT-nightly upgrade 2>&1 | tail -6
installed
say "Both channels enabled: the highest version wins"
dnf -y distro-sync --repo $PROJECT-stable --repo $PROJECT-nightly $PACKAGE 2>&1 | tail -3 || true
dnf repoquery --repo $PROJECT-stable --repo $PROJECT-nightly --queryformat '%{repoid} %{name}-%{version}-%{release}\n' $PACKAGE | sort
say "Back to stable: remove the nightly repository and downgrade"
rm -f /etc/yum.repos.d/$PROJECT-nightly.repo
dnf -y --repo $PROJECT-stable distro-sync $PACKAGE 2>&1 | tail -6
installed
say "Architecture-independent package from the same repository"
dnf -y --repo $PROJECT-stable install $PACKAGE-data 2>&1 | tail -3
rpm -q $PACKAGE-data
echo FEDORA-CLIENT-OK

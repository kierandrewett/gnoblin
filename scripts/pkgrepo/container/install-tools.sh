#!/bin/sh
# Install every program scripts/pkgrepo starts, on Debian 13 (trixie). The Containerfile and the CI workflows both run
# this script, so the list exists once.
#   createrepo-c            RPM repository metadata
#   rpm                     rpmsign (package signatures), rpmkeys and rpm -qp (package metadata)
#   apt-utils               apt-ftparchive, APT repository metadata
#   dpkg-dev                dpkg-deb, Debian package metadata
#   pacman-package-manager  repo-add, pacman database; makepkg holds the helper scripts repo-add sources
#   libarchive-tools        bsdtar, which repo-add reads packages with
#   gnupg, zstd, xz-utils   signing and the compression of pacman packages and repository metadata
#   openssl                 the RSA signature of the Alpine index (apk does not use GnuPG)
#   rsync, awscli, git      the three publish back ends (rsync, s3, pages)
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    apt-utils awscli ca-certificates createrepo-c curl dpkg-dev file git gnupg libarchive-tools makepkg openssh-client \
    openssl pacman-package-manager python3 rpm rsync tar xz-utils zstd
rm -rf /var/lib/apt/lists/*

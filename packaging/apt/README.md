# Gnoblin APT archive

The public archive lives under `https://kierandrewett.github.io/gnoblin/apt/`.
It has one suite for Debian 13 and two suites for Ubuntu 24.04 and 26.04. Each
suite contains only packages built and tested for that system.

`scripts/build-apt-repository.py` copies a release's three `.deb` files into
versioned pool paths, regenerates `Packages` and `Packages.gz`, writes a
`Release` file, then signs `InRelease` and `Release.gpg`. It refuses to replace
an existing versioned package with different content.

The archive key is published as `gnoblin-archive-keyring.asc`. Its fingerprint
is `4014 C902 BAAC EF89 AD16 9CE9 8BB8 7CE7 4060 2F47` and it expires on
19 September 2029. The private key and passphrase are GitHub Actions secrets:
`GNOBLIN_APT_SIGNING_KEY` and `GNOBLIN_APT_SIGNING_PASSPHRASE`.

The APT repository workflow runs after every GitHub release. It can also be
dispatched manually for a published release tag. Documentation deployment
preserves `apt/` on `gh-pages`; archive publishing preserves the documentation.

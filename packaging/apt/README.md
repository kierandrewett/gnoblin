# Gnoblin APT archive

The public archive lives under `https://gnoblin.org/apt/`.
The release pipeline is configured to publish suites for Debian 11, 12 and 13
and Ubuntu 22.04, 24.04 and 26.04. A suite is published only after its package
passes the build, installation, coexistence and removal jobs. Debian 11 builds
use the final Debian package snapshot from August 31, 2026.

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

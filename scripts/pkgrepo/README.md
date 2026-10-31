# pkgrepo

Builds, signs and uploads the package repositories of Gnoblin and Bingux. One
tool serves both projects. The repository layout, the channels, the version
rules and the key handling are in
[design/package-repositories.md](../../design/package-repositories.md). Users
install from the repositories as described in
[docs/package-repositories.md](../../docs/package-repositories.md).

The tool starts `createrepo_c`, `rpmsign`, `apt-ftparchive`, `repo-add` and
`gpg`. `container/install-tools.sh` installs all of them on Debian 13, and
`container/Containerfile` is the image that CI and a laptop both use:

```sh
podman build -t pkgrepo-builder -f scripts/pkgrepo/container/Containerfile scripts/pkgrepo/container
alias pkgrepo='podman run --rm --network=host -v "$PWD:/work:Z" -v "$PWD/scripts:/scripts:ro,Z" \
  -e GNUPGHOME=/work/.gnupg -e PYTHONPATH=/scripts -w /work pkgrepo-builder python3 -m pkgrepo'
```

## Commands

| Command       | What it does                                                                                          |
| ------------- | ----------------------------------------------------------------------------------------------------- |
| `version`     | Print the nightly version of a commit in every package format (`--git .` reads the checkout)          |
| `key-import`  | Import the private key from `REPO_GPG_PRIVATE_KEY` into a GNUPGHOME                                   |
| `needs-build` | Print `needs_build=true` when the published channel was built from another commit                     |
| `pull`        | Download the published manifest and packages of a channel, to merge a build with them                 |
| `build`       | Merge new packages with the pulled ones, apply retention, write and sign the metadata, then verify it |
| `verify`      | Check the checksums and signatures of a built tree the way a client would                             |
| `publish`     | Upload a built tree with the `rsync`, `s3` or `pages` back end. A dry run unless `--apply` is given   |

`build` expects the new packages as `PACKAGES/fedora/<release>/*.rpm`,
`PACKAGES/opensuse/<release>/*.rpm`, `PACKAGES/deb/<suite>/*.deb` and
`PACKAGES/arch/*.pkg.tar.zst`. Debug packages and source packages are skipped.

A local rehearsal with a throw-away key and the rsync back end writing to a
directory:

```sh
export GNUPGHOME=/work/.gnupg
gpg --batch --passphrase '' --quick-generate-key 'test key <test@example.invalid>' rsa3072 sign never
pkgrepo build --project gnoblin --channel nightly --packages packages --output tree --gnupg-home "$GNUPGHOME"
pkgrepo publish --project gnoblin --channel nightly --tree tree --backend rsync --destination site       # prints the plan
pkgrepo publish --project gnoblin --channel nightly --tree tree --backend rsync --destination site --apply
```

## Tests

```sh
python3 tests/pkgrepo.test.py            # rules, plans, rsync and Git back ends; no repository tools needed
tests/pkgrepo/containers.sh              # real tools in containers: builds, installs, upgrades, tampering, version orderings
```

`containers.sh` builds throw-away packages and a throw-away key, serves the
repositories on `127.0.0.1` and installs from them in clean Fedora, Debian,
Ubuntu and Arch containers. `PKGREPO_ENGINE` selects the container command (for
example `podman --root /var/tmp/containers --runroot /var/tmp/run`).

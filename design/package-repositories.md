# Package repositories

This is internal release engineering material. It describes the signed RPM, APT
and pacman repositories that serve both Gnoblin and Bingux, the channels they
have, and how the CI builds and publishes them. Users read
[`docs/package-repositories.md`](../docs/package-repositories.md). The tool is
`scripts/pkgrepo` (see its [README](../scripts/pkgrepo/README.md)).

Nothing described here is published yet. The domain, the signing key, the host
and the CI secrets do not exist; "What is left to do" lists them.

## Decisions

| Question           | Decision                                                                                     |
| ------------------ | -------------------------------------------------------------------------------------------- |
| Layout             | `https://repo.gnoblin.org/{gnoblin,bingux}/{stable,nightly}/…` and one `gpg.key` at the root |
| Choosing a channel | The user adds the stable repository, the nightly repository, or both                         |
| Package names      | The same in both channels, so a package moves between them as a normal upgrade               |
| Nightly version    | A pre-release of the next stable version: `0.1.13~nightly.<date>.<count>.g<commit>`          |
| Signing            | One key signs the RPMs, the pacman packages and the metadata of all three formats            |
| APT suites         | One suite per distribution release (`trixie`, `noble`), one component (`main`)               |
| APT indexing       | `apt-ftparchive`, which keeps no state                                                       |
| Retention          | Stable keeps everything. Nightly keeps the newest 14 versions of each package.               |
| Architectures      | `x86_64` (`amd64` for APT). Nothing builds for `aarch64` today.                              |
| Tool location      | `scripts/pkgrepo` in the Gnoblin repository, used by Bingux through a pinned commit          |
| Hosting            | A tree of static files. Back ends `rsync`, `s3` and `pages`; the default is a dry run.       |

## Layout

```text
repo.gnoblin.org/
  gpg.key                                      the public key, ASCII-armoured
  index.html
  <project>/                                   gnoblin or bingux
    <channel>/                                 stable or nightly
      manifest.json                            every package, its sha256, the commit it was built from
      index.html
      fedora/
        gnoblin-stable.repo                    ready-made dnf repository file ($releasever, $basearch)
        <release>/<arch>/                      45/x86_64/: RPMs, repodata/, repodata/repomd.xml.asc
      opensuse/
        <release>/gnoblin-stable.repo
        <release>/<arch>/                      tumbleweed/x86_64/
      deb/
        gnoblin-stable-<suite>.sources         ready-made deb822 sources file
        dists/<suite>/{Release,InRelease,Release.gpg}
        dists/<suite>/main/binary-<arch>/{Packages,Packages.gz,Packages.xz}
        pool/<suite>/*.deb
      arch/
        gnoblin.conf                           the pacman.conf section
        <arch>/                                packages, .sig files, gnoblin.db, gnoblin.files (+ .sig)
```

Details that matter:

- `<release>` of RPM trees is what the package manager substitutes: Fedora's
  `$releasever` is `45`, so `fedora/$releasever/$basearch` resolves. openSUSE
  Tumbleweed has no number, so its `.repo` file names `tumbleweed`.
- Architecture-independent packages (`noarch`, `all`, `any`) are copied into
  every architecture directory of their release, so a client never needs a
  second repository for them. APT keeps one copy in the pool and lists it in
  every `binary-<arch>/Packages`.
- The pacman database name is the `pacman.conf` section name. The stable
  section is `[gnoblin]` and its database `gnoblin.db`. The nightly section is
  `[gnoblin-nightly]` and its database `gnoblin-nightly.db`. Without distinct
  names a client could not have both. The database files are copies, not the
  symlinks `repo-add` makes, because static hosts have no symlinks.
- `manifest.json` is the machine-readable record. `pkgrepo pull` reads it to
  fetch the published packages, `pkgrepo needs-build` reads its commit, and
  `pkgrepo publish` reads its checksums to upload only changed files.

### One APT suite per distribution, not one per channel

The channel is already in the URL (`…/nightly/deb`), so a suite named after the
channel would say nothing new. What differs between Debian 13 and Ubuntu 24.04
is the libraries a package was built against, and that is what a suite is for.

- A suite is a distribution codename and there is one component, `main`.
- A package built for two suites is stored once per suite (`pool/<suite>/`).
  That costs disk and never claims a release the package was not built for.
- One `stable` suite with a component per distribution was rejected. APT
  resolves versions across the components of a suite, so a Debian build could
  be picked on Ubuntu.

### Why `apt-ftparchive`

| Tool             | Verdict                                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------- |
| `apt-ftparchive` | Chosen. Regenerates everything from the `.deb` files, so no database has to survive between CI runs.    |
| `reprepro`       | Needs its `db/` and `conf/` directories to be kept and uploaded; a lost database loses package history. |
| `aptly`          | A server-style tool with its own database and snapshot model. More than a static tree needs.            |

`apt-ftparchive` reads the packages and writes `Packages`. The tool splits the
output by architecture, compresses it, asks `apt-ftparchive release` for
`Release`, and signs it (`InRelease` clearsigned, `Release.gpg` detached).

## Channels

A user chooses a channel by adding its repository. There is no setting to
toggle. The same package name lives in both.

| Package manager | Both channels enabled                                                                     | Verified in             |
| --------------- | ----------------------------------------------------------------------------------------- | ----------------------- |
| dnf, zypper     | The highest version of any enabled repository wins: the nightly build.                    | Fedora 43               |
| apt             | The highest candidate wins: the nightly build.                                            | Debian 13, Ubuntu 24.04 |
| pacman          | The first repository in `pacman.conf` that has the package wins, not the highest version. | Arch                    |

So with both channels on, dnf and apt follow nightly. pacman follows whichever
section is listed first. To go back to stable, remove the nightly repository:
`dnf distro-sync` and `apt-get install pkg=VERSION` downgrade, and
`pacman -S pkg` downgrades by name.

## Versions

A nightly must sort above the last stable release and below the next one, in
every package manager, so that installing a release replaces the nightly build
of the same version.

The base of a nightly is the next stable version.

- A release commit writes the new version into the tree and the tag lands on
  that commit. Between releases the tree still carries the last version.
- The next version is therefore the last stable tag with the patch number
  raised.
- When the tree already carries a higher version (a release is being prepared),
  that version is the base.

| Format | Stable          | Nightly                                       | Why it sorts correctly                                             |
| ------ | --------------- | --------------------------------------------- | ------------------------------------------------------------------ |
| RPM    | `0.1.13-1.fc45` | `0.1.13~nightly.20261031.142.gabc1234-1.fc45` | `~` sorts before everything, including the end of the string.      |
| deb    | `0.1.13-1`      | `0.1.13~nightly.20261031.142.gabc1234-1`      | The same rule in dpkg.                                             |
| pacman | `0.1.13-1`      | `0.1.13nightly20261031.142-1`                 | A trailing letter segment sorts below the same version without it. |

The nightly string is `<base>~nightly.<UTC commit date>.<commits since the last
stable tag>.g<short commit>`. Within one day the commit count increases along
the branch, so two nightlies of one day also order correctly. The commit
hash is only a label.

pacman has no tilde. Its comparison treats a letter segment that follows the
number directly (`0.1.13nightly…`) as older than `0.1.13`, but a letter segment
after a dot (`0.1.13.nightly…`) as newer. So the Arch spelling has no dot before
`nightly` and no commit hash. `tests/pkgrepo.test.py` has a test that fails when
someone adds the dot.

`scripts/pkgrepo/versions.py` implements the three orderings and the spellings.
`tests/pkgrepo/compare-versions.py` compares them with `rpm`, `dpkg` and pacman's
`vercmp` on thousands of random pairs plus the nightly strings.

### Where the version is applied

The version is applied to a throw-away CI checkout, never to the repository.

1. `scripts/pkgrepo version --git .` computes the SemVer pre-release
   (`0.1.13-nightly.20261031.142.gabc1234`).
2. `scripts/apply-nightly-version.py` writes it into
   `versions/gnoblin-version.json` and the Gnoblin entries of
   `packaging/native-packages.json`.
3. It runs `sync-package-manifest.py write`, which spells the version for each
   recipe.

The RPM spec gains `%global source_version`. `Version:` holds the tilde
spelling, while the source bundle and its top directory keep the SemVer one. The
Arch recipe gains `_srcver` for the same reason. The portal package keeps the
version of its GNOME component, so a nightly rebuilds it under the release
version.

Bingux does the same in `scripts/build-rpms.sh`, `scripts/build-arch.sh` and
`scripts/build-debs.sh`: the version they receive is written into a copy of the
spec, the PKGBUILD or the Debian changelog.

## Signing

What is signed:

| Part                        | Signature                                                                       | Checked by                          |
| --------------------------- | ------------------------------------------------------------------------------- | ----------------------------------- |
| Each RPM                    | An embedded OpenPGP signature (`rpmsign --addsign`)                             | `dnf` with `gpgcheck=1`             |
| `repodata/repomd.xml`       | `repomd.xml.asc`, detached                                                      | `dnf` with `repo_gpgcheck=1`        |
| `dists/<suite>/Release`     | `InRelease` (clearsigned) and `Release.gpg` (detached)                          | `apt`                               |
| Each `.deb`                 | None of its own. `Packages` lists its SHA-256 and `Release` lists `Packages`'s. | `apt`, through the signed chain     |
| Each pacman package         | `<package>.sig`, detached and binary                                            | `pacman` with `SigLevel = Required` |
| `<name>.db`, `<name>.files` | `.sig`, detached and binary                                                     | `pacman` with `DatabaseRequired`    |

APT does not verify signatures embedded in `.deb` files (`debsigs`, `dpkg-sig`);
it verifies the hash chain from the signed `Release` file. A per-package `debsigs`
signature would be unchecked, so none is written.

The generated `.repo`, `.sources` and `pacman.conf` files turn all checks on.
`pkgrepo build` verifies its own output after signing: every checksum, and every
signature with a key ring that holds only `gpg.key`.

### The key

One key signs everything. Create it offline, once, on a machine that is not CI.
RSA 4096 is the conservative choice: older `rpm` releases cannot verify
Ed25519 signatures.

```sh
export GNUPGHOME="$(mktemp -d)"
gpg --quick-generate-key 'Gnoblin package repository <repo@gnoblin.org>' rsa4096 sign 3y
gpg --list-secret-keys --keyid-format long --with-colons | awk -F: '/^fpr/ {print $10; exit}'
gpg --armor --export-secret-keys FINGERPRINT > repo-private.asc     # store in a password manager and as the CI secret
gpg --armor --export FINGERPRINT > gpg.key                           # the public key
gpg --gen-revoke FINGERPRINT > revoke.asc                            # keep offline: the only way to retract the key
```

Then:

1. Replace `REPLACE-WITH-KEY-FINGERPRINT` in
   `docs/package-repositories.md` with the 40-character fingerprint.
2. Add the repository secrets `REPO_GPG_PRIVATE_KEY` (the contents of
   `repo-private.asc`) and, because the key has a passphrase,
   `REPO_GPG_PASSPHRASE`, to the Gnoblin and Bingux repositories.
3. Delete `repo-private.asc` and the temporary `GNUPGHOME` once the secrets exist.

The private key reaches the CI job only as an environment variable. The tool
imports it into a temporary `GNUPGHOME`, writes the passphrase to a mode 0600
file outside the repository tree, and removes both when the job ends. A fork
that has no secret builds and verifies the tree with a throw-away key and
publishes nothing.

A key that expires after three years forces a rotation. That is deliberate: a
leaked key does not stay valid for ever.

- To rotate, create a new key and publish both public keys in `gpg.key` for one
  release cycle. Then switch the secret and tell users to import the key again.
- To extend the same key, use `gpg --quick-set-expire`, publish the updated
  `gpg.key` and tell users to import it again.

## Retention

- Stable keeps every package. A published stable file is never replaced:
  `pkgrepo build` stops when a stable package of the same name and version is
  already published, and `--replace-stable` is the explicit repair.
- Nightly keeps the newest 14 versions of each package, per distribution release
  and architecture. Older files leave the tree, and the `publish` step removes
  them from the host. 14 days of nightlies is enough to bisect a regression in a
  fortnight; raise `--keep-nightlies` if it is not.
- A nightly package of a version that is already published is replaced. A rerun
  of the same night's build therefore works.

## Hosting

The tree is plain files with no server logic, so any static host serves it.
`pkgrepo publish` moves a tree with one of three back ends. Every back end
uploads in the same order, so a client never reads metadata that names a missing
file:

1. package files that are new or changed (the previous manifest says which),
2. metadata and pages (`repodata/`, `dists/`, the pacman databases,
   `manifest.json`, the index pages, `gpg.key`),
3. removal of files the new tree no longer has.

`--dry-run` is the default: the tool prints the plan, and only `--apply` uploads.

| Back end | Destination                           | Credentials (secrets)                                                                     |
| -------- | ------------------------------------- | ----------------------------------------------------------------------------------------- |
| `rsync`  | `user@host:/srv/repo`, or a directory | `REPO_RSYNC_KEY`, `REPO_RSYNC_KNOWN_HOSTS`                                                |
| `s3`     | `s3://bucket/prefix`                  | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, variable `AWS_ENDPOINT_URL` for R2 or MinIO |
| `pages`  | The Git URL of a Pages repository     | `REPO_PAGES_TOKEN`                                                                        |

### Which host

| Host                          | For                                                                                        | Against                                                                                     |
| ----------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| Cloudflare R2 (`s3`)          | The docs already deploy to Cloudflare. No egress fees, no practical size limit, no server. | A new product and its own API token.                                                        |
| bsociety with rsync and Caddy | Free, no new account, no limits, full control.                                             | Availability of one machine. Put Cloudflare's proxy in front for caching.                   |
| GitHub Pages (`pages`)        | No new account.                                                                            | A site is limited to about 1 GB. Bingux's RPMs are about 50 MB a release, in three formats. |
| Cloudflare Pages              | Same account as the docs.                                                                  | 25 MiB per file: an RPM does not fit. Not a back end.                                       |

The recommendation is R2 with `repo.gnoblin.org` as its custom domain. The
decision is the owner's. The workflows read it from the repository variables
`REPO_BACKEND` and `REPO_DESTINATION`, so changing host is not a code change.

bsociety is the alternative that needs no new product:

- Add a read-only bind mount of the repository directory to the Caddy container.
- Add a site block for `repo.gnoblin.org` with `file_server`.
- Create an SSH key for CI that may write only that directory.

## The tool and where it lives

`scripts/pkgrepo` is the only place that knows the layout, the versions and the
signing. Both projects use it, so a fix lands once.

| Option                                                  | Verdict                                                                                                                                                             |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| In Gnoblin's `scripts/`, Bingux fetches a pinned commit | Chosen for now. No new repository, the tool is tested next to the packaging it serves. Bingux pins the commit in `packaging/pkgrepo.ref`.                           |
| A third repository for the tool                         | Cleaner long term, but it needs a repository, releases and tags. The tool is one directory, so moving it later is `git subtree split` plus `packaging/pkgrepo.ref`. |
| A copy in each repository                               | Rejected. Two copies drift, and a signing fix has to land twice.                                                                                                    |
| A pip package                                           | Rejected. The tool needs system programs anyway, and a package index is one more thing to publish.                                                                  |

The programs it starts (`createrepo_c`, `rpmsign`, `apt-ftparchive`, `repo-add`,
`gpg`) come from one Debian 13 image, `scripts/pkgrepo/container`. CI jobs run
in `debian:trixie` and call the same `install-tools.sh`.

## CI

Gnoblin (`.github/workflows`):

- `release.yml` gains a `repository` stage after `publish`. A release tag builds
  the stable channel from the Fedora, openSUSE and Arch packages of stage 2 and
  publishes it. A push to `main`, a pull request or a manual dry run builds and
  verifies the tree with a throw-away key and publishes nothing.
- `nightly.yml` runs at 02:17 UTC and on demand. It compares the commit in the
  published `nightly/manifest.json` with the commit of the run and stops when they
  are equal. Otherwise it calls `release.yml` with `nightly: true`, which gives
  the checkout the nightly version, skips openSUSE and the GitHub release, runs the
  same build and clean-install checks as a release, and publishes the nightly
  channel.
- `package-repository.yml` is the reusable stage. It takes one concurrency group
  per channel, so two publications never interleave.

Bingux (`.github/workflows/packages.yml` and `nightly.yml`): the same stages, with
`make rpm-package`, `make arch-package` and `scripts/build-debs.sh` as the builds.

Jobs are gated so that a fork builds and verifies only. Publication needs all of:

| Name                                          | Kind     | Purpose                                               |
| --------------------------------------------- | -------- | ----------------------------------------------------- |
| `REPO_GPG_PRIVATE_KEY`, `REPO_GPG_PASSPHRASE` | secret   | Signing                                               |
| `REPO_BACKEND`, `REPO_DESTINATION`            | variable | `rsync`, `s3` or `pages`, and where to                |
| `REPO_URL`                                    | variable | Public base URL; default `https://repo.gnoblin.org`   |
| `REPO_RSYNC_KEY`, `REPO_RSYNC_KNOWN_HOSTS`    | secret   | The `rsync` back end                                  |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`  | secret   | The `s3` back end; variable `AWS_ENDPOINT_URL` for R2 |
| `REPO_PAGES_TOKEN`                            | secret   | The `pages` back end                                  |

## What each project builds today

| Project | Format | Targets built by CI today                      | Not built                                                                        |
| ------- | ------ | ---------------------------------------------- | -------------------------------------------------------------------------------- |
| Gnoblin | RPM    | Fedora 45 and openSUSE Tumbleweed, x86_64      | Fedora 43 and 44, Enterprise Linux, openSUSE Leap (see `packaging/targets.json`) |
| Gnoblin | pacman | Arch Linux, x86_64                             | aarch64                                                                          |
| Gnoblin | deb    | Nothing                                        | Debian and Ubuntu: no recipe exists. See the note below the table.               |
| Bingux  | RPM    | Fedora 43 (`make rpm-package`), x86_64         | aarch64                                                                          |
| Bingux  | pacman | Arch Linux (`make arch-package`), x86_64       | aarch64                                                                          |
| Bingux  | deb    | Debian 13 source package in `packaging/debian` | See the Bingux design note for what was built                                    |

Gnoblin published DEBs until the lean native runtime removed their recipe
(`1f53b8c6`). The current patched Mutter runtime has no Debian recipe and has not
been built against Debian's libraries, so no Gnoblin DEB is built or claimed.
`design/debian12-ubuntu2204-private-runtime.md` records why older releases
cannot work.

No package of either project is built for `aarch64`: the PKGBUILDs say
`arch=('x86_64')` and CI has no ARM runner. The tool takes the architecture from
the package, so an `aarch64` build needs a runner and nothing else.

## Rejected alternatives

- One repository tree per distribution release (`repo.gnoblin.org/fedora-45/…`).
  The layout the owner asked for groups by project and channel, which is also
  how a user thinks about it.
- A single channel that users pin by version. Nightly users would have to edit a
  repository file every day.
- Publishing the nightly under the stable name with a higher epoch. Epochs never
  decrease, so a user could not go back to stable.
- Signing only metadata. RPM and pacman check package signatures, and an
  unsigned package would be installed from a repository that was compromised
  after its metadata was signed.
- GitHub release assets as the repository (what `gnoblin.db` does today). They
  have no per-channel layout, no signatures and no retention. The release asset
  database stays until the repositories are live.

## What is left to do

Nothing was published, and no DNS, account or secret was created.

1. Create the key as above. Fill in the fingerprint in the user documentation.
2. Choose the host, create `repo.gnoblin.org` and point it at the host.
3. Add the secrets and variables in the table above to the Gnoblin and Bingux
   repositories.
4. Run the tool once by hand with `--apply` for each project and channel
   (`scripts/pkgrepo/README.md`), and install from the result on a clean machine.
5. Merge the branch. Cut a release to publish the first stable channel, or run
   `nightly.yml` with `force`.
6. Bingux: after the Gnoblin branch is merged, set `packaging/pkgrepo.ref` of
   Bingux to the merged commit if it is not the one it names.
7. `docs.yml` still copies an APT archive from the `gh-pages` branch into
   `gnoblin.org/apt` and `scripts/prepare-apt-pages.py` still exists, although
   the builder of that archive was removed in `1f53b8c6`. Decide whether to
   delete the old archive.

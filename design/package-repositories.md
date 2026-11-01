# Package repositories

This is internal release engineering material. It describes the signed RPM, APT,
pacman and Alpine repositories that serve Gnoblin and Bingux (Bingux has no
Alpine packages), the channels they have, and how the CI builds and publishes them. Users read
[`docs/package-repositories.md`](../docs/package-repositories.md). The tool is
`scripts/pkgrepo` (see its [README](../scripts/pkgrepo/README.md)).

Nothing described here is published yet. Set up so far: the R2 bucket `gnoblin-repo`
behind `repo.gnoblin.org`, and the signing key (fingerprint in the user documentation).
The key's private part and passphrase are the CI secrets `REPO_GPG_PRIVATE_KEY` and
`REPO_GPG_PASSPHRASE` of the Gnoblin and Bingux repositories. The upload credentials and
the `REPO_*` variables are still missing; "What is left to do" lists them.

## Decisions

| Question           | Decision                                                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| Layout             | `https://repo.gnoblin.org/{gnoblin,bingux}/{stable,nightly}/…`, one `gpg.key` and `<project>.rsa.pub` at the root                    |
| Choosing a channel | The user adds the stable repository, the nightly repository, or both                                                                 |
| Package names      | The same in both channels, so a package moves between them as a normal upgrade                                                       |
| Nightly version    | A pre-release of the next stable version: `0.1.13~nightly.<date>.<count>.g<commit>`                                                  |
| Signing            | One GnuPG key signs the RPMs, the pacman packages and the metadata of those three formats. A second, RSA key signs the Alpine index. |
| APT suites         | One suite per distribution release (`trixie`, `noble`), one component (`main`)                                                       |
| APT indexing       | `apt-ftparchive`, which keeps no state                                                                                               |
| Retention          | Stable keeps everything. Nightly keeps the newest 14 versions of each package.                                                       |
| Architectures      | `x86_64` (`amd64` for APT) and `aarch64` (`arm64` for APT).                                                                          |
| Tool location      | `scripts/pkgrepo` in the Gnoblin repository, used by Bingux through a pinned commit                                                  |
| Hosting            | A tree of static files. Back ends `rsync`, `s3` and `pages`; the default is a dry run.                                               |

## Layout

```text
repo.gnoblin.org/
  gpg.key                                      the public key, ASCII-armoured
  gnoblin.rsa.pub                              the public key of the Alpine index, when the project has apk packages
  <project>/                                   gnoblin or bingux
    <channel>/                                 stable or nightly
      manifest.json                            every package, its sha256, the commit it was built from
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
      alpine/
        <branch>/gnoblin-stable.list           the line for /etc/apk/repositories
        <branch>/<arch>/                       edge/x86_64/: .apk files and APKINDEX.tar.gz
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
- Alpine's `apk` adds `/<arch>/APKINDEX.tar.gz` to the repository line, so the
  line names the branch (`…/alpine/edge`) and the architecture directories hold
  the files. The branch is in the path because a package built on edge links
  libraries of edge; a release branch would be another directory.
- pkgsmith gives the `.apk` files of every architecture the same names
  (`name-version-rN.apk`), so the input of `pkgrepo build` has one directory per
  architecture (`alpine/edge/x86_64/`), and CI takes the architecture from the
  artifact the file came from. `pkgrepo` reads the architecture from the package.
- An architecture-independent package is copied into every architecture
  directory, and its index entry carries the architecture of that directory, not
  `noarch`. apk downloads a package from the directory named by the entry, so a
  `noarch` entry is looked for in `…/noarch/`. abuild does the same with
  `apk index --rewrite-arch`.
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
| apk             | The highest version of any repository wins: the nightly build.                            | Alpine edge (apk 3.0.7) |

So with both channels on, dnf, apt and apk follow nightly. pacman follows whichever
section is listed first. To go back to stable, remove the nightly repository:
`dnf distro-sync` and `apt-get install pkg=VERSION` downgrade, and
`pacman -S pkg` downgrades by name. `apk upgrade` never downgrades, but
`apk add pkg=VERSION` does.

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

| Format | Stable          | Nightly                                       | Why it sorts correctly                                                   |
| ------ | --------------- | --------------------------------------------- | ------------------------------------------------------------------------ |
| RPM    | `0.1.13-1.fc45` | `0.1.13~nightly.20261031.142.gabc1234-1.fc45` | `~` sorts before everything, including the end of the string.            |
| deb    | `0.1.13-1`      | `0.1.13~nightly.20261031.142.gabc1234-1`      | The same rule in dpkg.                                                   |
| pacman | `0.1.13-1`      | `0.1.13nightly20261031.142-1`                 | A trailing letter segment sorts below the same version without it.       |
| apk    | `0.1.13-r1`     | `0.1.13_pre20261031142-r1`                    | The suffix `_pre` sorts below the release, and one number orders builds. |

The nightly string is `<base>~nightly.<UTC commit date>.<commits since the last
stable tag>.g<short commit>`. Within one day the commit count increases along
the branch, so two nightlies of one day also order correctly. The commit
hash is only a label.

pacman has no tilde. Its comparison treats a letter segment that follows the
number directly (`0.1.13nightly…`) as older than `0.1.13`, but a letter segment
after a dot (`0.1.13.nightly…`) as newer. So the Arch spelling has no dot before
`nightly` and no commit hash. `tests/pkgrepo.test.py` has a test that fails when
someone adds the dot.

apk has no tilde and no dot rule either. Its suffix `_pre` sorts before the release
(`0.1.13_pre1` is older than `0.1.13`), and a number after it is compared as a number.
pkgsmith joins the digits of the date and of the commit count into that one number
(`0.1.13_pre20261031142`). Both only grow, so the number does too: a commit count that gains
a digit makes the number longer, which apk reads as larger.

`scripts/pkgrepo/versions.py` implements the four orderings and the spellings.
`tests/pkgrepo/compare-versions.py` compares them with `rpm`, `dpkg`, pacman's
`vercmp` and `apk version -t` on thousands of random pairs plus the nightly strings.

### Where the version is applied

The version is applied to a throw-away CI checkout, never to the repository.

1. `scripts/pkgrepo version --git .` computes the SemVer pre-release
   (`0.1.13-nightly.20261031.142.gabc1234`).
2. `scripts/apply-nightly-version.py` writes it into
   `src/gnoblin-version.json` and the Gnoblin entries of
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

| Part                        | Signature                                                                                                                          | Checked by                             |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| Each RPM                    | An embedded OpenPGP signature (`rpmsign --addsign`)                                                                                | `dnf` with `gpgcheck=1`                |
| `repodata/repomd.xml`       | `repomd.xml.asc`, detached                                                                                                         | `dnf` with `repo_gpgcheck=1`           |
| `dists/<suite>/Release`     | `InRelease` (clearsigned) and `Release.gpg` (detached)                                                                             | `apt`                                  |
| Each `.deb`                 | None of its own. `Packages` lists its SHA-256 and `Release` lists `Packages`'s.                                                    | `apt`, through the signed chain        |
| Each pacman package         | `<package>.sig`, detached and binary                                                                                               | `pacman` with `SigLevel = Required`    |
| `<name>.db`, `<name>.files` | `.sig`, detached and binary                                                                                                        | `pacman` with `DatabaseRequired`       |
| `APKINDEX.tar.gz`           | An RSA signature (SHA-256) in its first gzip stream, `.SIGN.RSA256.<project>.rsa.pub`                                              | `apk`, with the key in `/etc/apk/keys` |
| Each `.apk`                 | None of its own. The index entry holds the SHA-1 of the package's control part, and `.PKGINFO` holds the SHA-256 of its data part. | `apk`, through the signed index        |

APT does not verify signatures embedded in `.deb` files (`debsigs`, `dpkg-sig`);
it verifies the hash chain from the signed `Release` file. A per-package `debsigs`
signature would be unchecked, so none is written.

The generated `.repo`, `.sources` and `pacman.conf` files turn all checks on.
`pkgrepo build` verifies its own output after signing: every checksum, and every
signature with a key ring that holds only `gpg.key`. Each `APKINDEX.tar.gz` is
checked against `<project>.rsa.pub`, and against the packages next to it.

### The Alpine key

`apk` does not read OpenPGP signatures. It reads an RSA signature of the index and finds the
public key by file name in `/etc/apk/keys`, so the key is a second, separate key. The signature
member of the index names the key file (`gnoblin.rsa.pub`), and the tool publishes that file at
the root of the site next to `gpg.key`. The name is the same for the life of the key: a rotation
that keeps the name replaces the file, and one that changes it needs users to add the new name.

Create it offline, once, like the GnuPG key. An encrypted key is not supported, because `apk`
signing here uses `openssl` non-interactively.

```sh
openssl genrsa -out apk-private.pem 4096                    # store in a password manager and as the CI secret REPO_APK_PRIVATE_KEY
openssl rsa -in apk-private.pem -pubout -out gnoblin.rsa.pub   # what the site publishes; the tool derives it, so this copy is only for checking
```

The secret is the contents of `apk-private.pem`. Without it, a run that publishes leaves the Alpine
packages out, and a run that publishes nothing signs them with a throw-away key, so the code path
is still checked. A repository that has published Alpine packages cannot be built without the key.

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
2. metadata (`repodata/`, `dists/`, the pacman databases, `APKINDEX.tar.gz`,
   `manifest.json`, `gpg.key`, `<project>.rsa.pub`),
3. removal of files the new tree no longer has, and of the retired `index.html` at the site root (an earlier version
   of the tool wrote index pages; none are written now).

`--dry-run` is the default: the tool prints the plan, and only `--apply` uploads.

A build merges with the packages that are already published, so a build made
without them would remove them on upload. Two checks guard against it:

- `pkgrepo pull` fails when the published host cannot be reached. A run that
  publishes nothing (no key, no destination, a pull request) may pass
  `--allow-unreachable` and build without them.
- `pkgrepo publish` refuses a first publication when the destination already
  has a `manifest.json` for the channel. That is the sign that `REPO_URL` and
  `REPO_DESTINATION` name different places, or that the pull saw nothing.

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
`gpg`, `openssl`) come from one Debian 13 image, `scripts/pkgrepo/container`. CI jobs run
in `debian:trixie` and call the same `install-tools.sh`.

## CI

Gnoblin (`.github/workflows`):

- `release.yml` builds the source bundle, calls `packages.yml`, publishes the GitHub release for a
  tag, and then calls `package-repository.yml`. `packages.yml` builds the bundle into a prefix on
  Fedora 45 (amd64 and arm64), writes the rpm, deb, Arch and tar packages with pkgsmith, and
  installs them on clean Fedora, openSUSE, Arch and Debian testing systems.
- A release tag builds the stable channel and publishes it. A push to `main`, a pull request or a
  manual dry run builds and verifies the tree with a throw-away key and publishes nothing.
- `nightly.yml` runs at 02:17 UTC and on demand, and only where `REPO_DESTINATION` is set. It compares the
  commit in the published `nightly/manifest.json` with the commit of the run and stops when they
  are equal. Otherwise it calls `release.yml` with `nightly: true`, which gives the checkout the
  nightly version, skips the GitHub release, runs the same build and clean-install checks as a
  release, and publishes the nightly channel.
- `package-repository.yml` is the reusable stage. It takes one concurrency group
  per channel, so two publications never interleave. It sorts the packages into `fedora/45`,
  `opensuse/tumbleweed`, `deb/forky`, `arch` and `alpine/edge/<arch>`.

Bingux has its own workflows with the same stages and its own pkgsmith definitions.

Jobs are gated so that a fork builds and verifies only. Publication needs all of:

| Name                                          | Kind     | Purpose                                               |
| --------------------------------------------- | -------- | ----------------------------------------------------- |
| `REPO_GPG_PRIVATE_KEY`, `REPO_GPG_PASSPHRASE` | secret   | Signing                                               |
| `REPO_APK_PRIVATE_KEY`                        | secret   | Signs the Alpine index; optional (see above)          |
| `REPO_BACKEND`, `REPO_DESTINATION`            | variable | `rsync`, `s3` or `pages`, and where to                |
| `REPO_URL`                                    | variable | Public base URL; default `https://repo.gnoblin.org`   |
| `REPO_RSYNC_KEY`, `REPO_RSYNC_KNOWN_HOSTS`    | secret   | The `rsync` back end                                  |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`  | secret   | The `s3` back end; variable `AWS_ENDPOINT_URL` for R2 |
| `REPO_PAGES_TOKEN`                            | secret   | The `pages` back end                                  |

## What each project builds today

| Project | Format | Targets built by CI                           | Not built                                        |
| ------- | ------ | --------------------------------------------- | ------------------------------------------------ |
| Gnoblin | RPM    | Fedora 45 and openSUSE Tumbleweed, both archs | Fedora 43 and 44, Enterprise Linux, Leap         |
| Gnoblin | pacman | Arch Linux, both archs                        |                                                  |
| Gnoblin | deb    | Debian testing (`forky`), both archs          | Debian 13 and Ubuntu: the libraries are older    |
| Gnoblin | apk    | Alpine edge, both archs                       | Alpine release branches: the libraries are older |

Both architectures are built, installed in containers and published by the same jobs.
The arm64 packages have been written and checked here only from an amd64 prefix (names, metadata);
the first arm64 CI run is the first real arm64 build.

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

Nothing was published. Done: the R2 bucket `gnoblin-repo` (S3 API token to be created for
it), `repo.gnoblin.org` as its custom domain (minimum TLS 1.2), the signing key (RSA 4096,
no expiry, stored in the owner's Bitwarden) and the signing secrets in both repositories.

1. Done: the key is created and its fingerprint is in the user documentation.
2. Done: `repo.gnoblin.org` points at the bucket. Upload `gpg.key` to its root.
3. Create the Alpine key as described above and add `REPO_APK_PRIVATE_KEY` to the Gnoblin
   repository. Until it exists the repository carries no Alpine packages.
4. Add the upload secrets and the variables in the table above to the Gnoblin and
   Bingux repositories: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_ENDPOINT_URL`,
   `REPO_BACKEND` (`s3`) and `REPO_DESTINATION` (`s3://gnoblin-repo`). Only the signing
   secrets exist so far.
5. Run the tool once by hand with `--apply` for each project and channel
   (`scripts/pkgrepo/README.md`), and install from the result on a clean machine.
6. Merge the branch. Cut a release to publish the first stable channel, or run
   `nightly.yml` with `force`.
7. Bingux: after the Gnoblin branch is merged, set `packaging/pkgrepo.ref` of
   Bingux to the merged commit if it is not the one it names.
8. `infra.yml` still copies an APT archive from the `gh-pages` branch into
   `gnoblin.org/apt` and `scripts/prepare-apt-pages.py` still exists, although
   the builder of that archive was removed in `1f53b8c6`. Decide whether to
   delete the old archive.

# Release pipeline: audit and plan

This is internal release engineering material. It records what the CI and release
pipeline does today, what is wrong with it, and the order in which to fix it.
It replaces no document yet. `design/release-packaging.md` still holds the
history of past release runs.

Measured on 2026-10-08 from the GitHub run history and the checked-out tree.

## What a release is meant to produce

The support table in `docs/platform-support.md` names these outputs.

| Output                    | Built from                   | Published to            |
| ------------------------- | ---------------------------- | ----------------------- |
| Source bundle (`.tar.xz`) | The tagged commit            | GitHub release          |
| Fedora source RPMs        | The source bundle            | GitHub release and COPR |
| openSUSE Tumbleweed RPMs  | The source bundle            | GitHub release          |
| Arch package              | The source bundle            | GitHub release          |
| NixOS flake package       | The flake in the tagged tree | The repository itself   |

Debian and Ubuntu are listed as unsupported. Users build them from source.

## What is wrong

### 1. No release has completed since 22 September

- The newest published release is 0.1.4. The repository is at 0.1.10.
- Tags `gnoblin-v0.1.5` to `gnoblin-v0.1.10` exist, and no release is published for
  any of them. The runs for 0.1.7 to 0.1.10 failed. Three draft releases (0.1.7,
  0.1.8, 0.1.9) remain.
- The failing step moved between runs: Debian package builds, then the openSUSE
  job, which looks for `gnoblin-0.1.10-gnome-51.0-source.tar.xz` by name and
  cannot find it.

### 2. Every push to `main` publishes a GitHub release

- `release.yml` runs on every push to `main`, and also on `gnoblin-v*` tags.
- A push to `main` creates a pre-release named "Gnoblin development <sha>". There
  are 8 of them, all from one day. Each run failed after it published.
- The release list mixes real releases, development builds and drafts.

### 3. The Mutter source has two paths that drift

- `./build.sh` resets Mutter to the upstream tag and applies `patches/mutter/`.
- `scripts/make-tarball.sh mutter` does not apply patches. It archives the pinned
  fork commit and copies the overlay. The portal path applies patches; the Mutter
  path does not.
- So a release needs a fork commit with every patch, published by hand to
  `kierandrewett/gnoblin-mutter`. Nothing checks that this was done.
- Evidence: the pin on the branch was a local commit that no remote had. The
  published fork commit has 185 commits on top of the upstream tag. The patch series
  gives 221. Arch, the
  source tarball job and Tumbleweed all failed on one missing file from patch 0005.

### 4. Build dependencies are declared in too many places

Dependencies live in `nix/package.nix`, `packaging/native-packages.json` and the
RPM and Arch files made from it, `lua-runtime.yml`, `verify.yml`,
`application-e2e.yml`, `release.yml` and `docs/install-source.md`.

On one pull request this caused at least six CI failures, each a package, file or
list missing from one place: `ibus` and `gcr` in Nix and `lua-runtime.yml`,
`polkit` in the RPM specs, `gnome-desktop-4` in the openSUSE spec, the Dear ImGui
source in Nix, three workflows and the bundle, a stale test list, and a tarball
job that still expected no portal in the default build. `main` passed these jobs.

### 5. Work is repeated

- The source bundle is built in `verify.yml` and again in `release.yml`.
- The Arch package is built in `verify.yml` and again in `release.yml`.
- The Tumbleweed build runs from `opensuse-rpm.yml` on every push and again
  from the release.
- A pull request starts about 20 jobs.

### 6. A release is gated badly

- The source tarball is published first. A failure later leaves a published
  release with no packages.
- Package jobs for one distribution can block the others, or not, depending on
  `always()` conditions that are hard to read.
- Nothing records which gates a release passed.

### 7. Stale documents

- `design/release-packaging.md` describes the Debian build and the APT repository
  as live. `apt-repository.yml` was removed in `1f53b8c6`.
- The openSUSE RPM adapter workflow was already failing on `main` before this
  branch.

## Target pipeline

Principles:

1. One source of truth for each fact: the dependency lists, the Mutter source,
   the version, the list of tests.
2. Build once. Every package job takes the same source bundle as input.
3. A release is a draft until every required gate passes. Then one step
   publishes it.
4. A pull request runs the same jobs a release runs, on the same inputs. It never
   publishes.
5. A failure points at one job and one reason.

Proposed stages:

```
tag gnoblin-vX.Y.Z
  1. source      build the source bundle once, upload it as an artifact
  2. packages    in parallel, from the artifact:
                   fedora source RPMs, openSUSE RPMs, Arch package, Nix check
  3. install     in parallel: clean install, stock GNOME coexistence, removal
  4. publish     create the draft release, attach every asset and a checksum
                 file, publish only if stages 2 and 3 all passed
  5. copr        submit to COPR from the published source RPMs
```

Pull requests run stages 1 to 3 against the merge commit. `main` runs stages 1 to
3 too, and keeps the bundle as a workflow artifact for 30 days.

## Decisions for the owner

### A. Where does the Mutter source come from at release time?

| Option                                                           | For                                                                                                        | Against                                                                                          |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| 1. Keep the fork commit as the source (today)                    | The bundle holds a reviewed commit.                                                                        | A manual push to a second repository for every patch change. Drift. This broke the pull request. |
| 2. Apply `patches/mutter/` and the overlay onto the upstream tag | One path for `build.sh`, CI and the release. The repository alone is enough. The portal already does this. | The release build runs `git am` (about a minute). A patch conflict shows at release time.        |
| 3. Keep the fork, and have CI push the tip                       | Keeps today's design.                                                                                      | CI needs write access to a second repository. More moving parts.                                 |

### B. What happens to builds from `main`?

| Option                            | For                           | Against                                   |
| --------------------------------- | ----------------------------- | ----------------------------------------- |
| 1. A pre-release per push (today) | Anyone can download a build.  | 8 in a day. The release list is unusable. |
| 2. Workflow artifacts only        | No clutter. 30 day retention. | Needs a GitHub login to download.         |
| 3. One rolling `nightly` release  | One public link. No clutter.  | Each push replaces the files.             |

### C. Should a release publish before every package job passes?

Proposed: no. Build into a draft and publish when all required jobs pass.

## Work order

Each item lands as its own commit, and the tree stays working.

- [x] Build every test with one target, `gnoblin-tests`.
- [ ] Take build dependencies from the RPM specs in every Fedora job, with
      `dnf builddep`, and stop hand-written lists.
- [ ] Decide A, B and C above.
- [ ] Make `make-tarball.sh mutter` follow decision A.
- [ ] Split `release.yml` into the five stages above. Move shared steps into
      reusable workflows.
- [ ] Stop `main` from publishing releases, as decided in B.
- [ ] Make the openSUSE job read the source bundle by a name it is given.
- [ ] Fix the openSUSE RPM adapter, which fails on `main`.
- [ ] Add a check that fails when a dependency list and the specs disagree.
- [ ] Delete the stale Debian and APT sections from
      `design/release-packaging.md`.
- [ ] Dry run a release from a branch: draft only, no tag, no COPR upload.
- [ ] Clean up: remove the 8 development pre-releases and the 3 drafts, after the
      owner agrees.

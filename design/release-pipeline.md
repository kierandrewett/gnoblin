# Release pipeline: audit and plan

This is internal release engineering material. It records what the CI and release
pipeline does today, what is wrong with it, and the order in which to fix it.
It replaces no document yet. `design/release-packaging.md` still holds the
history of past release runs.

Measured on 2026-10-08 from the GitHub run history and the checked-out tree. Updated on 2026-10-21 after the
stage split; the problems below describe the pipeline as measured on the first date.

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

## The rule: the build is the product

`./build.sh` is the one way to build Gnoblin. Everything else is a view of its output:

- A source bundle is the tree `./build.sh` compiles, in an archive.
- A package is the staged install `./build.sh --layout system --destdir DIR` makes, plus
  the package metadata (requirements, scriptlets, file ownership).
- CI runs `./build.sh`. It does not hold its own copy of the build.

A recipe may add what only that distribution knows. It may not repeat what the build
already knows: the file layout, the build flags, the patch series, or the list of tests.

## What is wrong

### 1. No release has completed since 22 September

- The newest published release was 0.1.4. The repository was at 0.1.10.
- Tags `gnoblin-v0.1.5` to `gnoblin-v0.1.10` existed, and no release was published for
  any of them. The runs for 0.1.7 to 0.1.10 failed. Three draft releases (0.1.7,
  0.1.8, 0.1.9) remain.
- The failing step moved between runs: Debian package builds, then the openSUSE
  job, which looked for `gnoblin-0.1.10-gnome-51.0-source.tar.xz` by name and
  could not find it.

Since then, 0.1.10 was published on 2026-10-10, but its tag run still ended in failure. The release was
public with only Fedora source RPMs. The COPR job then built the portal source RPM in a Fedora chroot and
failed on a missing `json-glib` build requirement. The `gnoblin` package was never submitted, so COPR still
offers 0.1.4. The spec on `main` has the requirement, but nothing built it as a binary before the tag. That gap
is why the pipeline now rebuilds both source RPMs into binary RPMs, installs them on a clean Fedora and
checks coexistence and removal before it publishes.

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

Dependencies live in `packaging/nix/package.nix`, `packaging/native-packages.json` and the
RPM and Arch files made from it, `lua-runtime.yml`, `verify.yml`,
`application-e2e.yml`, `release.yml` and `docs/install-source.md`.

On one pull request this caused at least six CI failures. Each was a package, file or
list missing from one place:

- `ibus` and `gcr` in Nix and `lua-runtime.yml`.
- `polkit` in the RPM specs.
- `gnome-desktop-4` in the openSUSE spec.
- The Dear ImGui source in Nix, three workflows and the bundle.
- A stale test list.
- A tarball job that still expected no portal in the default build.

`main` passed these jobs.

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

Pull requests that change the pipeline run stages 1 to 3 against the merge commit. `main` runs them too,
and keeps the bundle as a workflow artifact for 30 days.

Implemented on 2026-10-21 with the install checks inside each package job, so stage 3 is part of stage 2:
`release.yml` holds `source`, `publish` and `copr`, and `fedora-rpm.yml`, `opensuse-rpm.yml` and
`arch-package.yml` hold the package jobs and their install gates.

## Decisions for the owner

### A. Where does the Mutter source come from at release time?

Decided by the rule above: the release tarball applies `patches/mutter/` and the overlay
to the upstream tag, as `./build.sh` does and as the portal already did. The fork no
longer has to carry the patched commits. Done in `81876e39`; two runs of
`scripts/make-tarball.sh mutter` gave byte-identical archives with all 221 patches.

### B. What happens to builds from `main`?

Tried 2026-10-09: option 2. A push to `main` runs the whole pipeline as a dry run and
publishes nothing. The source bundle and packages stay as workflow artifacts for 30 days.
Only a release tag publishes. The 17 existing development pre-releases are not touched.

| Option                             | For                           | Against                                  |
| ---------------------------------- | ----------------------------- | ---------------------------------------- |
| 1. A pre-release per push (before) | Anyone can download a build.  | 17 so far. The release list is unusable. |
| 2. Workflow artifacts only (now)   | No clutter. 30 day retention. | Needs a GitHub login to download.        |
| 3. One rolling `nightly` release   | One public link. No clutter.  | Each push replaces the files.            |

Switching to option 3 later means one more job that replaces the files of a `nightly`
release after a green run on `main`.

### C. Should a release publish before every package job passes?

Tried 2026-10-09: no. A tag creates a draft release with the source tarball.

- The `github-release` job attaches the packages and publishes the draft. It runs only
  when the source RPM, Arch install and Tumbleweed install jobs all passed.
- A failed gate leaves the draft in place.
- COPR runs after the publish, because it reads the source RPMs from the published release.

This path ran for 0.1.10 and published, but the COPR step after it failed (see problem 1). The
stage split on 2026-10-21 changes this again: no draft exists until every gate has passed, so a failed
gate leaves nothing behind but the run's artifacts, and the Fedora binary build is a gate.

### D. How should the portal package come from the one build?

The portal RPM and Arch package build from a separate portal source tarball with Meson, then
copy three files to public paths. That repeats the layout step in two recipes.

| Option                                                     | For                                                  | Against                                                       |
| ---------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------- |
| 1. Keep it. Share only the three-file list                 | No change to package names, versions or assets.      | The paths stay in two recipes and the script.                 |
| 2. Build the portal in the gnoblin recipe, split the files | One build. One source asset. No portal-only tarball. | The portal version (51.0) and gnoblin version (0.1.10) merge. |
| 3. Ship the layout script as a sidecar source              | Recipes call one script. Versions stay separate.     | Another release asset. The script must match the tarball.     |

This changes release assets and package metadata, so it needs the owner's decision.

## Work order

Each item lands as its own commit, and the tree stays working.

- [x] Build every test with one target, `gnoblin-tests`.
- [x] Take Fedora build dependencies from the RPM specs in the Lua runtime job.
- [x] Decision A: make `make-tarball.sh mutter` follow the patch series.
- [x] Add the system layout to the build: `./build.sh --layout system --destdir DIR`,
      with a test against the recipes' own file lists.
- [x] Make the Fedora, openSUSE and Arch recipes call `./build.sh` and copy the stage.
- [x] Watch the CI jobs that build those recipes, and fix what they show. Five fixes landed:
      the link check in `build.sh`, two stale packaging tests, the RPM isolation rules, and
      the `systemd-libs` requirement on openSUSE.
- [x] Run the release workflow on `main` with the new recipes. It first failed in three
      package jobs and the Arch install gate, and passed in full on `64c07a5a`. The fixes
      were a removed `mutter` spec, a wrong Arch package name, a path error in the
      openSUSE job, and a missing `ibus` runtime dependency. Pull request checks do not
      run these steps, so only a dry run of the release path can catch them early.
- [ ] Move the portal recipes onto the layout step. See decision D below.
- [x] Add `./build.sh package`: the source bundle, component archives, Arch recipes and
      checksums from the one tree, with `--srpm` for Fedora source RPMs. `verify.yml` and
      `release.yml` call it. A binary tree is already `--layout system --destdir`.
- [x] Try B and C above (option 2 for B, draft until green for C). Tag path run for 0.1.10; see problem 1.
- [x] Split `release.yml` into the stages above. Fedora, openSUSE and Arch are reusable workflows that
      take the source bundle as an artifact. One `publish` job attaches every asset, checksums and
      attests them, and publishes. Pull requests that change the pipeline run it as a dry run.
- [x] Stop `main` from publishing releases.
- [x] Make the openSUSE job read the source bundle by a name it is given. It takes the artifact name as
      an input and globs for the bundle inside.
- [x] Fix the openSUSE RPM adapter, which failed on `main`. It passes on `main` since 2026-10-21, and
      only the release workflow runs it on a push now.
- [x] Stop the Fedora and Arch jobs naming packages that the recipes already require. Fedora jobs
      install with `dnf builddep` from the specs, and the hand-written lists in `verify.yml`,
      `application-e2e.yml` and `lua-runtime.yml` keep only tools and test programs that no spec
      requires. Arch jobs install with `scripts/install-arch-build-deps.sh` from the PKGBUILDs.
      A package missing from a recipe now fails CI the way it fails a user. A name-by-name check
      across distros was not added: the same library has a different package name on each one.
- [ ] Keep the other hand-written lists in step with their recipes: `packaging/nix/package.nix`,
      `packaging/opensuse/gnoblin.spec`, and the small installs in `lua.yml`.
- [x] Mark `design/release-packaging.md` as a historical log, with a pointer to this file.
- [x] Dry run a release from a branch. `gh workflow run release.yml --ref BRANCH -f dry_run=true`
      builds the source bundle and the Fedora source RPMs, builds the Arch package and runs its
      clean-install gate, and builds the Tumbleweed RPMs and runs their install and GNOME
      coexistence gates. It publishes nothing: no release, no tag, no COPR upload. Run it after
      any change to `release.yml`, the recipes or the generator, and before the merge.
- [x] Run the Fedora binary build and its clean-install, GNOME coexistence and removal checks in a
      dry run. `fedora-rpm.yml` rebuilds both source RPMs with `rpmbuild --rebuild` as an unprivileged user,
      and `scripts/check-fedora-install.sh` runs the checks. COPR runs the same script after publication.
      The dry run on the branch passed every job after fixes; see "What the first dry run found" below.
- [x] Add `scripts/prepare-release.sh`: version bump in both manifests, recipe regeneration, commit and tag,
      without a push.
- [x] Publish one release from one script, `scripts/publish-github-release.sh`, with an asset check,
      a checksum check and no change to a public release. Add build provenance attestations and
      grouped release notes (`.github/release.yml`).
- [ ] Make the negated checks in `arch-package.yml`, `opensuse-rpm.yml` and `verify.yml` enforce. Replace each bare
      `! command` with a check that fails, then fix what the real checks show.
- [ ] Cut 0.1.11 from `main` to exercise the tag path with the new stages and fix what it shows.
- [ ] Clean up: remove the 8 development pre-releases and the 3 drafts, after the owner
      agrees.

## What the first dry run found

The dry run on the branch (run 39679190697) passed every job after three rounds of fixes. The new Fedora binary
build and install gate exposed each problem. Each one would have failed in COPR, or reached users, after
publication.

- **Compiler error in Mutter.** Mutter hardcodes `-Werror=array-bounds`. Fedora 45's GCC reports a false positive in
  `src/core/keybindings.c` under the RPM LTO flags. The spec adds `-Wno-error=array-bounds`.
- **Development packages as runtime requirements.** The `gnoblin` RPM required about 50 `pkgconfig(...)` names from
  the private Mutter `.pc` files. Each named a `-devel` package, so installing Gnoblin pulled in `gtk3`,
  `gtk3-devel` and `libcanberra-devel`. Both RPM specs now exclude every `pkgconfig(...)` requirement, and
  `scripts/check-rpm-isolation.py` rejects them.
- **Checks that never failed.** A bare `! command` does not stop a script under `set -e`, so the old "must be
  absent" checks enforced nothing. The Fedora gate uses helpers now. The `! pacman -Q` and `! rpm -q` lines in
  `arch-package.yml`, `opensuse-rpm.yml` and `verify.yml` have the same flaw and are not fixed yet.

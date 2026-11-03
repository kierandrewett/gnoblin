# Package suite

These tests check the packages that CI built, before anything is released. In the release workflow they run in the
`test` job of each target leg (`.github/workflows/target.yml`), on every target and architecture of
`scripts/supported-targets.py`. The release gate (`gate.py`) then decides which packages may be published: those
whose every target passed the prefix build, the packages, this suite and the graphical VM test.
`design/package-testing.md` lists the failure modes that each layer detects and what the suite cannot prove.

## Run it for one target

Download the package artifacts of a workflow run, then run the suite for one target:

```sh
gh run download RUN_ID --pattern '*-commit-0123456789ab-*' --dir artifacts
scripts/run-package-tests.sh --target deb13 --arch amd64 --artifacts artifacts --release-tag commit-0123456789ab
```

The report is in `build/package-tests/deb13-amd64/`: `report.json`, `junit.xml`, `summary.md` and `evidence/`.
`evidence/command` is the command that repeats the run.

| Option               | Meaning                                                                                                |
| -------------------- | ------------------------------------------------------------------------------------------------------ |
| `--target`, `--arch` | The slug and the architecture of the target, as `scripts/supported-targets.py --table` names them      |
| `--targets FILE`     | The output of `scripts/supported-targets.py`. Without it the script runs the generator                 |
| `--artifacts DIR`    | The artifact folders of the run, with their workflow names                                             |
| `--release-tag TAG`  | The tag in the artifact names. Default: `local`                                                        |
| `--layers LIST`      | Some of `static,install,runtime,repository`. Default: all four                                         |
| `--prefixes DIR`     | The prefix artifacts of the run. With them, the static layer checks that the packages are reproducible |
| `--version SEMVER`   | The version the packages were written with, for a nightly. Default: the version of the tree            |
| `--mutate NAME`      | A negative test: break a copy of the packages and pass only when the layer rejects it                  |

The install, runtime and repository layers start containers. `CONTAINER_ENGINE` names the engine and its options, for
example `CONTAINER_ENGINE="podman --root /var/tmp/podman"`. The static layer needs `rpm`, `rpm2cpio`, `cpio`,
`dpkg-deb`, `zstd`, `readelf`, `desktop-file-validate`, `glib-compile-schemas` and `systemd-analyze` on the machine.

## Files

| File                    | What it does                                                                             |
| ----------------------- | ---------------------------------------------------------------------------------------- |
| `static.py`             | Layer 1: metadata, files, modes, links, build paths, checksums and ABI, no install       |
| `install.sh`            | Layer 2, in a container of the target: install, upgrade, reinstall and remove            |
| `abi_owner.py`          | Case I5 of layer 2: each library comes from a package that the package asks for          |
| `runtime.sh`            | Layer 3, in a container of the target: installs the packages and the test tools          |
| `runtime-session.sh`    | The session cases of layer 3: globals, bridge, `gnoblinctl`, configuration, capture      |
| `runtime_probe.py`      | Helpers of layer 3                                                                       |
| `repository.py`         | Layer 4: a signed repository tree of the packages with throw-away keys                   |
| `gate.py`               | The release gate: the verdict of every target, the coverage table and the publish set    |
| `mutate.py`             | The broken packages of the negative tests                                                |
| `formats.py`, `elf.py`  | Readers of the package formats and of ELF files                                          |
| `targets.py`            | Which files of which artifact belong to which target                                     |
| `report.py`, `suite.py` | Results, `report.json`, `junit.xml`, `summary.md`                                        |
| `expectations.json`     | The package set, its licences and architectures, accepted messages and exceptions        |
| `globals.json`          | The 57 Wayland globals that the compositor can register, and the container reasons       |
| `log-allowlist.txt`     | Log lines that the runtime layer accepts, each with its reason                           |
| `selftest/`             | The self test, run by `tests/check-fast.sh` and the `Verify installation guide` workflow |

## Negative tests

Each mutation breaks a copy of the gnoblin package of one target, and the run passes only when the named layer fails:

```sh
scripts/run-package-tests.sh --target deb13 --arch amd64 --artifacts artifacts --mutate foreign-file
```

| Mutation           | Layer        | Case | Formats        |
| ------------------ | ------------ | ---- | -------------- |
| `foreign-file`     | `static`     | S4   | deb, Arch, tar |
| `build-path`       | `static`     | S7   | deb, Arch, tar |
| `drop-copyright`   | `static`     | S2   | deb            |
| `drop-library-dep` | `install`    | I5   | deb            |
| `drop-gobject-dep` | `install`    | I6   | deb            |
| `rename-global`    | `runtime`    | R2   | deb, Arch      |
| `corrupt-payload`  | `repository` | P1   | rpm            |

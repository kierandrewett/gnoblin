# Session install status

`cargo xtask dist` installs the Rust session (compositor, control CLI, portal, PAM module, the core library and their data) into a prefix. Source: `xtask/src/dist.rs`. `cargo xtask dist-check` checks the result (`xtask/src/dist_check.rs`) and `cargo xtask stage` makes the build tree runnable (`xtask/src/stage.rs`).

## Commands

```sh
just dist prefix=/usr/local                 # cargo build --release --workspace and the XR extension, then install
just dist prefix=/opt/gnoblin destdir=/tmp/stage
cargo xtask dist install --prefix /usr/local [--destdir DIR] [--profile NAME] [--libdir NAME]
cargo xtask dist uninstall --prefix /usr/local [--destdir DIR]
just dist-check                             # build, stage, install into temporary trees and check them (CI runs this)
```

`--destdir` is prepended to the prefix, so a staged tree keeps the prefix layout under it. `--profile` selects the build directory of the binaries (`release` by default; `debug` for a plain `cargo build`). `--libdir` names the library directory under the prefix: `lib` (the default), `lib64`, or a multiarch directory such as `lib/x86_64-linux-gnu`. `CARGO_TARGET_DIR` is honoured.

## Build order

`gnoblin` records `NEEDED libgnoblin-core.so.1` once. The link goes through a stub that `crates/gnoblin/build.rs` generates (`gnoblin_core_abi::build_support::link_core`), so `cargo build -p gnoblin` succeeds alone and never builds the library. The binary's run path is `$ORIGIN:$ORIGIN/../lib:$ORIGIN/../lib64` (`GNOBLIN_RPATH` replaces it when `gnoblin` is built; empty means none, which a `/usr` package wants), so with `LD_LIBRARY_PATH` unset it finds the library beside itself in the build tree and under `<prefix>/lib` or `<prefix>/lib64` once installed. A multiarch libdir under a private prefix is not on the run path; `dist-check` names that.

In the build tree the library is `target/<profile>/libgnoblin_core_impl.so`, and must be reachable as `libgnoblin-core.so.1` beside `gnoblin`:

| Command                                             | Result                                                                                                                                                                                                               |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo build --workspace`, `cargo test --workspace` | builds the library too, and `gnoblin-core-impl/build.rs` links `target/<profile>/libgnoblin-core.so.1` to it                                                                                                         |
| `cargo xtask stage [--release]`                     | builds `gnoblin-core-impl`, refuses a library `dist-check` would refuse, makes the SONAME link and the linker link `libgnoblin-core.so`; run it before `cargo build -p gnoblin` or `cargo test -p gnoblin`           |
| `cargo build -p gnoblin` alone                      | succeeds, with a build-script warning that names `cargo xtask stage` while the core is not beside the binary; running the binary then stops with the loader's `libgnoblin-core.so.1: cannot open shared object file` |

## `dist-check`

`cargo xtask dist-check --prefix P [--destdir D] [--libdir NAME] [--shell FILE]... [--run]` checks an install tree, `--staged [--profile NAME]` the build tree, and `--core`, `--launcher`, `--std`, `--shell`, `--std-shell` single files. Each line is `ok`, `FAIL` or `info`; the exit code is 1 on any `FAIL`.

| Check                                                                                                                                                                                                            | Where it applies                                           |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| core: SONAME `libgnoblin-core.so.1`, `NODELETE`, defined symbols equal `exports.txt`, no `__rust_*` or `rust_eh_personality`                                                                                     | core                                                       |
| launcher: `NEEDED libgnoblin-core.so.1` exactly once, no std, no `gnoblin_*` of its own                                                                                                                          | `bin/gnoblin`, `--staged`                                  |
| every gnoblin library the launcher needs resolves the way the loader would with `LD_LIBRARY_PATH` unset: run path, then the tree's own libdir for a `/usr` prefix; the file found carries the name as its SONAME | launcher, std                                              |
| `ldd` reports nothing missing (the system libraries; a gnoblin library resolved through a staged `/usr` is not on this machine's path)                                                                           | launcher                                                   |
| a shell: core only in NEEDED, defines no `gnoblin_*`, exports only `gnoshell_entry_v1`, references only exported names                                                                                           | `--shell`, and every `.so` under `<libdir>/gnoblin/shells` |
| SONAME links, `include/gnoblin/core.h` equals the generated header, the `.pc` file parses and a C program built through `pkg-config` runs and reports the ABI version                                            | install tree                                               |
| every manifest entry is present and unchanged; the layout of `design/modular-framework.md` section 11 is complete                                                                                                | install tree                                               |
| the glibc symbol floor of each file is printed (`--max-glibc X.Y` turns it into a failure)                                                                                                                       | all files                                                  |
| `--run`: `gnoblin --version` and `gnoblin --role compositor --help` succeed with `LD_LIBRARY_PATH` and `LD_PRELOAD` removed                                                                                      | launcher                                                   |

## Exit codes

| Code | Meaning                                                                           |
| ---- | --------------------------------------------------------------------------------- |
| 0    | Success.                                                                          |
| 1    | Refused or failed install or uninstall. A refusal happens before the first write. |
| 2    | Usage error (unknown action or flag, missing `--prefix`, bad value).              |

## What is installed

| Installed path                                                                                                     | Source                                                         |
| ------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| `bin/gnoblin`, `bin/gnoblinctl`                                                                                    | build directory                                                |
| `libexec/xdg-desktop-portal-gnoblin`                                                                               | build directory                                                |
| `libexec/gnoblin-xr` (only when built)                                                                             | build directory                                                |
| `lib/security/pam_gnoblin_keyring.so`                                                                              | build directory                                                |
| `lib/pam.d/gnoblin-lock`                                                                                           | `data/pam.d/gnoblin-lock`                                      |
| `lib/systemd/user/gnoblin-session.target`                                                                          | `data/session/systemd-user/`                                   |
| `share/wayland-sessions/gnoblin.desktop` (template)                                                                | `data/session/`                                                |
| `share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service` (template)                             | portal crate `data/`                                           |
| `share/xdg-desktop-portal/portals/gnoblin.portal`                                                                  | portal crate `data/`                                           |
| `share/xdg-desktop-portal/gnoblin-portals.conf`                                                                    | `data/gnoblin-portals.conf`                                    |
| `<libdir>/libgnoblin-core.so.<major>.<minor>.<patch>`                                                              | `libgnoblin_core_impl.so` of `gnoblin-core-impl`, renamed      |
| `<libdir>/libgnoblin-core.so.<major>` (symlink to the file above), `<libdir>/libgnoblin-core.so` (symlink to that) | symlinks, recorded in the manifest as `symlink:<target>`       |
| `include/gnoblin/core.h`                                                                                           | generated from the ABI crate (`cargo xtask abi-header`)        |
| `<libdir>/pkgconfig/gnoblin-core.pc` (template)                                                                    | generated                                                      |
| `share/gnoblin/js/gnoblin-core/**`, `share/gnoblin/js/gnoblin-std/**`                                              | `data/js/gnoblin-core/`, `data/js/gnoblin-std/`, every file    |
| `share/gnoblin/shells/`, `<libdir>/gnoblin/shells/` (empty directories)                                            | made by the install; launcher files and shell modules go there |

The core, its links, the header, the `.pc` file and the two shell directories are installed only when `gnoblin-core-impl` was built, so a tree never describes a library it does not contain. `just dist` builds it. The JavaScript library is installed as data while the runtime still embeds a copy and falls back to it; nothing reads the installed copy yet. The old Lua data (`init.lua.example`, `default-config`, `lua`, `shaders`) is no longer installed; the Lua shell was removed.

Template files carry `@PREFIX@`, replaced with the prefix when written. The prefix must be an absolute path with no trailing slash, `..`, whitespace, quotes, backslash, `%` or `$`.

Every installed file is listed in `share/gnoblin/install-manifest` as `<sha256>  <path>`, relative to the prefix. A symlink is listed as `symlink:<target>  <path>` (its target must stay inside the prefix) and a directory the install made as `dir  <path>`.

- An install replaces only files the manifest lists and that still hold what it wrote. Any other file at a destination is refused (exit 1, nothing written).
- A listed file changed since the install is refused too.
- A file the manifest lists but the build no longer ships is removed if unchanged, and kept with a message if changed.
- `uninstall` removes unchanged listed files, keeps changed ones, prunes empty directories, and removes the manifest when nothing is kept. A listed directory is removed only while it is empty (`left <path>: not empty`): a shell package may have installed into it. An install adopts a directory that already exists, and refuses a file where one belongs.

## Not installed

- GeoClue authorisation files, process priority rules and man pages are distribution-owned and not in the install table.

## Version

- `Cargo.toml` `[workspace.package] version`: `0.2.0-alpha.0`.
- Not bumped. The release decision is Kieran's.

## Gates

Run on Agent37 (`a37-gate.sh`) at commit `b40efadc9`, which this change is based on. Every gate exited 0.

| Gate                                                    | Result                  |
| ------------------------------------------------------- | ----------------------- |
| `cargo check --workspace --all-targets`                 | rc 0, no warnings       |
| `cargo clippy --workspace --all-targets -- -D warnings` | rc 0                    |
| `cargo fmt --all --check`                               | rc 0                    |
| `cargo test -p xtask`                                   | rc 0, 94 passed         |
| `cargo test -p xdg-desktop-portal-gnoblin`              | rc 0, all suites passed |
| `xtask dupes --deny`                                    | rc 0                    |
| `xtask logging`, `xtask deps`                           | rc 0 each               |

## Installed-tree smoke

Run on Agent37 with a headless session: `cargo build --release`, `xtask dist install --destdir /home/node/smoke/stage`, then only the installed binaries and data.

| Step                                              | Result                                            |
| ------------------------------------------------- | ------------------------------------------------- |
| Release build of the four artifacts               | rc 0                                              |
| Install into the staging root                     | rc 0, 24 files                                    |
| `gnoblinctl ping`                                 | rc 0, `pong`                                      |
| `gnoblinctl status`                               | rc 0, `running`, revision 9, unlocked             |
| `gnoblinctl window list`                          | rc 0, empty list                                  |
| Installed `init.lua` accepted by the runtime      | compositor log: configuration revision 1 accepted |
| `gnoblinctl config validate <installed init.lua>` | rc 2, see the bug below                           |
| Uninstall via manifest                            | rc 0, 24 files removed, no files left             |
| Guardian exit on SIGTERM                          | 0                                                 |

The staging root directories (`stage`, `stage/usr`, `stage/usr/local`) remain. The smoke created them and the uninstall keeps the prefix directory itself.

The portal fails to start in this smoke because there is no session bus at `run/bus`. The guardian retries and then stops with the session. This is expected for a headless run without D-Bus.

### Bug found by the smoke (not fixed here)

`gnoblinctl config validate` cannot be reached. In `crates/gnoblinctl/src/cli.rs`, `command_tree()` adds the generated object command `config` (`list`, `watch`) before the hand-written verb `config` (`get`, `reload`, `validate`). Both are named `config`, so the object command shadows the verb, and clap reports `unrecognized subcommand 'validate'`. The fix is a design decision: rename one of them, or merge them under one `config` command. It needs an owner's decision and is outside the dist change.

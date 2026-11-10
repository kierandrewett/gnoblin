# Session install status

`cargo xtask dist` installs the Rust session (compositor, control CLI, portal, PAM module and their data) into a prefix. Source: `xtask/src/dist.rs`.

## Commands

```sh
just dist prefix=/usr/local                 # cargo build --release --workspace, then install
just dist prefix=/opt/gnoblin destdir=/tmp/stage
cargo xtask dist install --prefix /usr/local [--destdir DIR] [--profile NAME]
cargo xtask dist uninstall --prefix /usr/local [--destdir DIR]
```

`--destdir` is prepended to the prefix, so a staged tree keeps the prefix layout under it. `--profile` selects the build directory of the binaries. `CARGO_TARGET_DIR` is honoured.

## Exit codes

| Code | Meaning                                                                           |
| ---- | --------------------------------------------------------------------------------- |
| 0    | Success.                                                                          |
| 1    | Refused or failed install or uninstall. A refusal happens before the first write. |
| 2    | Usage error (unknown action or flag, missing `--prefix`, bad value).              |

## What is installed

| Installed path                                                                         | Source                              |
| -------------------------------------------------------------------------------------- | ----------------------------------- |
| `bin/gnoblin`, `bin/gnoblinctl`                                                        | build directory                     |
| `libexec/xdg-desktop-portal-gnoblin`                                                   | build directory                     |
| `lib/security/pam_gnoblin_keyring.so`                                                  | build directory                     |
| `lib/pam.d/gnoblin-lock`                                                               | `data/pam.d/gnoblin-lock`           |
| `lib/systemd/user/gnoblin-session.target`                                              | `data/session/systemd-user/`        |
| `share/wayland-sessions/gnoblin.desktop` (template)                                    | `data/session/`                     |
| `share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service` (template) | portal crate `data/`                |
| `share/xdg-desktop-portal/portals/gnoblin.portal`                                      | portal crate `data/`                |
| `share/xdg-desktop-portal/gnoblin-portals.conf`                                        | `data/gnoblin-portals.conf`         |
| `share/gnoblin/init.lua.example`, `share/gnoblin/shaders/*`                            | `data/`                             |
| `share/gnoblin/default-config/**`, `share/gnoblin/lua/**`                              | `data/default-config/`, `data/lua/` |

Template files carry `@PREFIX@`, replaced with the prefix when written. The prefix must be an absolute path with no trailing slash, `..`, whitespace, quotes, backslash, `%` or `$`.

Every installed file is listed in `share/gnoblin/install-manifest` as `<sha256>  <path>`, relative to the prefix.

- An install replaces only files the manifest lists and that still hold what it wrote. Any other file at a destination is refused (exit 1, nothing written).
- A listed file changed since the install is refused too.
- A file the manifest lists but the build no longer ships is removed if unchanged, and kept with a message if changed.
- `uninstall` removes unchanged listed files, keeps changed ones, prunes empty directories, and removes the manifest when nothing is kept.

## Not installed

- `data/session/systemd-user/gnoblin-idle.service.in` is stale. It runs `@PREFIX@/libexec/gnoblin-idle`, a binary from the Mutter-era CMake build (`src/idle.c`). No Rust target builds it, and nothing in the Rust tree references the unit. Decide whether to port the idle daemon or delete the unit. Until then it is not installed.
- Ananicy rules, the GeoClue authorisation file and man pages are distribution-owned and not in the install table.

## Version

- `Cargo.toml` `[workspace.package] version`: `0.2.0-alpha.0`.
- `data/gnoblin-version.json`: `0.1.13` (format 1). Read by `scripts/gnoblin-version.py`, `scripts/build-identity.py` and `.github/workflows/packages.yml`.
- The comment at `Cargo.toml:97` says the version comes from `versions/gnoblin-version.json`. That path does not exist; the file is `data/gnoblin-version.json`. The comment is wrong.
- Not bumped. The release decision is Kieran's.

## Mutter-era files still in the tree (cutover work)

These build or package the retired Mutter-based compositor. They are not part of the Rust install and are not removed by this change.

- Build: `CMakeLists.txt` (5 mutter references), `cmake/` (9 files, Mutter-era build helpers), `build.sh` (7 references), `flake.nix` (6 references). `Makefile` has no mutter reference but only drives `build.sh`.
- Session: `data/session/schemas/00_org.gnoblin.mutter.gschema.override`.
- Packaging: `pkgsmith/` at the repo root (Starlark specs). `runtime.star` has a mutter reference. `packaging/nix` does not exist.
- CI: `.github/workflows/packages.yml`, `verify.yml`, `lua-runtime.yml`, `application-e2e.yml` reference Mutter.

Not stale: the `crates/compat/gnoblin-compat-dbus-mutter-*` crates. They implement Mutter's D-Bus interfaces on purpose.

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
| `xtask logging`, `xtask deps`, `xtask layout`           | rc 0 each               |

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

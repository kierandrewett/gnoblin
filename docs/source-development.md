# Source development

Start with [build from source](install-source.md). Gnoblin is a Cargo workspace driven by `just` recipes and
`cargo xtask` commands. `justfile` is the list of what CI runs, so a local `just ci` and the GitHub jobs cannot drift.

## Build

```sh
just build              # cargo build --workspace --all-targets
cargo xtask stage       # stage the core and std libraries beside target/debug/gnoblin
```

The compositor lives in `libgnoblin_core_impl.so`, not in the `gnoblin` launcher, so a launcher built alone needs the
library staged beside it.

- `cargo build --workspace` stages the library.
- `cargo xtask stage` stages it for a launcher built with `cargo build -p gnoblin`.
  `CONTRIBUTING.md` has the build order in "The core library in the build tree".

Native compositor changes require a fresh session. A config reload does not load rebuilt libraries. For shell API
methods, events and operation results, use the [runtime API reference](/shell-api/) and [event catalog](/events/).

## Preview a change

Run the build tree as a nested session in a window of your desktop:

```sh
cargo xtask stage
GNOBLIN_BACKEND=nested target/debug/gnoblin
```

`GNOBLIN_BACKEND=headless` runs the same session with no window at all. See [backends](backends.md) for the settings of
each. A nested session is not a login: use [hardware verification](real-hardware-verification.md) for that.

## Choose a prefix

`cargo xtask dist install --prefix PREFIX` installs a release build under `PREFIX`; `just dist PREFIX` builds it first.

- `--libdir`, `--destdir` and `--profile` choose the library directory, a staging root and the cargo profile.
- The [install layout](install-layout.md) lists the files.
- `cargo xtask dist uninstall --prefix PREFIX` removes exactly what the install wrote.
- `just dist-check` installs into three temporary trees and checks each.

## Portal backend

The portal backend, `xdg-desktop-portal-gnoblin`, is a crate of the workspace and `cargo xtask dist` installs it. A GNOME
session can continue using GNOME's backend. To choose another backend for Gnoblin, see [portals](/config/configure/portals).

## Verify changes

```sh
just ci                 # the whole gate: the root workspace, the extension workspace and the core-only build
just workspace-ci       # only the root workspace
just script-tests       # the scripts' own tests (no build needed)
cargo test -p gnoblin-js -p gnoblin-js-types -p gnoblinctl
```

[Testing](testing.md) says which check proves which part of the session. A successful build or test run does not verify
login on a real seat; see [hardware verification](real-hardware-verification.md).

## Where code goes

Gnoblin-owned behaviour goes in a crate under `crates/`, named for its area (`gnoblin-portal-core`) or, when it builds a
binary, for the binary. Do not commit build prefixes, generated tarballs or local session configuration.
`cargo xtask layout` fails on a reference to the retired C source and patch trees of the Mutter product, which lives on
the `main` branch.

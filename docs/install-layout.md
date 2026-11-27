# Install layout

This page lists the files a Gnoblin install puts on disk, so you can package it, find a library a shell needs, or check
that an install is whole. Paths are relative to the install prefix, `/usr` for a distribution package.

`<libdir>` is the library directory: `lib` by default, `lib64` on some distributions, or a multiarch directory such as
`lib/x86_64-linux-gnu`.

## Install from a source tree

Build the tree, then install it:

```sh
cargo build --release
cargo xtask dist install --prefix ~/gnoblin --profile release
```

| Option            | Accepted values           | Default   | Effect                                                                      |
| ----------------- | ------------------------- | --------- | --------------------------------------------------------------------------- |
| `--prefix PREFIX` | An absolute path          | required  | Where the files go. The session file and the portal file name it.           |
| `--destdir DIR`   | A directory               | none      | Stages the install under `DIR`, for a package build.                        |
| `--profile NAME`  | A cargo profile directory | `release` | Which `target/NAME/` directory to install from, for example `debug`.        |
| `--libdir NAME`   | A relative path           | `lib`     | The library directory under the prefix: `lib`, `lib64` or a multiarch path. |

`dist install` lists every file it wrote in `share/gnoblin/install-manifest` with a sha256 digest. It refuses to
overwrite a file the manifest does not list, and `cargo xtask dist uninstall --prefix PREFIX` removes exactly what the
manifest lists and still holds what the install wrote.

A prefix that is not on the loader's search path needs no `LD_LIBRARY_PATH`: `gnoblin` looks for its libraries in the
`lib` and `lib64` directories under its own prefix.

A shell that uses the std library needs nothing either. Before `gnoblin` loads such a shell it opens the std library
from beside the core, so the shell runs under any prefix. [Write a shell in
Rust](rust-shell.md#link-to-the-installed-libraries) has the details.

A multiarch library directory works for a prefix the loader already searches, such as `/usr`.

`cargo xtask dist-check --prefix PREFIX` verifies an installed tree: the core library's name and exports, what each
library links against, and that no library is missing.

## Programs

| Path                                 | What it is                                     |
| ------------------------------------ | ---------------------------------------------- |
| `bin/gnoblin`                        | The session entry point. Starts the shell.     |
| `bin/gnoblinctl`                     | The command-line client for a running session. |
| `libexec/xdg-desktop-portal-gnoblin` | The portal backend.                            |

## Libraries

Each Gnoblin library has a SONAME with its ABI major version: `libgnoblin-core.so.1` and `libgnoblin-std.so.1`. A shell
records that SONAME, so any installed library with the same major version satisfies it.

| Path                                            | What it is                                                              |
| ----------------------------------------------- | ----------------------------------------------------------------------- |
| `<libdir>/libgnoblin-core.so.1.<minor>.<patch>` | The compositor: windows, input, outputs, the tree and the shell loader. |
| `<libdir>/libgnoblin-core.so.1`                 | A link to the file above. This is the SONAME a shell asks for.          |
| `<libdir>/libgnoblin-core.so`                   | A link for the linker, used when you build a shell.                     |
| `<libdir>/libgnoblin-std.so.1.<minor>.<patch>`  | The optional desktop library: workspaces, layouts, window policy.       |
| `<libdir>/libgnoblin-std.so.1`, `.so`           | The same two links for the std library.                                 |

The core is required. The std library is optional: an install without it runs core-only shells, and a shell that needs
it fails to load with a message that names `libgnoblin-std`.

## Language runtimes

A runtime is a library the shell loader opens when you run a script. It never opens one it does not need, so a session that
runs a Rust shell does not load Python.

| Path                                            | Runs                                   | Needs                              |
| ----------------------------------------------- | -------------------------------------- | ---------------------------------- |
| `<libdir>/gnoblin/libgnoblin-runtime-js.so`     | `.js`, `.mjs`, `.ts` and `.mts` shells | Nothing beyond the core            |
| `<libdir>/gnoblin/libgnoblin-runtime-python.so` | `.py` shells                           | The system Python's shared library |

Without a runtime, a script shell is refused with the library's name and the directories searched.
`GNOBLIN_RUNTIME_DIR` names a directory to search first.

## Headers and pkg-config

Development files let you build a shell against the installed libraries from C or any language that links C libraries.

| Path                                 | What it is                                                |
| ------------------------------------ | --------------------------------------------------------- |
| `include/gnoblin/core.h`             | The C declarations of the core library.                   |
| `include/gnoblin/std.h`              | The C declarations of the std library. Includes `core.h`. |
| `<libdir>/pkgconfig/gnoblin-core.pc` | Compile and link flags for the core.                      |
| `<libdir>/pkgconfig/gnoblin-std.pc`  | Flags for std. Requires `gnoblin-core`.                   |

```sh
pkg-config --cflags --libs gnoblin-core
pkg-config --libs gnoblin-std
```

On a prefix that is not on the default path, set `PKG_CONFIG_PATH=<prefix>/<libdir>/pkgconfig`. The `Version:` field of
each file is the library's ABI version, and `pkg-config --atleast-version=1.0 gnoblin-core` checks it.

## Shells and their data

| Path                                   | What it is                                                                                                                                                                                   |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `share/gnoblin/default-shell/shell.ts` | The shell a fresh install runs when you choose none. It is a TypeScript file; a Python or Rust shell is not packaged here.                                                                   |
| `share/gnoblin/shells/`                | Launcher files (`name.toml`) that name an installed shell. Empty on a fresh install.                                                                                                         |
| `<libdir>/gnoblin/shells/`             | The native shell libraries that launcher files name with `module`.                                                                                                                           |
| `share/gnoblin/js/`                    | The TypeScript library, `gnoblin-core` and `gnoblin-std`, as source. Its object modules are written by `cargo xtask dist` for editors; the runtime serves its own.                           |
| `share/gnoblin/types/python/`          | The stub package of the Python runtime: `py.typed`, the stubs of `gnoblin` and `gnoblin.std`, `_gnoblin_core.pyi`. The package itself is embedded in the runtime and installed as no source. |

A shell package installs its library into `<libdir>/gnoblin/shells/` and a launcher file into `share/gnoblin/shells/`.
[Start Gnoblin and choose a shell](starting-gnoblin.md#installed-shells) describes the launcher file and how Gnoblin finds it.

## Session files

| Path                                                                        | What it is                                    |
| --------------------------------------------------------------------------- | --------------------------------------------- |
| `share/wayland-sessions/gnoblin.desktop`                                    | The login-manager entry.                      |
| `lib/systemd/user/gnoblin-session.target`                                   | The systemd user target.                      |
| `share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service` | Starts the portal backend.                    |
| `share/xdg-desktop-portal/portals/gnoblin.portal`, `gnoblin-portals.conf`   | Portal routing.                               |
| `lib/security/pam_gnoblin_keyring.so`, `lib/pam.d/gnoblin-lock`             | PAM module and the lock screen's PAM service. |

See [package repositories](package-repositories.md) for the packages Gnoblin publishes.

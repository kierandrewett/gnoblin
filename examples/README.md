# Examples

One directory per example of the modular framework, numbered as the specification numbers them (design/modular-framework.md,
section 12). Each directory's entry file starts with a comment that says what the shell shows and how to run it.

Run a script as it is:

```sh
gnoblin ./examples/04-js-esm/shell.js
gnoblin ./examples/03-python-std/shell.py
gnoblin ./examples/05-ts-types/shell.ts
```

Build a Rust shell with Cargo and run the library:

```sh
cargo build -p gnoblin-example-minimal-core-shell
gnoblin ./target/debug/libminimal_core_shell.so
```

A std shell (examples 2, 6, 9, 19 and 20) links `libgnoblin-std.so.1`. In a source tree run `cargo xtask stage` first, and put `target/debug` on
`LD_LIBRARY_PATH` for the shell; an installed Gnoblin has the library where the loader looks.

| #   | Directory                                                   | Language                 | Layers    | Run                                                                                                                                                                                      |
| --- | ----------------------------------------------------------- | ------------------------ | --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | [`01-minimal-core-shell`](01-minimal-core-shell/src/lib.rs) | Rust                     | Core      | `gnoblin ./target/debug/libminimal_core_shell.so`                                                                                                                                        |
| 2   | [`02-std-prelude`](02-std-prelude/src/lib.rs)               | Rust                     | Core, std | `gnoblin ./target/debug/libstd_prelude.so`                                                                                                                                               |
| 3   | [`03-python-std`](03-python-std/shell.py)                   | Python                   | Core, std | `gnoblin ./examples/03-python-std/shell.py` (`layouts.py` switches layouts with `Super+1` to `Super+6`)                                                                                  |
| 3   | [`03-typescript-std`](03-typescript-std/shell.ts)           | TypeScript               | Core, std | `gnoblin ./examples/03-typescript-std/shell.ts`                                                                                                                                          |
| 4   | [`04-js-esm`](04-js-esm/shell.js)                           | JavaScript               | Core, std | `gnoblin ./examples/04-js-esm/shell.js`                                                                                                                                                  |
| 5   | [`05-ts-types`](05-ts-types/shell.ts)                       | TypeScript               | Core      | `gnoblin ./examples/05-ts-types/shell.ts`                                                                                                                                                |
| 6   | [`06-rust-layout`](06-rust-layout/src/lib.rs)               | Rust                     | Core, std | `gnoblin ./target/debug/librust_layout.so`                                                                                                                                               |
| 7   | [`07-script-binding`](07-script-binding/shell.ts)           | TypeScript, Python       | Core      | `gnoblin ./examples/07-script-binding/shell.ts` or `shell.py`                                                                                                                            |
| 8   | [`08-no-workspaces`](08-no-workspaces/src/lib.rs)           | Rust                     | Core      | `gnoblin ./target/debug/libno_workspaces.so`                                                                                                                                             |
| 8   | [`08-no-workspaces-py`](08-no-workspaces-py/shell.py)       | Python                   | Core      | `gnoblin ./examples/08-no-workspaces-py/shell.py`                                                                                                                                        |
| 9   | [`09-native-linked`](09-native-linked/src/lib.rs)           | Rust                     | Core, std | `gnoblin ./target/debug/libnative_linked.so`, then read the log: the libraries it maps and the ABI it was built for                                                                      |
| 10  | [`10-hot-reload`](10-hot-reload/src/lib.rs)                 | Rust                     | Core      | `gnoblin --watch ./target/debug/libhot_reload.so`, then `cargo build -p gnoblin-example-hot-reload` after an edit                                                                        |
| 19  | [`19-shader-tint`](19-shader-tint/src/lib.rs)               | Rust                     | Core, std | `gnoblin ./target/debug/libshader_tint.so`, then `<Super>w` logs why a shader is not drawn                                                                                               |
| 19  | [`19-shader-tint-ts`](19-shader-tint-ts/shell.ts)           | TypeScript               | Core, std | `gnoblin ./examples/19-shader-tint-ts/shell.ts`                                                                                                                                          |
| 19  | [`19-shader-tint-py`](19-shader-tint-py/shell.py)           | Python                   | Core, std | `gnoblin ./examples/19-shader-tint-py/shell.py`                                                                                                                                          |
| 20  | [`20-native-object`](20-native-object/src/lib.rs)           | Rust                     | Core, std | `gnoblin ./target/debug/libnative_object.so`, then `<Super>e` types into the entry the shell made                                                                                        |
| 20  | [`20-native-object-ts`](20-native-object-ts/shell.ts)       | TypeScript               | Core, std | `gnoblin ./examples/20-native-object-ts/shell.ts`, then `Super+e` types into the entry the shell made                                                                                    |
| 20  | [`20-native-object-py`](20-native-object-py/shell.py)       | Python                   | Core, std | `gnoblin ./examples/20-native-object-py/shell.py`, then `<Super>e` types into the entry the shell made                                                                                   |
| 21  | [`21-native-model`](21-native-model/src/lib.rs)             | Rust, Python, TypeScript | Core      | `gnoblin ./target/debug/libnative_model.so`, then open and close windows; Python: `gnoblin ./examples/21-native-model-py/shell.py`; TypeScript: `./examples/21-native-model-ts/shell.ts` |
| 22  | [`22-native-ui`](22-native-ui/src/lib.rs)                   | Rust                     | Core, std | `gnoblin ./target/debug/libnative_ui.so`, then type into the entry the shell put over the windows and press Return                                                                       |
| 23  | [`23-native-services`](23-native-services/src/lib.rs)       | Rust, TypeScript, Python | Core      | `gnoblin ./target/debug/libnative_services.so` (or `gnoblin ./examples/23-native-services-ts/shell.ts`), then `<Super>e` asks for the session lock                                       |
| 24  | [`24-native-scene`](24-native-scene/src/lib.rs)             | Rust, TypeScript, Python | Core, std | `gnoblin ./target/debug/libnative_scene.so` (or `gnoblin ./24-native-scene-ts/shell.ts`), then read the view and camera nodes with `gnoblinctl tree`                                     |

Example 19 sets a custom shader. A window draws it unless the compositor's `compositor.user-shaders` setting is switched off, and only on a renderer that draws shaders; see
[custom shaders](../docs/custom-shaders.md).

The numbers 11 to 18 are behaviours, not shells, and run on the examples above:

| #   | Behaviour                                  | What runs it                                                                                                              |
| --- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| 11  | Python and TypeScript hot reload           | edit `03-python-std/shell.py` or `05-ts-types/shell.ts`, or a module either imports, while `gnoblin` runs it              |
| 12  | Library upgrade without recompiling        | the stored shells of `tests/modular/golden` (built once against ABI 1.0) on the libraries of a later minor                |
| 13  | `gnoblin` discovers a user shell           | put `shell.so`, `shell.py`, `shell.js` or `shell.ts` in `~/.config/gnoblin/` and run `gnoblin`                            |
| 14  | A system-installed shell by name           | a launcher file `tiler.toml` in `/usr/share/gnoblin/shells/`, then `gnoblin --shell tiler` (`gnoblin --list-shells`)      |
| 15  | Several shells discovered                  | two shells in one tier are an error that lists them; a shell named `default`, or `[shell] default` in `config.toml`, wins |
| 16  | Clear failure when there is no shell       | `gnoblin` with no shell anywhere says where it looked and starts no compositor                                            |
| 17  | Correct operation without `libgnoblin-std` | examples 1, 4 and 8 on an install with no std library; a shell that imports std is told which library is missing          |
| 18  | Recovery from a failed reload              | build a broken `10-hot-reload` while it runs: the running shell keeps serving and the log gives the reason                |

Which test runs each one in a real session, and its result, is the table in design/modular-framework.md, section 34.2.

# Write a shell in Rust

A Rust shell is a library you build with Cargo and run with `gnoblin`. It links the Gnoblin libraries installed on the
machine, so it carries no copy of the compositor, and Gnoblin can reload it while it runs.

This page builds a first shell, then covers running it, reloading it while windows stay open, and what happens when you
update Gnoblin.

## Create the project

Make a library crate outside the Gnoblin source tree:

```sh
cargo new --lib myshell
cd myshell
```

Replace `Cargo.toml` with:

```toml
[package]
name = "myshell"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
gnoblin-core = { path = "/path/to/gnoblin/crates/gnoblin-core" }
```

Replace the contents of `lib.rs`, in the `src` directory, with a shell that starts a terminal on `Super+Return`:

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    g.log("myshell is running");
    Ok(())
}
```

Build and run it:

```sh
cargo build
gnoblin ./target/debug/libmyshell.so
```

The file name is `lib` plus the crate name with dashes turned to underscores, plus `.so`. The log says `myshell is
running`, and `Super+Return` starts `foot`. Change `"foot"` to your terminal.

This shell uses only the core, which has no window management of its own: windows open at the top-left corner and nothing
arranges them. [Build a desktop](build-a-desktop.md#a-tiling-window-manager-on-the-core-alone) adds a layout.

`<Super>Return` is a key chord: each modifier in angle brackets, then the key name. `<Super><Shift>q` and `<Alt>Tab` are
other examples. See [shortcuts](/guides/shortcuts) for the full syntax.

## What the project needs

- **A recent Rust toolchain.** Use at least the `rust-version` that `crates/gnoblin-core/Cargo.toml` declares. The SDK uses
  the 2024 edition.
- **A C compiler.** The build links a small stub that names `libgnoblin-core.so.1`, so the finished shell asks for the
  installed library instead of containing one.
- **The default `panic = "unwind"`.** A panic inside a callback is caught where the callback meets Gnoblin, logged, and the
  next event is still delivered. A build that sets `panic = "abort"` is refused with an explanation.
- **No `unsafe` code.** The `#[gnoblin::main]` macro writes the one exported symbol the loader looks for,
  `gnoshell_entry_v1`, and your function stays ordinary safe Rust.

The macro takes a function whose first parameter is `&Gnoblin`. The function returns nothing or a `Result` whose error
can be printed. A returned error fails the load, and its message goes to the log.

Everything runs on one thread, the shell thread, so a `Gnoblin` is neither `Send` nor `Sync`. To log or post work from a
thread you start, hold a `Poster`.

## Choose the dependencies

A shell names the crates for the libraries it uses.

| Crate          | Links             | Gives you                                                                        |
| -------------- | ----------------- | -------------------------------------------------------------------------------- |
| `gnoblin-core` | `libgnoblin-core` | Windows, outputs, input claims, programs, timers and the node tree.              |
| `gnoblin-std`  | `libgnoblin-std`  | Workspaces, layouts, clients and the pointer bindings. Needs `gnoblin-core` too. |

A shell with only `gnoblin-core` runs on a machine that has no std library. [Build a desktop](build-a-desktop.md) shows when
to add `gnoblin-std`.

### Where the crates come from

The SDK crates live in the Gnoblin source repository. They are marked `publish = false`, so they are not on
crates.io. Depend on a checkout with a `path` dependency, as above, or with a `git` dependency on the repository. Cargo
builds only the safe wrapper code from them, never the compositor.

The crates are laid out so that their major version equals the ABI major version of the library they wrap: SDK 1 targets
every `libgnoblin-core.so.1`. The ABI major version, the `1` in the library's name, is what decides whether a shell and an
installed library work together.

The SDK checks the library when the shell loads:

- A shell built for a **newer minor version** than the installed library still loads. Calls the older library does not
  have return the error kind `Unsupported`, which you handle like any other `Err`.
- A shell built for a **different major version** does not load. The log says:

    ```text
    incompatible ABI: this shell was built for gnoblin ABI 2.3; the installed core is 1.0
    ```

## Run the shell

```sh
gnoblin ./target/debug/libmyshell.so
gnoblin --watch ./target/debug/libmyshell.so
```

The first form starts a session with your library as the shell. The second reloads it whenever the file is rewritten (see
[Reload while it runs](#reload-while-it-runs)). To run it every time, copy it to `~/.config/gnoblin/shell.so`, or install
it by name: see [Start Gnoblin and choose a shell](starting-gnoblin.md).

By default the shell runs inside the compositor's process, on a thread of its own. A shell that hangs costs only the shell
its own events, but a crash in native code ends the compositor. `gnoblin --isolate` runs the shell in a child process
instead: a crash there restarts the shell and leaves your windows alone.

## Reload while it runs

Run the shell with `--watch`, change the source, and build again:

```sh
gnoblin --watch ./target/debug/libmyshell.so
cargo build                      # in another terminal
```

Gnoblin notices the rewritten file after 100 ms without further changes, loads the new library next to the old one, and
switches over only if the new one starts cleanly. The log shows each step:

```text
reloading /path/target/debug/libmyshell.so (changed: /path/target/debug/libmyshell.so)
retired generation 1 (myshell): 3 registration(s) removed, library NOT dlclosed; 1 of 32 retired shell libraries retained
generation 2 (myshell) active (1 retired, 0 stale calls rejected); ...
```

Other ways to start a reload:

| Way                               | Effect                                                                                        |
| --------------------------------- | --------------------------------------------------------------------------------------------- |
| `gnoblinctl shell reload`         | Reloads now and waits. Exit code 0 prints `reloaded: generation N`; 1 prints the reason.      |
| `gnoblinctl shell status`         | Prints the running generation, reload counts and the last outcome.                            |
| `gnoblinctl shell watch [PATH]`   | Reloads each time `PATH` is rebuilt. `--once` exits after the first reload.                   |
| `cargo xtask watch-shell PROJECT` | From a Gnoblin checkout: rebuilds the project on each source change, so `--watch` reloads it. |
| `g.request_reload()`              | A shell asks to be reloaded. Example 10 binds it to a key.                                    |

Gnoblin never runs `cargo`. A build that fails leaves the old library in place, so the running shell carries on.
`gnoblinctl shell reload` works for a shell running in the compositor's process. An isolated shell reloads with `--watch`.

### What a reload keeps

A reload replaces your code and the things it registered. Everything the compositor owns stays put.

| Kept across a reload                                            | Lost with the old generation                                       |
| --------------------------------------------------------------- | ------------------------------------------------------------------ |
| Windows, clients and their Wayland connections and surfaces     | Variables and statics in your code                                 |
| Nodes you made with `adopt_node`, found again by class and name | Callbacks, chords and timers you registered                        |
| Bytes you stored with `keep_state`                              | Threads your code started (their calls return `Retired`)           |
| Programs you spawn again with the same arguments                | Nodes made with `adopt_ephemeral_node` that you do not adopt again |

Write the entry function so it registers everything every time. Gnoblin removes the old generation's registrations as it
retires, so a reload never leaves two listeners running.

To carry a value across, store it with `keep_state(key, bytes, version)` and read it back with `kept_state(key)`. The
version is yours: a new generation that finds a version it does not understand starts from nothing instead of reading bytes
in the wrong layout. This shell counts how many times it has started:

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

/// The key the count is kept under, and the version of its layout.
const STARTS: &str = "starts";
const LAYOUT: u32 = 1;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    // The bytes the previous generation kept; the first start finds none.
    let before = g
        .kept_state(STARTS)?
        .filter(|(_, version)| *version == LAYOUT)
        .and_then(|(bytes, _)| bytes.try_into().ok())
        .map_or(0, u32::from_le_bytes);
    let starts = before + 1;
    g.keep_state(STARTS, &starts.to_le_bytes(), LAYOUT)?;

    g.log(&format!("counter: this generation is start number {starts}"));
    Ok(())
}
```

Run it, then run `gnoblinctl shell reload` twice. The log reads `start number 1`, `start number 2` and `start number 3`.

For nodes, `g.adopt_node("canvas", "badge")` returns the node of that class and name the previous generation made, or makes
one. A node from `adopt_ephemeral_node` is removed at the next reload unless the new code adopts it again. The
[hot-reload example](build-a-desktop.md#the-examples) uses all of these.

### Limits

- **A failed reload changes nothing.** If the new library cannot load, its `init` fails or panics, or the file is
  truncated, the old generation keeps serving and the reason is logged and shown by `gnoblinctl shell status`:

    ```text
    reload of /path/libmyshell.so failed, still running generation 1: /path/libmyshell.so: cannot load the shell library: file too short
    ```

    Fix the build and the next change tries again. Anything the failed `init` did outside Gnoblin, such as starting a
    program or writing a file, is not undone.

- **Retired libraries stay loaded.** Native code that may still be running is never unloaded. After 32 reloads in one
  session, the next one is refused with `32 retired shell libraries are still mapped (the limit is 32)`. Restart `gnoblin`
  to reload again.

- **The compositor does not reload.** The core library, its ABI version and the backends change when `gnoblin` restarts.

## Link to the installed libraries

A shell links the libraries the way any program does. Check what a built shell asks for:

```sh
readelf -d target/debug/libmyshell.so | grep NEEDED
```

A core-only shell lists `libgnoblin-core.so.1` and no other Gnoblin library. A shell that uses `gnoblin-std` lists
`libgnoblin-std.so.1` and `libgnoblin-core.so.1`.

When Gnoblin is installed in a prefix the dynamic loader does not search, such as `$HOME/gnoblin`, `gnoblin` itself still
finds its libraries. A shell that uses `gnoblin-std` also needs to find `libgnoblin-std.so.1`. Do one of these:

- Set the library path when you start it:

    ```sh
    LD_LIBRARY_PATH=$HOME/gnoblin/lib gnoblin ./target/debug/libmyshell.so
    ```

- Or build the shell with a run path, so it carries the location:

    ```sh
    RUSTFLAGS="-C link-arg=-Wl,-rpath,$HOME/gnoblin/lib" cargo build
    ```

A core-only shell needs neither. Without one of them, a std shell fails to load and the log says
`libgnoblin-std.so.1: cannot open shared object file`.

To find the compile and link flags, use pkg-config. The [install layout](install-layout.md) lists every installed file,
including `gnoblin-core.pc`:

```sh
export PKG_CONFIG_PATH=$HOME/gnoblin/lib/pkgconfig
pkg-config --libs gnoblin-core         # -L/home/you/gnoblin/lib -lgnoblin-core
```

To link a real library at build time instead of the stub, set `GNOBLIN_CORE_LIB_DIR` (and `GNOBLIN_STD_LIB_DIR` for std) to
the directory that holds `libgnoblin-core.so`.

## Update Gnoblin without rebuilding

A compatible update of `libgnoblin-core.so.1` or `libgnoblin-std.so.1` changes what your shell runs on without a rebuild.
The library keeps the same SONAME, your shell asks for it by that name, and the next time `gnoblin` starts it maps the new
file.

Updates fall into three kinds:

- **A minor or patch release** keeps every existing call working and may add new ones. Your shell loads and runs
  unchanged. It can use a new call after you rebuild against the newer SDK.
- **A major release** is a new SONAME, such as `libgnoblin-core.so.2`, that installs beside the old one, so a shell built
  for the old major keeps loading the library it was built for. A shell asked to run on a different major does not load.
- **A newer shell on an older library** loads, and the calls that library lacks return `Unsupported`.

`gnoblin::installed_abi()` returns the version of the core library the shell found, and `gnoblin::ABI_MINOR` is the minor
it was built against. An update takes effect at the next start of `gnoblin`, not at a reload, because the core is the
compositor itself. [ABI compatibility](abi-compatibility.md) explains the version numbers and how to upgrade the
libraries.

## Examples

[Build a desktop](build-a-desktop.md#the-examples) lists the example shells in `examples/`, including a core-only shell, a std
shell, a custom layout and a shell built for hot reload.

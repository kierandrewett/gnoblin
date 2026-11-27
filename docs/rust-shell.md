# Rust shell

A Rust shell is a library you build with Cargo and run with `gnoblin`. It links the Gnoblin libraries installed on the
machine, so it carries no copy of the compositor, and Gnoblin can reload it while it runs.

A shell can be written in [TypeScript](typescript-shell.md), [Python](python-shell.md) or [Rust](rust-shell.md). The three
pages have the same sections, and the three languages reach the same objects: windows, workspaces, outputs, input and the
std library. Pick the language you like. The [shell API](/shell-api/) and the [API reference](/reference) show every member in
each of them.

This page builds a first shell, then covers running it, reloading it while windows stay open, and what happens when you
update Gnoblin. The types and functions you call are in the [API reference](/reference), which lists every class and member of
`gnoblin-core` and `gnoblin-std` as Rust spells it.

## Create a shell {#create-the-project}

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

### The same shell as the other languages

With `gnoblin-std` as a second dependency the library is the whole desktop. This shell does what the first TypeScript and
Python shells do: decorations, a workspace with a master-and-stack layout, rounded corners and two keys. Add
`gnoblin-std = { path = "/path/to/gnoblin/crates/gnoblin-std" }` to `[dependencies]`, then:

```rust
use std::rc::Rc;

use gnoblin_core::objects::{ClientQuery, ModelRoot};
use gnoblin_std::objects::{CornerRadius, Decorations, MasterStackLayout, MasterStackLayoutOptions, Workspace, WorkspaceOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let objects = g.std()?.objects()?;

    Decorations::get(&objects)?.start()?;

    let layout = MasterStackLayout::new_with(
        &objects,
        MasterStackLayoutOptions {
            ratio: Some(0.6),
            gap: Some(12.0),
            ..MasterStackLayoutOptions::default()
        },
    )?;
    let code = Workspace::new_with(
        &objects,
        "code",
        WorkspaceOptions {
            name: Some("Code".to_owned()),
            layout: Some(layout.as_layout()?),
            ..WorkspaceOptions::default()
        },
    )?;
    model.workspaces()?.add(&code)?;

    let clients = model.clients()?;
    clients.style()?.set_corner_radius(&CornerRadius {
        all: Some(12.0),
        ..CornerRadius::default()
    })?;
    let firefox = ClientQuery {
        app_id: Some("firefox".to_owned()),
        ..ClientQuery::default()
    };
    clients.where_(&firefox, None)?.style()?.set_corner_radius(&CornerRadius {
        all: Some(18.0),
        ..CornerRadius::default()
    })?;

    let seat = model.input()?.primary()?;
    let launcher = Rc::clone(&model);
    seat.bind(
        "Super+Return",
        Some(move |_| {
            launcher.spawn(&["foot".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    let closer = Rc::clone(&model);
    seat.bind(
        "Super+q",
        Some(move |_| {
            if let Some(client) = closer.input()?.primary()?.focused_client()? {
                client.close()?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

Like the other shells, the library is the whole configuration: it registers everything every time it starts, and a reload
replaces it.

### Start from the full default

To start from the whole default desktop instead of a few lines, run this once:

```sh
gnoblinctl config init --language rust
```

It writes the project above, with the default desktop as its `src/lib.rs`, to `~/.config/gnoblin/rust-shell`
(`$XDG_CONFIG_HOME/gnoblin/rust-shell` when that is set), and prints the commands to build it and to make `gnoblin` find
the library:

```text
ok  Rust shell project written: /home/me/.config/gnoblin/rust-shell
ok  written: /home/me/.config/gnoblin/rust-shell/Cargo.toml
ok  written: /home/me/.config/gnoblin/rust-shell/src/lib.rs

Next:
  1. Open /home/me/.config/gnoblin/rust-shell/src/lib.rs in your editor.
  2. Build it with `cargo build --manifest-path /home/me/.config/gnoblin/rust-shell/Cargo.toml`.
  3. Run it once, from a terminal, with `gnoblin /home/me/.config/gnoblin/rust-shell/target/debug/libmyshell.so`.
  4. To run it at every start, copy the library into the config directory with `cp /home/me/.config/gnoblin/rust-shell/target/debug/libmyshell.so /home/me/.config/gnoblin/shell.so`, or set `[shell] path = "/home/me/.config/gnoblin/rust-shell/target/debug/libmyshell.so"` in /home/me/.config/gnoblin/config.toml.
```

The `Cargo.toml` it writes depends on the `gnoblin-core` and `gnoblin-std` crates of the Gnoblin repository on GitHub. To
depend on a checkout instead, as in the `path` lines above, add `--sdk /path/to/gnoblin`.

The project is its own Cargo workspace, and `config init` never replaces it. A `Cargo.toml` or a `src/lib.rs` already there
stops it, and `--force` writes the starter over both. The [recipe](/recipes/complete-starter-config) shows `src/lib.rs` in
full, and [`gnoblinctl`](gnoblinctl.md#write-a-starter-config) has the other languages.

## Run it

```sh
gnoblin ./target/debug/libmyshell.so
gnoblin --watch ./target/debug/libmyshell.so
```

The first form starts a session with your library as the shell. The second reloads it whenever the file is rewritten (see
[Reload while it runs](#reload-while-it-runs)).

| To run the shell          | Do this                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------- |
| Once, from a terminal     | `gnoblin ./target/debug/libmyshell.so`                                                |
| At every start            | Copy it to `~/.config/gnoblin/shell.so`, then run `gnoblin`                           |
| By name                   | Copy it to `~/.local/share/gnoblin/shells/tiler.so`, then run `gnoblin --shell tiler` |
| Chosen in a file          | Set `[shell] path = "~/dev/tiler/libtiler.so"` in `~/.config/gnoblin/config.toml`     |
| Chosen by the environment | `GNOBLIN_SHELL=~/dev/tiler/libtiler.so gnoblin`                                       |

`gnoblin --list-shells` shows which shell would run and why. [Start Gnoblin and choose a shell](starting-gnoblin.md) lists
every place Gnoblin looks and the errors it gives. Do not put both a `shell.so` and a `shell.ts` in the config directory: two
equal shells stop the start with a list of both.

### Where the config lives

Gnoblin runs one shell, chosen once at start from seven places in order. For a Rust shell the ones you use are:

1. **The command line:** `gnoblin ./libmyshell.so` runs that file, and `gnoblin --shell tiler` runs the installed shell called
   `tiler`.
2. **`GNOBLIN_SHELL`:** a value with a `/` is a path, anything else is the name of an installed shell.
3. **`config.toml`:** `[shell] path = "~/dev/tiler/libtiler.so"` names a file, and `[shell] default = "tiler"` names an
   installed shell. Set one of them, not both.
4. **A file in the config directory:** `shell.so` in `~/.config/gnoblin` (or `$XDG_CONFIG_HOME/gnoblin`). `shell.ts` and
   `shell.py` are found the same way, for a [TypeScript](typescript-shell.md) and a [Python](python-shell.md) shell.
5. **Installed for your user:** a file such as `tiler.so`, or a launcher file `tiler.toml`, in `~/.local/share/gnoblin/shells/`.
6. **Installed for the system:** the same, in `gnoblin/shells/` under each `XDG_DATA_DIRS` directory.
7. **The packaged default:** `<prefix>/share/gnoblin/default-shell/shell.ts`, the stock config that a fresh install runs.

The first place that gives exactly one shell wins.

By default the shell runs inside the compositor's process, on a thread of its own. A shell that hangs costs only the shell
its own events, but a crash in native code ends the compositor. `gnoblin --isolate` runs the shell in a child process
instead: a crash there restarts the shell and leaves your windows alone.

### What the project needs

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

- **Retired libraries stay loaded.** Native code that may still be running is never unloaded, so each reload keeps one
  library mapped for the life of the session, and so does a replacement that mapped and then failed (its `init` failed or
  panicked). A file that cannot be mapped at all keeps nothing. The bound is 32 libraries. After that the next reload is
  refused with `32 retired shell libraries are still mapped (the limit is 32)`, the shell that serves keeps serving and
  `gnoblinctl shell status` shows `32 of 32 retired shell libraries retained`. Restart `gnoblin` to reload again.

- **The compositor does not reload.** The core library, its ABI version and the backends change when `gnoblin` restarts.

## Types and editor support

Rust is typed by the compiler, so `cargo check` is the type check: every member the SDK crates have is a method with its
types, and a name that does not exist does not compile. An editor with `rust-analyzer` completes the members and shows
their documentation, which is the generated [API reference](/reference).

To check that the installed `gnoblin` loads your built shell, without starting a session:

```sh
gnoblinctl check ./target/debug/libmyshell.so    # ok, or error and the loader's message
```

`check --types` is for `.ts` files, and the SDK crates need no declaration files: Cargo finds them through the `path` or
`git` dependency.

The stubs and `tsconfig.json` that `gnoblinctl types init` writes are for TypeScript. The stub package for Python is
`<prefix>/share/gnoblin/types/python`.

## Imports and libraries

A shell names the crates for the libraries it uses.

| Crate          | Links             | Gives you                                                                        |
| -------------- | ----------------- | -------------------------------------------------------------------------------- |
| `gnoblin-core` | `libgnoblin-core` | Windows, outputs, input claims, programs, timers and the node tree.              |
| `gnoblin-std`  | `libgnoblin-std`  | Workspaces, layouts, clients and the pointer bindings. Needs `gnoblin-core` too. |

A shell with only `gnoblin-core` runs on a machine that has no std library. [Build a desktop](build-a-desktop.md) shows when
to add `gnoblin-std`. The crates take the names `gnoblin_core::objects` (the core's classes) and `gnoblin_std::objects` (the
std classes), and `use gnoblin_std::prelude::*;` brings in the traits that give the core's classes their std members.

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

### Link to the installed libraries

A shell links the libraries the way any program does. Check what a built shell asks for:

```sh
readelf -d target/debug/libmyshell.so | grep NEEDED
```

A core-only shell lists `libgnoblin-core.so.1` and no other Gnoblin library. A shell that uses `gnoblin-std` lists
`libgnoblin-std.so.1` and `libgnoblin-core.so.1`.

`gnoblin` finds both libraries wherever Gnoblin is installed, including a prefix the dynamic loader does not search, such
as `/usr/local` on some distributions or `$HOME/gnoblin`. You do not need to set `LD_LIBRARY_PATH` or build the shell with
a run path.

- The launcher finds the core through its own run path.
- When a shell lists `libgnoblin-std.so.1` among its `NEEDED` libraries, `gnoblin` opens the one beside the core
  (`$GNOBLIN_STD_DIR` first) before it loads the shell. A core-only shell never causes `libgnoblin-std.so.1` to load.
- With no std beside the core and none on the loader's path, the shell is refused with a message that names the library,
  the directories looked in and what to install.

To find the compile and link flags, use pkg-config. The [install layout](install-layout.md) lists every installed file,
including `gnoblin-core.pc`:

```sh
export PKG_CONFIG_PATH=$HOME/gnoblin/lib/pkgconfig
pkg-config --libs gnoblin-core         # -L/home/you/gnoblin/lib -lgnoblin-core
```

To link a real library at build time instead of the stub, set `GNOBLIN_CORE_LIB_DIR` (and `GNOBLIN_STD_LIB_DIR` for std) to
the directory that holds `libgnoblin-core.so`.

### Update Gnoblin without rebuilding

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

### Read the root object of the core

The typed object model of the core starts at `ModelRoot`. Its members are the ones a script reads on `gnoblin`: `timers()` (`after` and
`now`), `version()`, `motion(duration)`, `compositor()` (`user_shaders`, `set_user_shaders` and `reset`), and `find`.

```rust
use gnoblin_core::objects::{Client, ClientQuery, Finds, ModelRoot, Output};

let model = ModelRoot::get(&g.objects()?)?;
let started = model.timers()?.now()?;
println!("gnoblin {}", model.version()?.gnoblin);
let milliseconds = model.motion(200.0)?; // 0 while the seat asks for reduced motion
let foot = model.find::<Client>(Some(&ClientQuery {
    app_id: Some("foot".to_owned()),
    ..ClientQuery::default()
}))?;
let outputs = model.find::<Output>(None)?;
```

`compositor.set_user_shaders(false)?` writes `compositor.user-shaders` to the shell's settings document, `user_shaders()` reads what was
written (else what the running compositor holds), and `reset("user_shaders")` takes it back.

The shell commits nothing itself. After the turn that wrote it, Gnoblin lays the settings the shell wrote over the configuration the
compositor holds and commits the result, so the next login has `compositor.user-shaders` (the setting of a device applies at once). The
rest of the configuration is not touched, and a shell that writes no setting changes nothing.

A setting stays in the configuration until a login writes another value. To go back to the default, write it
(`compositor.set_user_shaders(true)?`), or call `reset("user_shaders")` in the run that wrote it. A reload whose shell does not write a
setting takes it back.

Gnoblin keeps the settings only for the shell the stored configuration belongs to: a shell run by another path (`gnoblin --shell ./dev.so`)
while another shell is installed in the configuration directory commits nothing, and the log says so.

`find` takes the kind as a type. A kind the core does not list has no `Finds<ModelRoot>` impl, so asking for it is an error when the
shell is built and not when it runs. `find` takes a filter record. For a test a record cannot say, `find_matching` takes a closure that
answers `Ok(true)` for each object to keep, as `find(kind, function)` does in TypeScript and Python:

```rust
let drafts = model.find_matching::<Client>(|window| Ok(window.title()?.contains("draft")))?;
```

The closure returns `Result<bool, Fail>`, so `?` works in it, and its first error is the error of the call. `examples/35-native-root` is
this.

### Read the std members of the root object

`use gnoblin_std::prelude::*;` brings in `ModelRootStdExt` and `OutputCollectionStdExt`, which give the core's typed `ModelRoot` the members
the std library adds: `workspaces()`, `displays()`, `scene()`, `gestures()` and `request_transition(..)`, and `profiles()` for the
`outputs()` of the root.

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::Workspace;
use gnoblin_std::prelude::*;

let model = ModelRoot::get(&g.objects()?)?;
let objects = g.std()?.objects()?;
let workspaces = model.workspaces()?;
let main = Workspace::new(&objects, "main")?;
workspaces.add(&main)?;
workspaces.show(&main, None)?; // `show_by_id("main", None)` takes the id; `area()` is where windows are laid out
let displays = model.displays()?; // `replace(&old, &new)` swaps a display of the shell's own for another
let profiles = model.outputs()?.profiles()?;
```

A shell that does not use the std prelude has none of them, and a call of one does not compile: the trait is not in scope, and the
shell's `readelf -d` names `libgnoblin-core.so.1` and no std library. Each member is read through the std library of the generation
the core's object belongs to. `examples/37-native-std-root` is this.

### Make an object with options

`new` takes the properties the catalog names for the constructor. `new_with` takes them and then an options struct named after the
class (`WorkspaceOptions` for `Workspace`).

The struct has an `Option` field for every other writable property, and it implements `Default`.

A `Some` field is sent in the call that makes the object. A `None` field is not sent, so that property keeps the library's default.

```rust
use gnoblin_std::objects::{Gaps, MasterStackLayout, Workspace, WorkspaceOptions};

let main = Workspace::new_with(
    &objects,
    "main",
    WorkspaceOptions {
        name: Some("Main".to_owned()),
        gaps: Some(Gaps { inner: 6.0, outer: 12.0 }),
        layout: Some(MasterStackLayout::new(&objects)?.as_layout()?),
    },
)?;
let half = Workspace::new_with(
    &objects,
    "half",
    WorkspaceOptions { name: Some("Half".to_owned()), ..WorkspaceOptions::default() },
)?;
```

- A value the library refuses fails the call with the library's error, and no object is left.
- A closure property is a boxed closure in the struct: `on_tick: Some(Box::new(|x| Ok(x * 10)))`.
- The struct is `<Class>Options`, or `<Class>CreateOptions` where the catalog already has a type of the first name.
- `examples/38-native-options` is this.

### Name the hooks of a listener

An interface is a struct of handlers, one closure for each method. A method the library has a default for is an `Option`, and a struct
with no method the shell must implement has `Default`, so name the hooks you want and leave the rest:

```rust
let listener = ClientCollectionListenerHandlers {
    on_focus_changed: Some(Box::new(|client, focused| {
        client.set_opacity(if focused { 1.0 } else { 0.95 })?;
        Ok(())
    })),
    ..Default::default()
}
.implement(&objects)?;
```

A struct with a method the shell must implement has no `Default`, because a default would have to invent that closure.

- `compute` of `LayoutHandlers` and `on_request` of `RequestListenerHandlers` are such methods.
- `new` takes the required closures and starts every other hook at `None`:

```rust
let columns = LayoutHandlers {
    set_split: Some(Box::new(|token, ratio, _cross_ratio| {
        println!("divider {token} moved to {ratio}");
        Ok(())
    })),
    ..LayoutHandlers::new(|context| Ok(place_in_columns(context)))
}
.implement(&objects)?;
```

A field is named as its method is in the catalog, so `set_split` is the name in TypeScript, Python and Rust. A handle spells the same
method `move_split`, because `set_split` is the setter of the `split` property of `DwindleLayout`.

### Keep a handle in a closure

Every handle and service is `Clone`. A clone is the same object, held through a reference of its own, so the object lives until the last
clone is dropped, and a closure that outlives the entry function owns a clone and needs no `Rc`:

```rust
let model = ModelRoot::get(&g.objects()?)?;
let closer = model.clone();
model
    .on_client_closed(move |_client| {
        if let Ok(clients) = closer.clients() {
            g.log(&format!("{} windows left", clients.length().unwrap_or(0)));
        }
    })?
    .keep();
```

A clone made after the object ended is a handle of an object that is gone: its calls fail as the original's do. Two clones are equal, and
hash alike, because they hold one object.

### Use `?` in a handler

A handler returns `Result<_, Fail>`. `?` works on a call of the core or of the std library inside a handler:

- A `CoreError` or a `gnoblin_std::Error` converts into `Fail`, with its kind as the code and its text as the message.
- `Fail` is `gnoblin_core::Fail`, and `use gnoblin_std::prelude::*;` brings it in.

```rust
on_added: Some(Box::new(move |client, _clients| {
    let workspaces = g.std()?.workspaces()?; // gnoblin_std::Error
    g.class("window")?; // CoreError
    client.set_opacity(0.95)?; // Fail
    Ok(())
})),
```

A handler that fails is logged as a failed listener, and the next event is delivered.

### Reach the classes of both libraries

`g.objects()?` is the `Objects` of the core and `g.std()?.objects()?` the `Objects` of the std library. They are two types, because a class is
made from the library that serves it. A shell that makes classes of both takes the second from the first, or from `g`:

```rust
let core = g.objects()?;
let std = core.std()?; // the same as g.std_objects()?
```

`core.std()` is `ObjectsStdExt`, and `g.std_objects()` is `GnoblinStdExt`; both are in `gnoblin_std::prelude`.

### Check which class an object is

A property typed as a base class or an interface, such as the `Layout` a workspace holds, may hold a class that extends it. Rust casts in
both directions:

- `as_<base>` goes up, from a class to one it extends.
- `downcast` goes down. Call it on the class you want, with the library's `Objects` and any value that holds an object.

`downcast` returns:

- `Ok(Some(class))` when the object is of that class or of a class that extends it.
- `Ok(None)` when the object is of another class.
- An error when the object is gone.

| Language   | Check                                       |
| ---------- | ------------------------------------------- |
| TypeScript | `layout instanceof StripLayout`             |
| Python     | `isinstance(layout, StripLayout)`           |
| Rust       | `StripLayout::downcast(&objects, &layout)?` |

```rust
use gnoblin_std::objects::{StripLayout, Workspace};

fn focus_next_column(objects: &gnoblin_std::objects::Objects, workspace: &Workspace) -> gnoblin_std::Result<()> {
    if let Some(layout) = workspace.layout()?
        && let Some(strip) = StripLayout::downcast(objects, &layout)?
    {
        strip.focus_right()?;
    }
    Ok(())
}
```

The starter's `Super+Left` and `Super+Right` are this. [Workspaces](shell-api/workspaces#act-on-the-layout-a-workspace-has) shows the whole
binding in the three languages.

### Keyboard focus

A shell on the core alone has no focus policy. The core moves the keyboard when a shell writes it (`Window::focus`), so
`examples/01-minimal-core-shell` calls `window.focus()` for each window it places, and a click moves nothing.

A shell that uses `gnoblin-std` to make workspaces, displays or a window frame gets the stock policy with no line of its own:

- A new window and a click take the keyboard, and a click raises, as the seat's focus settings say. `examples/02-std-prelude`
  is this.
- A shell with a focus policy of its own stops the stock one first, so that the two never answer one click:

```rust
use gnoblin_std::objects::FocusPolicy;

FocusPolicy::get(&objects)?.stop()?;
```

## Use the objects

### Call the compositor's verbs and hear its signals

The tree holds windows and the nodes you make. The compositor also has objects that are not nodes: the session (log out, power
off, the bell), the idle watch and the outputs. These are the `gnoblin` namespace.

A shell reaches them with `call_verb` and `subscribe`. The first call opens a connection of its own to the compositor's owner
socket, so a shell that never uses them pays nothing.

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::schema::gnoblin::{idle, session};
use gnoblin_core::{Gnoblin, ShellValue, Value};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    // Keep the session awake while this shell runs. The answer arrives later, on the shell thread.
    g.call_verb_with(idle::CLASS, idle::method::INHIBIT, &[Value::Text("my-shell".into())], |answer| {
        match &answer.error {
            None => g.log("the session is held awake"),
            Some(code) => g.log(&format!("the compositor refused: {code}")),
        }
    })?;

    // Hear the bell. The payload is in the schema's order; `fields::bell::COUNT` names the position of `count`.
    g.subscribe(session::CLASS, session::signal::BELL, |event| {
        if let Some(ShellValue::Uint(count)) = event.value(session::fields::bell::COUNT) {
            println!("the bell has rung {count} time(s)");
        }
    })?;
    Ok(())
}
```

- `call_verb(class, method, args)` sends the call and does not wait.
- `call_verb_with` also takes a closure that runs once with the answer. `answer.result` is the operation's value as JSON text
  (empty for a bare "done"). `answer.error` is the compositor's error code, such as `denied` or `unavailable`.
- `call_verb_on(object, ...)` names one of several objects of a class, such as an output.

The class, method and signal names are constants generated from the schema, and so are the positions of a signal's fields. A
name that does not exist does not compile.

Arguments are checked against the method's parameters before anything is sent: a wrong list is an `Err` from the call itself. A
call that fails to start never runs its closure. A reload removes the subscriptions of the generation that made them, and a
call that was still waiting for its answer is dropped with it.

Python and TypeScript have the same two calls, generated from the same schema: `verbs.session.on_bell(fn)` and
`verbs.idle.inhibit("holder", on_answer=fn)` in Python, `verbs.session.onBell(fn)` and `verbs.idle.inhibit("holder", answer =>
...)` in TypeScript (`import { verbs } from "gnoblin-core/generated/verbs"`).

### Session, processes, idle and timers

- `model.session()` logs out, suspends, powers off and locks.
- `model.spawn(&[..], None)` starts a program, `model.process()` reports how programs end, and `model.timers()` runs a closure later.
- `model.idle()?.watch(ms, &listener)` runs code when the session has had no input for a while, and `authentication` and `launches`
  watch polkit requests and launches.

See [Session and runtime](/shell-api/session-runtime), [Processes](/shell-api/processes), [Idle watches](/shell-api/idle),
[Authentication prompts](/shell-api/authentication) and [Launch feedback](/shell-api/launch-feedback). [Window policy](/js-window-policy)
and [animation](/js-transitions) show every example in TypeScript, Python and Rust.

### XR

XR scripting is part of `gnoblin-std`, and the Rust types match the TypeScript classes. See
[XR mode](/xr#scripting-xr) for the room, the viewport and the bindings that move and resize a panel.

### Run your own event loop

A default shell never sees the loop: the host calls `dispatch` for it. A shell that already has a loop (tokio, calloop, GLib, an
embedded interpreter) needs the opposite: something to poll beside its own sources. `wake_fd` is that descriptor:

```rust
use std::os::fd::AsFd;

let fd = g.wake_fd()?;
loop {
    // ... poll `fd` together with your own descriptors ...
    if readable(fd.as_fd()) {
        g.dispatch(0)?; // never blocks; runs what is queued and drains the descriptor
    }
}
```

The descriptor is readable when:

- a post or an event is queued, from any thread;
- a timer is due;
- the compositor's link has bytes to read.

It is level triggered and carries no data. A wake can be spurious, and then `dispatch(0)` returns `0`.

It is one descriptor for the whole shell, so a reload's new generation gets the same one. You own the copy you are given and
close it by dropping it. A `Poster` has the same `wake_fd`, for a thread that is not the shell's.

Only the shell thread may call `dispatch`.

### Draw your own interface

`libgnoblin-std` carries a toolkit for the interface a shell draws itself: bars, entries, toasts and your own widgets. A canvas is a
node of the compositor's tree that paints a list of drawing operations. The classes are in `gnoblin_std::objects`:

- `Canvas`, `UiBox`, `Grid`, `Text` and `Icon` for layout and drawing;
- `Entry`, `Clock`, `Bar` and `Toast` for the widgets the library keeps up to date.

A member of `gnoblin_std::objects` that takes a window takes `&gnoblin_core::objects::Client`, the core's own class. Text where a window
is expected is a build error. `examples/36-native-clients` makes these calls:

```rust
decorations.inset_of(&client)?;
menus.show(&client, Some(10.0), Some(10.0))?;
workspace.clients()?.add(&client)?;
```

How the library passes a window back to your shell:

- A window that has closed is still a `Client`, and a member called with it fails with `the client is gone`.
- The workspace classes give the core's `Client`: `members`, `on_added` and a layout's `children`. It is equal to the one
  `ModelRoot::clients()` and `get_by_id` give.
- The frame, menu, policy and shell classes still give a window's id as text.

The library lays out, measures text, encodes the paint list and delivers pointer events and keys. You set properties and hear
signals:

```rust
let entry = Entry::new(&objects)?;
entry.set_width(320.0)?;
entry.set_height(36.0)?;
entry.mount_top()?;
entry.focus()?;
entry.set_on_submit(move |text| {
    g.log(&format!("submitted {text:?}"));
    Ok(())
})?;
```

A canvas stays on screen when you let go of the value that made it. `destroy` ends it and what is under it. A canvas the compositor
ends, because its window closed, reads as it was with `alive` false until you let go.

Text is measured at once by the painter the compositor draws with. A `Text` has its size the moment you set its string.

Three more functions of the toolkit need no canvas, and each is an associated function that takes the `Objects` of the library:

- `Ui::estimate_text(&objects, text, size)` is what a `Text` takes the size of its string to be when it cannot be measured: half the font
  size for each character of the widest line, a line of one and a half font sizes for each line, and a baseline at 1.1 font sizes.
- `Ui::encode_paint_list(&objects, &ops)` is the bytes a canvas's `content` holds for a list of `PaintOp`s. A list with one operation that
  cannot be encoded is refused whole, and the error names it (`draw op 2: ...`).
- `Toast::show(&objects, text, ..)` makes a toast, and `Launcher::open(&objects, spawn)` opens the launcher, or focuses the one that is open.

What only your shell can say is the `Widget` interface (`WidgetHandlers::implement`), given to a canvas as its `behaviour`:

- a custom paint, measure or arrangement, and a handler for pointer events or keys;
- anything you leave out, the library does itself;
- a handler answers `true` to keep an event from the canvas's parent;
- `paint_default` and `arrange_default` answer what the library would have done, so an override can extend it.

Example 22 (`examples/22-native-ui`) puts a text and an entry over the windows, and [UI toolkit](/js-ui) shows each widget in
all three languages.

## Window requests

A client asks the compositor for a change of its window: fullscreen, minimise, maximise, raise, activate, move, resize.
The compositor changes nothing until the shell answers. `WindowState::get(&objects)?.start()?` makes the shell answer every
kind, the same way in every language: [Window requests](typescript-shell.md#window-requests) has the table.

A `RequestListenerHandlers` with an `on_request` closure hears every request first. Call `request.take()` to answer it
yourself (`handle` in the other languages: every object of the Rust SDK already has a `handle` of its own): the built-in
answer does not run, and no later listener hears it.

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{ClientRequestKind, RequestListenerHandlers, WindowState};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let state = Rc::new(WindowState::get(&objects)?);

    let policy = Rc::clone(&state);
    let listener = RequestListenerHandlers {
        on_request: Box::new(move |request| {
            let clients = model.clients()?;
            let Some(client) = clients.get_by_id(&request.client()?)? else {
                return Ok(());
            };
            match request.kind()? {
                ClientRequestKind::Minimize if client.app_id()? == "org.example.Clock" => {
                    request.take()?; // this window never minimises
                }
                ClientRequestKind::Maximize => {
                    request.take()?;
                    policy.enter_fullscreen(&client)?; // answer a maximise as a fullscreen
                }
                _ => {}
            }
            Ok(())
        }),
    }
    .implement(&objects)?;

    state.add_listener(&listener, None)?;
    state.start()?;
    Ok(())
}
```

`add_listener(&listener, Some(&client))` hears only that window's requests. Those listeners are asked first, then the ones
for every window, each in the order it was added. The first one to call `request.take()` wins.

`WindowState` is the policy the built-in answer calls. Its methods return whether they ran: `maximise`, `unmaximise`,
`enter_fullscreen`, `leave_fullscreen`, `minimise`, `unminimise`, `present`, `raise` and `lower`.

## Troubleshooting

### Read why a call failed

A call of a typed object returns `Result<T, Fail>`, where `Fail` is `gnoblin_object::Fail`. You import nothing to match it:

- `fail.code()` is the `ErrorCode`. Its `Display` and `as_str()` give the snake_case word, such as `not_found`.
- `fail.reason()` is `Option<ErrorCode>`, and `None` for a bad argument.
- `fail.target()` is `Option<&str>`, the object and member, such as `Client#7.close`.

```rust
match client.close() {
    Ok(()) => {}
    Err(fail) => g.log(&format!("{}: {}", fail.target().unwrap_or("none"), fail.code())),
}
```

A bad argument has `reason()` of `None` and `code()` of `invalid_argument`.

The `?` operator into `gnoblin_core::Result` keeps the words and drops the code and the target, so match the `Fail` first when you need them.
`examples/39-native-errors` does this for a taken name, a closed window and a bad argument. [Errors](shell-api/errors.md) lists the codes.

### Debugging

`g.log(&str)` writes a line to the compositor's log, and a `println!` shows on the terminal when you run `gnoblin ./libmyshell.so`
from one. Read the log with `gnoblinctl log --follow` (see [Logging](/guides/logging)), and set `GNOBLIN_LOG=debug` to see
each generation's life.

A panic in a callback is caught where the callback meets Gnoblin, logged with its message, and the next event is still
delivered. Build with the default `panic = "unwind"` to get that.

### A failed load

A library that fails to load when the shell starts, or whose entry function returns an error, is a failed start: the loader's
text names the library and the reason, and the session runs without a shell, on the compositor's own window management,
until a working build is in place. Nothing needs the session restarted:

- the guardian logs `the shell failed to start:` and the text, `gnoblinctl shell status` prints `state: failed` with the
  same text, and `gnoblin ./libmyshell.so` shows it on your terminal;
- building again with `gnoblin --watch` running loads the new file, and `gnoblinctl shell reload` loads it on request,
  exiting 1 with the new text while it still fails.

A session with no shell file at all runs the packaged starter, which is a `shell.ts`: see [the TypeScript
shell](typescript-shell.md#with-no-shell-ts) for what it does.

If the shell stops, the compositor keeps running and answers every client. With `gnoblin --isolate` a crash in native code
restarts the shell and leaves your windows alone.

## Examples

[Build a desktop](build-a-desktop.md#the-examples) lists the example shells in `examples/`, including a core-only shell, a std
shell, a custom layout and a shell built for hot reload. Each has a TypeScript, Python and Rust version where the example
shows an object of the model.

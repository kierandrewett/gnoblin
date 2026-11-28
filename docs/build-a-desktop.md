# Build a desktop

A Gnoblin desktop is a compositor plus a shell. The compositor is always the same; the shell decides what the desktop
looks like and does. You can write the shell in TypeScript, Python or Rust (JavaScript runs too), or assemble a desktop
from separate programs.

This page shows how the pieces fit together, builds two small shells in all three languages, lists the example shells, and
then covers assembling a desktop from existing programs.

## How the layers fit together

```text
  your shell     a TypeScript, Python or Rust shell (or JavaScript)
      |
      |  optional
      v
  libgnoblin-std.so.1    workspaces, layouts, window policy, styles, animation
      |
      v
  libgnoblin-core.so.1   the compositor: windows, outputs, input, the node tree, shell loader
```

Gnoblin has two libraries and a shell chooses which it uses:

| Layer | Library           | Required | What it provides                                                                  |
| ----- | ----------------- | -------- | --------------------------------------------------------------------------------- |
| Core  | `libgnoblin-core` | Yes      | Windows, outputs, input claims, programs, timers, the lock card.                  |
| Std   | `libgnoblin-std`  | No       | Workspaces, layouts, move and resize bindings, snapping, stock window management. |

With only the core, windows are placed where your shell puts them and nothing arranges them otherwise: with no shell
placing a window, it opens at the top-left corner. Add std when you want the library to do the arranging.

Choose by what the shell needs:

- **Core only** when your shell is a window manager of its own, such as a tiler, with its own rules for where windows go.
  It runs on an install that has no std library.
- **Core and std** when you want workspaces and ready-made layouts, or to write a layout and let the library place windows
  with it.

An install without `libgnoblin-std` runs core-only shells. A shell that uses std there fails to load, and the message
names the library.

Each language reaches the two layers the same way:

| Language   | Core                                    | Std                                   | Where the file goes                              |
| ---------- | --------------------------------------- | ------------------------------------- | ------------------------------------------------ |
| TypeScript | `import ... from "gnoblin-core"`        | `import ... from "gnoblin-std"`       | `~/.config/gnoblin/shell.ts`                     |
| Python     | `from gnoblin.objects.model import ...` | `from gnoblin.objects.std import ...` | `~/.config/gnoblin/shell.py`                     |
| Rust       | the `gnoblin-core` crate                | the `gnoblin-std` crate               | a library crate: `gnoblin ./target/debug/lib.so` |

## A tiling window manager on the core alone

This shell gives every window an equal column of the screen, in the order the windows opened. It needs no workspaces and no
library other than the core.

A TypeScript or Python shell is one file that you save under the config directory. For Rust, make the project as described
in [Rust shell](rust-shell.md#create-the-project), then put the Rust file in `lib.rs`, in the `src` directory.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

const GAP = 10;

// Gives every window an equal column of the first output, in the order the windows opened.
function retile(): void {
    const output = gnoblin.outputs.at(0);
    if (!output) return;
    const windows = gnoblin.clients.toList().filter((client) => client.alive);
    const width = (output.width - GAP) / Math.max(windows.length, 1) - GAP;
    windows.forEach((client, index) => {
        client.bounds = {
            x: output.x + GAP + index * (width + GAP),
            y: output.y + GAP,
            width,
            height: output.height - 2 * GAP,
        };
    });
}

retile();
gnoblin.onClientCreated(retile);
gnoblin.onClientClosed(retile);

gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin, Rect

gnoblin = Gnoblin.get()
GAP = 10.0


def retile(*_event: object) -> None:
    """Gives every window an equal column of the first output, in the order the windows opened."""
    output = gnoblin.outputs.at(0)
    if output is None:
        return
    windows = [client for client in gnoblin.clients.to_list() if client.alive]
    width = (output.width - GAP) / max(len(windows), 1) - GAP
    for index, client in enumerate(windows):
        client.bounds = Rect(
            x=output.x + GAP + index * (width + GAP),
            y=output.y + GAP,
            width=width,
            height=output.height - 2 * GAP,
        )


def terminal(event: BindingEvent) -> None:
    gnoblin.spawn(["foot"])


retile()
gnoblin.on("client_created", retile)
gnoblin.on("client_closed", retile)
gnoblin.input.primary.bind("Super+Return", terminal)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::{Gnoblin, Rect};

const GAP: f64 = 10.0;

/// Gives every window an equal column of the first output, in the order the windows opened.
fn retile(model: &Gnoblin) -> gnoblin::Res<()> {
    let Some(output) = model.outputs()?.at(0)? else {
        return Ok(());
    };
    let clients = model.clients()?;
    let mut windows = Vec::new();
    for client in clients.to_list()? {
        if client.alive()? {
            windows.push(client);
        }
    }
    let (x, y, screen_width, screen_height) = (output.x()?, output.y()?, output.width()?, output.height()?);
    let width = (screen_width - GAP) / windows.len().max(1) as f64 - GAP;
    for (index, client) in windows.iter().enumerate() {
        client.set_bounds(&Rect {
            x: x + GAP + index as f64 * (width + GAP),
            y: y + GAP,
            width,
            height: screen_height - 2.0 * GAP,
        })?;
    }
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    retile(&model)?;

    let created = model.clone();
    model
        .on_client_created(move |_client, _clients| {
            if let Err(error) = retile(&created) {
                g.log(&format!("cannot tile the windows: {error}"));
            }
        })?
        .keep();
    let closed = model.clone();
    model
        .on_client_closed(move |_client| {
            if let Err(error) = retile(&closed) {
                g.log(&format!("cannot tile the windows: {error}"));
            }
        })?
        .keep();

    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn(&["foot".to_owned()], None) {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;
    Ok(())
}
```

:::

Run it with `gnoblin ./shell.ts`, `gnoblin ./shell.py`, or `cargo build` and `gnoblin ./target/debug/libmyshell.so`. Press
`Super+Return` twice: the two terminals sit side by side, and closing one gives the other the whole width.

How it works:

- The collection of windows (`gnoblin.clients`) lists the windows of the session, including ones open before the shell started.
  A window that has closed stays readable for a moment, so the shell keeps only the windows that are `alive`.
- Writing `bounds` places a window. A rectangle is `x`, `y`, `width` and `height` in pixels.
- The `client_created` and `client_closed` signals call your function when a window opens or closes: `onClientCreated` and
  `onClientClosed` in TypeScript, `on("client_created", ...)` in Python and `on_client_created(..)` in Rust.
- An output has `x`, `y`, `width` and `height`: its place and size in the compositor's coordinates.

The [`08-no-workspaces`](#the-examples) example takes this further with a master-and-stack layout and a key that chooses the
master window, in Rust and in Python.

## A shell that uses std

Add the std library to the shell. A TypeScript shell imports `gnoblin-std`, a Python shell imports `gnoblin.objects.std`, and
a Rust shell adds `gnoblin-std` to `Cargo.toml` next to `gnoblin-core`:

```toml
[dependencies]
gnoblin-core = { path = "/path/to/gnoblin/crates/gnoblin-core" }
gnoblin-std = { path = "/path/to/gnoblin/crates/gnoblin-std" }
```

Then use the library's workspaces and layouts instead of placing windows yourself:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { MasterStackLayout, Workspace } from "gnoblin-std";

// One workspace; every window that opens joins it and the library tiles it.
gnoblin.workspaces.add(
    new Workspace("main", {
        name: "Main",
        layout: new MasterStackLayout({ ratio: 0.6 }),
        gaps: { inner: 12, outer: 20 },
    }),
);

gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin
from gnoblin.objects.std import Gaps, MasterStackLayout, Workspace, Workspaces

gnoblin = Gnoblin.get()

# One workspace; every window that opens joins it and the library tiles it.
Workspaces.get().add(
    Workspace(
        "main",
        name="Main",
        layout=MasterStackLayout(ratio=0.6),
        gaps=Gaps(inner=12.0, outer=20.0),
    )
)


def terminal(event: BindingEvent) -> None:
    gnoblin.spawn(["foot"])


gnoblin.input.primary.bind("Super+Return", terminal)
```

```rust
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(gnoblin: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = gnoblin.objects()?.std()?;
    // One workspace; every window that opens joins it and the library tiles it.
    let layout = MasterStackLayout::new_with(
        &objects,
        MasterStackLayoutOptions {
            ratio: Some(0.6),
            ..MasterStackLayoutOptions::default()
        },
    )?;
    let main = Workspace::new_with(
        &objects,
        "main",
        WorkspaceOptions {
            name: Some("Main".to_owned()),
            layout: Some(layout.as_layout()?),
            gaps: Some(Gaps {
                inner: 12.0,
                outer: 20.0,
            }),
            ..WorkspaceOptions::default()
        },
    )?;
    Workspaces::get(&objects)?.add(&main)?;

    let shell = gnoblin.clone();
    gnoblin.bind("Super+Return", move || {
        let argv = ["foot".to_owned()];
        if let Err(error) = shell.spawn(&argv, None) {
            shell.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    gnoblin.log("std shell is running");
    Ok(())
}
```

:::

| Setting                        | Accepted values                                                                                                                                                       | Effect                                                       |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| The id of a `Workspace`        | A non-empty string                                                                                                                                                    | Names the workspace. The id is also its name by default.     |
| `name`                         | Any string                                                                                                                                                            | The name people see.                                         |
| `layout`                       | `FloatingLayout` (default), `MasterStackLayout`, `GridLayout`, `MonocleLayout`, `DwindleLayout`, `StripLayout`, `ArcLayout`, `Carousel3dLayout`, or your own `Layout` | How windows are arranged.                                    |
| `gaps`                         | Two numbers of at least 0, in pixels: `inner`, then `outer`                                                                                                           | Space between windows, then between windows and the edge.    |
| `ratio` of `MasterStackLayout` | 0.1 to 0.9                                                                                                                                                            | The share of the width the master window takes. Default 0.5. |

In Rust the workspace is made with `Workspace::builder(id)`, and `.gaps(inner, outer)` takes the two numbers; the
object-model spelling `Workspace::new_with(&objects, id, WorkspaceOptions { .. })` is the same call the TypeScript and Python
constructors make, as [Rust shell](rust-shell.md#make-an-object-with-options) shows.

Windows that open join the workspace and the library tiles them. A Rust shell links `libgnoblin-std.so.1`, which links the
core, and a session loads one copy of each. If Gnoblin is installed outside the system library path, give the shell a
library path: see [Link to the installed libraries](rust-shell.md#link-to-the-installed-libraries).

To write a layout yourself, implement the `Layout` interface in your language: the library calls `compute` whenever the
workspace changes and applies the rectangles you return. The [`06-rust-layout`](#the-examples) example does this in Rust, and
[`03-typescript-std`](#the-examples) and [`03-python-std`](#the-examples) do it in TypeScript and Python.

## The examples

The `examples/` directory of the source tree holds the example shells. Each is a project you can build and run as it
stands. Rust ones build with `cargo build -p <package>` from the source tree; scripts run directly with
`gnoblin ./examples/<directory>/shell.ts` (or `shell.py`, `shell.js`).

| Directory               | Language                 | Layers    | What it shows                                                                                        |
| ----------------------- | ------------------------ | --------- | ---------------------------------------------------------------------------------------------------- |
| `01-minimal-core-shell` | Rust                     | Core      | Places every window in a cascade and gives it the keyboard, starts a terminal on a chord, closes it. |
| `02-std-prelude`        | Rust                     | Core, std | One workspace per output, click focus from the library, chords that step through workspaces.         |
| `03-python-std`         | Python                   | Core, std | A workspace with a layout of your own, a listener on windows, a chord.                               |
| `03-typescript-std`     | TypeScript               | Core, std | The same shell in TypeScript.                                                                        |
| `04-js-esm`             | JavaScript               | Core, std | ES modules: imports `gnoblin-core` and `gnoblin-std`, and a relative module.                         |
| `05-ts-types`           | TypeScript               | Core, std | Three files with types; an error points at the `.ts` line that wrote it.                             |
| `06-rust-layout`        | Rust                     | Core, std | A `Layout` written in Rust: equal columns, and a chord that swaps two windows.                       |
| `07-script-binding`     | TypeScript, Python       | Core      | A key binding written entirely in a script that moves the focused window.                            |
| `08-no-workspaces`      | Rust, Python             | Core      | A master-and-stack tiler of raw windows with no workspaces and no scene graph.                       |
| `09-native-linked`      | Rust                     | Core, std | Links the installed libraries and says which ones its process maps and which ABI each has.           |
| `10-hot-reload`         | Rust                     | Core      | A shell built to be rebuilt while it runs: kept state, adopted nodes, no duplicate listeners.        |
| `19-shader-tint`        | Rust, TypeScript, Python | Core, std | A shader filter that tints every window, and a chord that says why one is not drawn.                 |
| `20-native-object`      | Rust, TypeScript, Python | Core, std | The typed object API of std: an entry model edited and heard, a focus decision.                      |
| `22-native-ui`          | Rust                     | Core, std | The toolkit of std: a text and a focused entry over the windows; Return is heard with what was typed |

The numbers also name behaviours that are not shells of their own:

- **Reload of a script (11):** edit `03-python-std/shell.py` or `03-typescript-std/shell.ts`, or a module it imports, and
  the shell reloads in place. See the [Python shell](python-shell.md#reload-while-it-runs), the
  [TypeScript shell](typescript-shell.md#reload-while-it-runs) and the [Rust shell](rust-shell.md#reload-while-it-runs).
- **Linking the installed libraries (9):** [`09-native-linked`](#the-examples) logs the libraries its process maps and the ABI
  it was built for and finds installed. Any Rust shell that uses `gnoblin-std` links the same way. See
  [Rust shell](rust-shell.md#link-to-the-installed-libraries).
- **Update without rebuilding (12):** a shell built for one version of the libraries keeps running when they are upgraded.
  See [ABI compatibility](abi-compatibility.md).
- **Discovery and failures (13 to 16):** how `gnoblin` finds a shell, picks one by name, lists several and fails with none.
  See [Start Gnoblin and choose a shell](starting-gnoblin.md).
- **Core only (17):** a shell on an install with no std library runs; one that needs std says which library is missing.
- **Failed reload (18):** a replacement that cannot load leaves the running shell serving. See
  [Rust shell](rust-shell.md#limits).

## Assemble a desktop from existing programs

Gnoblin supplies the compositor, window management and interfaces that desktop
components can use. A shell supplies the visible controls. The shell can be one
program or a collection of smaller clients; it is independent of Gnoblin.
Bingux is one separate project built on these interfaces.

| Piece            | Gnoblin provides                                       | Your desktop chooses                                           |
| ---------------- | ------------------------------------------------------ | -------------------------------------------------------------- |
| Windows          | Focus, move, resize, workspaces, rules and decorations | Dock grouping, switcher order and menus                        |
| Input            | Keybindings and input-source control                   | Launcher, search and popup design                              |
| Surfaces         | Layer-shell placement, exclusive zones and effects     | Bar, dock, wallpaper and overlays                              |
| Desktop services | Portal routing, and optional native features           | Portal backend, notification daemon and other visible controls |

![Firefox in a Gnoblin session with the Bingux dock](images/gnoblin-bingux-firefox.png)

_Bingux is one separate shell example; Gnoblin provides the compositor and window management._

The [configuration reference](/config/configure) describes the
settings. The [CLI](cli/index.md) is convenient for commands; the
[compositor bridge](bridge/index.md) supplies subscriptions and shortcuts
to a long-running shell. [Wayland protocols](wayland-protocols.md) serve native
clients, and [event handlers](user-scripts.md) react to compositor events.

### Assemble a small desktop

Install a bar, launcher, notification daemon and terminal. This example uses
Waybar, Fuzzel, Mako and Foot. They are independent applications; replace any
of them with a component you prefer. Put this in a **new** shell file:
`~/.config/gnoblin/shell.ts` for TypeScript, `~/.config/gnoblin/shell.py` for Python, or the `lib.rs` of a Rust
shell library.

::: tabs

```typescript
gnoblin.spawn(["waybar"]);
gnoblin.spawn(["mako"]);

const input = gnoblin.input.primary;
input.bind("Super+d", () => {
    gnoblin.spawn(["fuzzel"]);
});
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin

gnoblin = Gnoblin.get()

gnoblin.spawn(["waybar"])
gnoblin.spawn(["mako"])


def launcher(event: BindingEvent) -> None:
    gnoblin.spawn(["fuzzel"])


def terminal(event: BindingEvent) -> None:
    gnoblin.spawn(["foot"])


seat = gnoblin.input.primary
seat.bind("Super+d", launcher)
seat.bind("Super+Return", terminal)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    model.spawn(&["waybar".to_owned()], None)?;
    model.spawn(&["mako".to_owned()], None)?;

    let seat = model.input()?.primary()?;
    for (trigger, program) in [("Super+d", "fuzzel"), ("Super+Return", "foot")] {
        let shell = model.clone();
        seat.bind(
            trigger,
            Some(move |_| {
                shell.spawn(&[program.to_owned()], None)?;
                Ok(true)
            }),
            None,
        )?;
    }
    Ok(())
}
```

:::

![Settings app open under Waybar](images/gnoblin-waybar-settings.png)

_A settings app beneath Waybar._

The cursor uses your system's default theme.

A shell or package ships its settings as a module of the shell's own language (a TypeScript module that your `shell.ts`
imports, a Python module beside your `shell.py`, or a crate your Rust shell depends on); there are no system drop-in
directories.

Log in to Gnoblin. The active file is the shell file in `~/.config/gnoblin`. Run
`gnoblinctl config reload` to apply edits; Gnoblin reports any
configuration errors.

Waybar's Sway and Hyprland modules expect those compositors' own IPC and do
not gain that IPC merely by running under Gnoblin. Configure supported generic
modules or write a module using [window data](bridge/session-and-windows.md#read-snapshots).

### Give each visible function an owner

Start one notification daemon from your shell or the session. Configure window
switchers and keyboard-layout popups in your shell. Gnoblin's input API selects
the active input source.
Use [shortcut guidance](/guides/shortcuts#avoid-conflicts) to resolve binding
conflicts.

A dock can use the foreign toplevel protocols for basic window handles or the
bridge for records, previews, activation and shortcut sessions. Keep window IDs
only for the lifetime of their windows. Refresh snapshots after windows close
or workspaces change. [Bridge examples](bridge/examples.md) show clients in
several languages.

### Choose the right interface

| Task                                 | Interface                                                                    |
| ------------------------------------ | ---------------------------------------------------------------------------- |
| Change a window from a script        | `gnoblinctl window ...`                                                      |
| Maintain a live switcher or dock     | Compositor bridge `windows` subscription                                     |
| Place a bar or dock                  | `zwlr_layer_shell_v1`                                                        |
| Capture output with a native client  | `zwlr_screencopy_manager_v1`, subject to its protocol gate                   |
| Apply per-app styling                | The `style` of `clients.where(...)` in your shell: [window style](/js-style) |
| React to a window or workspace event | A listener in your shell (see [event handlers](/user-scripts))               |

The desktop portal backend decides what each app may do. Disabling a Wayland
protocol does not change what a portal backend allows for screen sharing or
remote control; see [portal routing](/config/configure/portals).

### Work on the desktop in a nested session

`GNOBLIN_BACKEND=nested gnoblin` starts Gnoblin in a window of your desktop (see
[run on a desktop](backends.md#run-on-a-desktop)). Use it to try a bar or rule
before logging out.

For screenshots and demos, give it fresh home and XDG config, data, cache,
state, and runtime directories. Launch only the applications that belong in
the example.

For packaging or a final session check, use a real Gnoblin login. A nested
image proves what appeared in that nested run, not what is installed in a
login session.

### Continue from here

- [Window rules](/guides/window_rules) and [effects](/guides/window_effects)
- [Shell integration](shell-integration.md), [window menus](/guides/window_menu) and [snapping](/guides/window_snapping)
- [Protocol catalog](wayland-protocols.md) and [compositor bridge](bridge/index.md)
- [Troubleshooting](troubleshooting.md) when a component does not appear

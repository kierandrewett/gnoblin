# Architecture

This file is a map of the source tree: what each part does, where to find it, and the rules between the parts. It
describes things that rarely change. How a part works is in that crate's own documentation (`//!` at the top of its
`lib.rs` and modules); what the user sees is in `docs/`.

## The problem

Gnoblin is a Wayland compositor that does not decide what the desktop looks like. The shell does. A shell is a module
written in Rust, Python, JavaScript or TypeScript that the compositor loads into its own process, and it may use as much
or as little of Gnoblin's desktop library as it wants.

Three layers, the dependency arrow always pointing down:

```text
myshell.so / shell.py / shell.ts      the user's shell
        |
libgnoblin-std.so.1                   optional: clients, workspaces, layouts, bindings, animation, style
        |
libgnoblin-core.so.1                  required: Wayland server, backends, renderers, the tree, the shell host
        |
gnoblin                               a thin launcher that links the core
```

A shell that uses only the core never maps the std library.

## Codemap

### The launcher and the libraries

- `crates/gnoblin`: the `gnoblin` executable. It runs shell discovery and dispatches the roles (guardian, compositor,
  shell, services). It links `libgnoblin-core.so.1` and does not contain the compositor.
- `crates/gnoblin-core-impl`: `libgnoblin-core.so.1`. It implements the C ABI, the in-process shell link and
  `gnoblin_main`, and owns the SONAME and the link flags.
- `crates/gnoblin-std-impl`: `libgnoblin-std.so.1`, the desktop object model above the core. It registers the stock
  providers (window manager, portal dialogs, motion defaults) when a shell first uses it.
- `crates/gnoblin-core-abi`, `crates/gnoblin-std-abi`: the C ABI of each library: `repr(C)` types, error codes, the
  versioned function table, the export list and the `abi.lock` that CI checks.
- `crates/gnoblin-shell-host`: the language-neutral shell host: generation contexts and the registrations they own, the
  dispatch loop, the native module loader and its retire list, reload staging and strikes.
- `crates/gnoblin-shell-discovery`: which shell to run. The seven tiers, installed shell entries, `--list-shells` and
  the error text when nothing or too much is found.

### The SDKs and language runtimes

- `crates/gnoblin-core`, `crates/gnoblin-std`, `crates/gnoblin-macros`: the Rust SDK. Safe wrappers and
  `#[gnoblin::main]`; they link the installed libraries through a stub and never contain a copy of them.
- `crates/gnoblin-runtime-js`, `crates/gnoblin-runtime-python`: `libgnoblin-runtime-js.so` and
  `libgnoblin-runtime-python.so`. Each is a native module on the same ABI that runs script shells.
- `crates/gnoblin-js`: the QuickJS engine behind the JavaScript runtime: TypeScript stripping (oxc), the module loader,
  the bytecode cache, budgets, the console and hot reload of a generation.
- `crates/gnoblin-object`, `crates/gnoblin-std-dyn`, `crates/gnoblin-provider-wire`: the object protocol, the dynamic
  call layer over the std table, and the provider vtables, each written once for every language.
- `crates/gnoblin-sdkgen`: generates every language binding and its editor files from the schema catalog at build time.
  Nothing it writes is committed.
- `crates/gnoblin-schema`: the classes, properties, methods, signals and error codes of the tree, with the version that
  added each member. The ABI is addressed by these names.
- `data/js`: the hand-written TypeScript library (`gnoblin-core`, `gnoblin-std`) installed as `share/gnoblin/js`.

### The compositor

- `crates/gnoblin-compositor`: the composition root and event loop; the only crate that knows every subsystem.
- `crates/gnoblin-wayland`: the compositor's side of each protocol (placing layer surfaces, sending configures,
  publishing window lists). It runs on Smithay.
- `crates/protocols/*`: one crate per Wayland protocol the server advertises: the global, request decoding and the
  protocol's own rules. `xtask/protocols.toml` lists them and `cargo xtask protocol-crates` writes their manifests.
- `crates/gnoblin-backend`, `crates/gnoblin-backend-drm`: display backends (nested, headless, DRM/KMS).
- `crates/gnoblin-render*`: the scene graph and effects (`gnoblin-render`), the renderer-neutral frame
  (`gnoblin-render-frame`) and the GLES, Vulkan and software renderers. All live inside the core library and are chosen
  at run time.
- `crates/gnoblin-input`, `crates/gnoblin-input-eis`, `crates/gnoblin-output`, `crates/gnoblin-cursor`,
  `crates/gnoblin-capture`, `crates/gnoblin-pipewire-export`, `crates/gnoblin-xwayland`: input, outputs, cursors, screen
  capture and Xwayland.
- `crates/gnoblin-lock`, `crates/gnoblin-lock-auth`: the session lock state machine and its PAM check.
- `crates/gnoblin-wm-api`, `crates/gnoblin-std-wm`: the window-manager provider contract and the stock window manager.

### Around the compositor

- `crates/gnoblin-session`: the guardian: supervision, systemd and logind, environment and recovery.
- `crates/gnoblin-services`, `crates/gnoblin-service-*`: the services role: polkit, keyring, location, privacy,
  ScreenSaver and idle agents on D-Bus.
- `crates/xdg-desktop-portal-gnoblin`, `crates/gnoblin-portal*`, `crates/gnoblin-std-portal`: the portal backend and
  its dialogs.
- `crates/compat/*`: adapters for GNOME, GTK, Qt and X11 interfaces that applications expect.
- `crates/gnoblinctl`, `crates/gnoblin-client`, `crates/gnoblin-proto`: the command-line client and the IPC it speaks.
- `extensions/gnoblin-xr`: the XR program, its own Cargo workspace, run out of process.

### Everything else

- `examples/`: one directory per example shell; an example in several languages keeps them side by side.
- `packaging/`: the files a distribution installs (session entry, PAM service, portal default) and the target list.
- `docs/`: the user documentation, published as the documentation site from `infra/site`.
- `xtask/`: the repository's checks and tools (`cargo xtask`): dependency rules, duplicate types, ABI checks, staging and
  install, docs parity.
- `tests/`: tests that span crates: the modular examples, parity recordings, wlcs and the script tests.
- `scripts/`: CI and developer scripts.

## Invariants

These hold everywhere. Most are checked by `cargo xtask deps` or a test; breaking one is a design change, not a patch.

- **The arrow points down.** Std depends on the core; nothing in the core names a std crate. A shell may use the core
  alone, and then std is never loaded.
- **One copy of the core per process.** Shells, std and the language runtimes link `libgnoblin-core.so.1` dynamically;
  the SDK links a stub that records the SONAME, so no shell carries the compositor.
- **The ABI only grows within a major.** Function tables are appended, never reordered or retyped; structs start with a
  `size` field; `abi.lock` fails CI on any other change. A shell built for 1.0 runs on 1.x, and a shell built for a newer
  minor degrades function by function on an older core.
- **No Rust type crosses the ABI.** Inputs are borrowed for the call, each side frees what it allocated, and a panic is
  caught at every boundary and returned as an error code.
- **The compositor thread never runs shell code.** Shell callbacks run on the shell thread; a slow or hung shell cannot
  stall a frame.
- **A retired generation cannot call back in.** After a reload the old generation's registrations are removed and its
  calls return `E_RETIRED`. A native module is never `dlclose`d; each reload loads a unique copy.
- **Policy is std's, mechanism is the core's.** The test: can a complete tiling shell be written without it? If yes, it
  belongs in std. The lock card is the exception: it stays in the core so a locked session with no shell still unlocks.
- **No custom Wayland protocols.** The server speaks standard protocols only; Gnoblin's own surface is the tree over the
  shell ABI and the IPC.
- **Secrets stay secret.** Values the schema marks secret are copied into wiped buffers and never appear in logs, errors
  or traces.
- **Generated code is not committed.** Bindings, declarations and protocol manifests are rendered from their sources at
  build time or by `cargo xtask`.

## Cross-cutting concerns

- **Discovery and startup.** `gnoblin`, `gnoblin ./shell.so` and `gnoblin --shell NAME` all reach the same loader; only the
  tier that answered differs. The loader picks native or a language runtime by file content, then extension.
- **Hot reload.** Every language uses one protocol: stage a new generation, run its `init`, commit or abandon. A failed
  reload keeps the running shell. Compositor-owned state (clients, surfaces, outputs) never lived in the shell and is
  untouched.
- **Logging.** Every crate logs through `gnoblin-log`; `cargo xtask logging` rejects other logging.
- **Testing.** Unit tests sit beside the code. Real-session tests start the compositor headless (`crates/gnoblin/tests`,
  the `modular` target runs every example). `CONTRIBUTING.md` lists the gates.

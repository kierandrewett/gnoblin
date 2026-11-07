# Accessibility for Gnoblin surfaces

Companion to the "Accessibility: `gnoblin-a11y`" section of [rust-compositor.md](rust-compositor.md). It asks one question: how do Gnoblin's own surfaces (portal dialogs, the recovery overlay, any future egui surface) become readable by Orca. The keyboard monitor is covered only where it meets this path. Decisions stay in the main document; this file holds the detail.

Status: proposed. Nothing in this design is implemented. No crate has been added. Approval is needed before any dependency lands (see "Needs Kieran").

Citations are `path:line`, or `egui 0.37.0` and `accesskit_unix 0.24.1` source paths from the crates named. `[INFERENCE]` marks a claim not checked against code or a primary source.

## Today

Gnoblin's own UI is drawn by egui on the CPU. Its accessibility tree is never built, so no assistive technology can see it.

- **The dialog shell never enables AccessKit.** `DialogShell` owns a `Context::default()` (`crates/gnoblin-portal-core/src/ui.rs:82`). Nothing calls `enable_accesskit` in any Gnoblin crate. egui leaves it off by default (`egui 0.37.0` `src/context.rs:451`).
- **No tree is produced.** With the flag off, egui emits no `accesskit_update`. `run_frame` keeps only `output.shapes` (`ui.rs:185-212`) and nothing else from the platform output is read.
- **No AccessKit crate is in the tree.** No `Cargo.toml` under `crates/` names `accesskit` or any AT-SPI crate. The lock resolves `accesskit` 0.25.2 only as a transitive dependency of egui.
- **The spec claims otherwise.** `rust-compositor.md:2558` says the egui tree is "exposed through AccessKit, as D5 requires". The code does not do this. The table at `rust-compositor.md:1237` marks the same claim `[INFERENCE: check on Linux]`. This document is the check.
- **There is no Rust window menu.** The window menu is Lua-side only (`rust-compositor.md:3068`), so it has no egui tree to expose.
- **`gnoblin-a11y` has no AT-SPI code.** It holds the keyboard-monitor grab table and the policy flags (`crates/gnoblin-a11y/src/lib.rs:1-8`). `monitor.rs` implements `org.freedesktop.a11y.KeyboardMonitor` as state only. Its D-Bus server is outside this crate.
- **Two crate names in the spec are stale.** `gnoblin-diag` (`rust-compositor.md:1237`) is not in this branch's `crates/`. The recovery overlay lives in the portal-core dialog shell.

## What egui gives us

egui 0.37.0 can produce an accessibility tree, but only when asked. The egui-side facts below come from the 0.37.0 source.

- **Off by default.** `Context` has an accesskit flag that starts off (`egui src/context.rs:451`).
- **Turned on by `enable_accesskit`.** The method is at `egui src/context.rs:4234`. Once it is on, each frame builds a `TreeUpdate` (`egui src/context.rs:561-575`).
- **Carried in the output.** The update reaches the caller as `PlatformOutput::accesskit_update` (`egui src/context.rs:3197`).
- **Actions come back in.** AT requests, such as a press on a button, arrive as `accesskit::ActionRequest` in the input state (`egui src/input_state/mod.rs:882`).
- **The winit adapter does the wiring.** `egui-winit` connects the tree to a platform adapter (`egui-winit src/lib.rs:220-228`). That adapter is winit-only. Gnoblin has no winit window, so the wiring must be written by hand.
- **The egui dependency is unconditional.** egui depends on `accesskit` 0.25.0 with no feature gate (`egui Cargo.toml:92`). The lock resolves it to 0.25.2. The tree-building code is therefore already compiled in; only the flag decides whether it runs.

So the egui side needs no new crate. Turning the flag on and forwarding the update is enough to produce a tree. The missing part is the bridge from that tree to AT-SPI.

## AccessKit to AT-SPI on Linux

The Linux bridge is `accesskit_unix`. Its source is in the crates.io package for 0.24.1 (`accesskit_unix-0.24.1.crate`, read in this lane).

- **Licence and version.** `accesskit_unix` 0.24.1 is MIT OR Apache-2.0, published 2026-10-25, MSRV 1.87. The lock does not contain it yet.
- **Version match with egui.** It depends on `accesskit` 0.25.2, the version egui 0.37 uses, so the tree holds one `accesskit`, not two. The 0.22.1 notes used a different line and do not apply.
- **Its other dependencies.** `accesskit_atspi_common` 0.21.1 (MIT OR Apache-2.0), `atspi` 0.29 (Apache-2.0 OR MIT), and `zbus` 5.19 with `async-io`.
- **The zbus cost.** The workspace pins `zbus = "6.2.1"` (`Cargo.toml:285`), which the lock resolves to 6.2.1. `accesskit_unix` needs zbus 5.19, and `atspi` 0.29 needs zbus 5.5. Adding the bridge therefore brings a second zbus major and its runtime into the build. This is the largest cost of Option A below.
- **No window needed.** The 0.24.1 adapter has the same method names as 0.22.1: `Adapter::new` (`src/adapter.rs:133`), `set_root_window_bounds` (`:173`), `update_if_active` (`:196`) and `update_window_focus_state` (`:222`). `[INFERENCE]` the parameter types match 0.22.1, where `set_root_window_bounds` is documented as X11 only. The adapter is driven directly; no winit window is involved.
- **The bus.** The adapter reads `AT_SPI_BUS_ADDRESS`. If it is unset, it asks `org.a11y.Bus` for `GetAddress` on the session bus (`accesskit_unix` 0.22.1 `src/bus.rs:42-47`). `[INFERENCE]` 0.24.1 does the same; the code path is shared through `atspi`.
- **The objects.** The adapter registers the application root at `/org/a11y/atspi/accessible/root` and the cache at `/org/a11y/atspi/cache` (0.22.1 `src/object_id.rs:15-16`).
- **Registration.** The application embeds itself with the `org.a11y.atspi.Registry` socket (`atspi-proxies` `socket.rs:17-18`). After that, AT-SPI clients such as Orca walk the tree from the root object.

The compositor process is the AT-SPI application. A dialog that is not an xdg window is still a node under that application's root. Orca sees it as a child of the application, not as a window. `[INFERENCE]` this is how a compositor-drawn surface gets exposed; no primary source covers the compositor case specifically.

## Focus, geometry and keys

- **Focus.** Orca learns focus from AT-SPI events, not from the tree alone. The adapter sends a focus event when `update_window_focus_state` is called and when the tree's focus node changes. The compositor must call it when a dialog gains or loses keyboard focus. `[INFERENCE]` from the adapter's API; event emission not read in this lane.
- **Geometry.** The AT-SPI `Component` interface reports extents in screen coordinates. The egui tree uses surface coordinates. The compositor must translate them using the dialog's placement (`ui.rs` size; `portal.rs:370-377` for placement). `set_root_window_bounds` does not help on Wayland; it is X11 only.
- **Keys.** Keys do not travel through the tree. Orca watches and grabs keys through `org.freedesktop.a11y.KeyboardMonitor`, which the compositor serves (lane `A11yKeyboardMonitor`). A key that a grab takes never reaches the dialog, so the dialog only sees keys that Orca lets through.
- **Interaction.** Actions such as "press" come back through the action handler. They must be turned into egui input on the next frame. Gnoblin's frame loop is one thread, so this needs a queue, not a call from the bus thread.

## Options

### A: accesskit_unix in a Gnoblin bridge crate

Enable the egui flag on each dialog context. Forward `accesskit_update` to an `accesskit_unix` adapter. Map actions back to egui input.

- Pros: standard AccessKit output; the same tree works for any future egui surface; Orca already speaks this protocol.
- Cons: a second zbus major (5.x) beside 6.x; an async D-Bus runtime in the compositor process; extra objects on the a11y bus.
- Risks: the compositor's frame loop and zbus's executor must not block each other; a slow Orca must not stall rendering.

### B: a minimal AT-SPI tree written with zbus

Write only the interfaces Orca needs (Accessible, Component, Action, Text for dialogs) on zbus 6, which the workspace already uses.

- Pros: no second zbus; no dependency from the accessibility stack; we own the whole surface.
- Cons: AT-SPI is a large protocol, and getting the object model, cache and events right is the hard part. We would reimplement what `accesskit_atspi_common` already does. A bug here breaks Orca silently.
- Risks: incomplete interfaces make Orca report wrong roles; this is hard to test without Orca.

### C: do nothing until Newton

Keep the gap. GNOME's Newton is the Wayland-native accessibility effort. Its 2024 announcement described a prototype: Mutter takes client trees over a D-Bus protocol and passes them to Orca as file descriptors.

The announcement lists no explore-all-surfaces and no screen coordinates, and says GNOME Shell is not on Newton. The Codeberg `WaylandNewton/orca` project has a "Basic newton support" commit dated 2026-08-23. `[INFERENCE]` Newton is not a stable protocol we can target; its status is not checked beyond these sources.

Sources: GNOME accessibility blog, 2024-06-18 (blogs.gnome.org/a11y/2024/06/18); Codeberg `WaylandNewton/orca`, commit of 2026-08-23.

- Pros: no code; no dependency.
- Cons: the gap stays. Dialogs stay unreadable, and the spec's D5 claim stays false.

### Recommendation

Choose A for the first working version, in phases. It uses the AccessKit output egui already produces and the Linux bridge AccessKit maintains. Its cost is the second zbus major. Keep C as the fallback if Kieran rejects the dependency; keep B out unless A proves unworkable.

`[INFERENCE]` A is the better fit because it costs one new dependency tree, while B makes us maintain an AT-SPI implementation ourselves. This is judged from the crate sizes and not yet measured by building.

## Crates needed

Each crate is a candidate, not an approved addition. Sizes are the unpacked source in the crates.io package, read in this lane.

| Crate                  | Version | Licence           | Size   | Why                                      |
| ---------------------- | ------- | ----------------- | ------ | ---------------------------------------- |
| accesskit_unix         | 0.24.1  | MIT OR Apache-2.0 | 220 KB | Linux adapter; registers on the a11y bus |
| accesskit              | 0.25.2  | MIT OR Apache-2.0 | 212 KB | Tree types; already in the lock via egui |
| accesskit_atspi_common | 0.21.1  | MIT OR Apache-2.0 | 236 KB | AT-SPI objects built from the tree       |
| atspi                  | 0.29    | Apache-2.0 OR MIT | 80 KB  | AT-SPI proxies and types                 |
| zbus                   | 5.19    | MIT [INFERENCE]   | large  | Second copy; needed by the above only    |

- `[INFERENCE]` `atspi` 0.29 pulls the rest of the AT-SPI stack (`atspi-common`, `atspi-connection`, `atspi-proxies`). The `atspi-common` crate alone is 792 KB unpacked at 0.14.0; the 0.29 family is not measured here.
- `zbus` 5.19 is the cost. Its licence is MIT; its source is not measured here.
- The bridge code itself goes in one new crate, `gnoblin-atspi` (name proposed). It holds the adapter and no egui code. It needs the report and approval the rules require before it is created.

## Phased plan

Each phase is one commit. Each lists its verification. Builds run only on the gate (`~/dev/lanes/a37-gate.sh`), never locally.

1. **Bridge crate and bus test.** Add `gnoblin-atspi` with `accesskit_unix` and a fixed tree of one root and one button. Verify with a private bus: start `dbus-daemon --session`, run the adapter, and call `GetChildren` on `/org/a11y/atspi/accessible/root` from an AT-SPI client. The test passes when the button appears as a child.
2. **Dialog tree.** Enable the egui flag in `DialogShell` and forward `accesskit_update` to the bridge. Verify: the consent dialog's buttons appear as children of the root in the bus test, with their labels.
3. **Actions.** Map AT `DoAction` press requests to egui input, queued for the next frame. Verify: a `DoAction` on the Deny button produces `cancelled` or `denied` on the portal request, as a keyboard press would.
4. **Focus and geometry.** Send focus state when a dialog gains focus, and translate extents to output coordinates. Verify: the `Component.GetExtents` result matches the dialog's placement rectangle on the output.
5. **Keys with Orca.** Confirm that the keyboard monitor's grabs still take keys before the dialog sees them when AT-SPI is active. Verify with a bus test that registers a grab and checks the dialog gets no key.
6. **Review Newton.** Re-check its status before committing to anything beyond phase 5.

## Risks

- **Two zbus majors.** Build time, binary size and a second async runtime. Measure after phase 1.
- **Bus availability.** AT-SPI needs an a11y bus. If none runs, the adapter must fail quietly and leave the dialog working. `[INFERENCE]` the adapter's failure path is not read in this lane.
- **Stale spec claim.** `rust-compositor.md:2558` must not be trusted until phase 2 lands.
- **Frame-loop coupling.** Actions and tree updates cross threads. A deadlock here freezes the dialog; keep the queue bounded.

## Needs Kieran

1. Approve the dependency set above (`accesskit_unix` 0.24.1 and its tree), including the second zbus 5.x copy. The real alternative is option B, which avoids the second zbus but means maintaining an AT-SPI tree ourselves.
2. Choose between A and C for now. C leaves the D5 claim false until Newton lands.
3. Decide whether the Gnoblin session starts the AT-SPI bus launcher, or whether Orca's own session must provide it. `[INFERENCE]` no launcher is in this branch; not checked.
4. Decide whether AT-SPI is always on or behind a Settings toggle. Always-on exposes dialog text to any local client on the bus.
5. Scope of the first phases: dialogs only, as drafted, or also the recovery overlay in the same pass.

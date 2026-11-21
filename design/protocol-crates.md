# Protocol crates

Status: design, lane ProtoCrates. Phase 1 (this note) is decided. Phase 2, the extraction, follows the order in
[Migration order and gates](#migration-order-and-gates). The numbers come from `origin/rust-compositor` at `9d20a9a6d`.

## Status

Landed in this lane (each is a crate under `crates/protocols/` and its module is gone from `gnoblin-wayland`):

- the seam and tooling: `gnoblin-wayland-api` (`ProtocolInfo`, `Registrar`, the host traits `CoreHost`, `WindowHost`,
  `OutputHost`), the aggregate `gnoblin-wayland-protocols` (the `ALL` table), `xtask/protocols.toml`,
  `cargo xtask protocol-crates [--check]`, and the deps rules `protocol-layering` and `protocol-root`;
- `access` moved into the api crate, and `Registrar` carries the roster (`Registrar::filter`);
- the real-client harness is the crate `gnoblin-wayland-testkit` (`crates/protocols/testkit`): the headless session on a socket of its own, the client that binds the globals a test exercises, the stock window manager, and the client halves of the protocols whose events a test reads (tablet, KDE decoration, primary selection, workspaces, frame callbacks). `gnoblin-wayland`'s integration tests and the Xwayland test depend on it as a development dependency;
- the plain-data crate `gnoblin-wayland-types` (see [The types crate](#the-types-crate)), and 53 protocol crates with the shared crate `foreign-toplevel-shared`: `wp-alpha-modifier-v1`, `wp-single-pixel-buffer-v1`, `zwp-pointer-gestures-v1`,
  `zwp-relative-pointer-v1`, `zwp-idle-inhibit-v1`, `xdg-system-bell-v1`, `xdg-toplevel-icon-v1`,
  `xdg-toplevel-tag-v1`, `xdg-dialog-v1`, `wl-output`, `wl-shm`, `zwp-primary-selection-v1`, `wp-content-type-v1`,
  `wp-fifo-v1`, `wp-fractional-scale-v1`, `wp-tearing-control-v1`, `wp-presentation-time`,
  `zwp-keyboard-shortcuts-inhibit-v1`, `wp-viewporter`, `wp-cursor-shape-v1`, `zxdg-output-v1`, `zwp-pointer-constraints-v1`, `ext-data-control-v1`, `zwlr-data-control-v1`, `zwp-input-method-v2`, `ext-background-effect-v1`, `wp-security-context-v1`, `zwlr-gamma-control-v1`, `zwlr-output-power-management-v1`, `xdg-foreign`, `ext-idle-notify-v1`, `zwp-text-input-v3`, `ext-foreign-toplevel-list-v1`, `zwlr-foreign-toplevel-management-v1`, `zwp-virtual-keyboard-v1`, `zwlr-output-management-v1`, `zxdg-decoration-v1`, `xdg-activation-v1`, `zwlr-layer-shell-v1`, `wl-compositor`, `wl-seat`, `xdg-shell`, `zwp-tablet-v2`, `zwp-linux-dmabuf-v1`, `wl-data-device-manager`, `ext-session-lock-v1`, `ext-image-capture-source-v1`, `ext-image-copy-capture-v1`, `zwlr-screencopy-v1`. Their forwarding
  impls are in `crates/gnoblin-wayland/src/glue/`, one file per crate.

Not yet done:

- The capture engine stays in `gnoblin-wayland`: `capture_shared`, `capture_dmabuf` and `capture_views` in `protocols/`, and
  `capture_image_copy`, the rules behind `ext_image_copy_capture_v1`. See [What blocks the rest](#what-blocks-the-rest).
- The view-source modules `gnoblin_view_source_v1` and `capture_views`, which wait for the owner's decision.
- The move of the remaining real-client tests into their crates. The capture tests all drive the pixel path, the lock or the Views, so they stay with the engine.
- Deleting `protocols/mod.rs`, with the last module.

What each needs first is under [What blocks the rest](#what-blocks-the-rest).

The three standard capture protocols are crates. Each holds the wire objects, the global and a host trait, and the glue forwards the trait to the capture engine.

| Crate                         | Glue                                  |
| ----------------------------- | ------------------------------------- |
| `ext-image-capture-source-v1` | `glue/ext_image_capture_source_v1.rs` |
| `ext-image-copy-capture-v1`   | `glue/ext_image_copy_capture_v1.rs`   |
| `zwlr-screencopy-v1`          | `glue/zwlr_screencopy_v1.rs`          |

`ext-image-copy-capture-v1` also holds the session and frame objects that were `image_copy_session`.

Three decisions made while doing the work differ from the text below.

- The table lives in `xtask/protocols.toml`, not in `crates/protocols/`.
- A row carries no module name or access column, because each crate states its own `PROTOCOL`.
- There is no capture shared crate and no `image-copy-session` crate. The capture engine stays in `gnoblin-wayland`, so the only users of the session and frame objects are the engine and `ext-image-copy-capture-v1`. The crate that holds the objects is the one that needs them, and the engine names them from it.

## What blocks the rest

The protocols still in `gnoblin-wayland` name types that live in the compositor, so their crates cannot depend only on the api crate. The blockers, by module:

- `xdg_shell`, `wl_compositor`, `wl_seat` have crates that hold the global, what the protocol advertises and the code that is the protocol's alone (`WlCompositorClientState`, `verify_buffer`, the implicit touch grab). Their handlers stay in `crates/gnoblin-wayland/src/glue/` for good, and that is the decision, not a gap: each answer is the engine's own work (the popup manager and its grabs, interactive move and resize, the commit path, the seat's focus), so a host trait that gave a crate that access would be the whole of `State` behind a trait with one implementor.
- `wl_data_device_manager` and `ext_session_lock_v1` have crates that hold the global, what it advertises and the table the protocol alone reads (`WlDataDeviceManagerSelections`). Their handlers stay in the engine for good: a paste chooses between a client's source, the X11 window manager and the host's clipboard, and a lock needs the frame path, the lock card and the lock's authentication. The lock's engine module is `session_lock`, outside `protocols/`.
- the capture engine: `capture_shared`, `capture_dmabuf`, `capture_views` and `capture_image_copy`. It holds the session table, the last frame of each output, the PipeWire streams, the lock embargo, the frames queued for the renderer and the pictures of the Views, and it reads the composer, the window manager, the backend and `gnoblin_pipewire_export`. It stays in `gnoblin-wayland`, as the lock's engine module does. The three standard capture protocols are crates over it: each asks the engine through a host trait, and the glue forwards. `capture_views` and `gnoblin_view_source_v1` are not touched, and the engine cannot leave until the owner decides how a View is captured, because the session table, the pixel path and `CaptureTables::new` all name the View tables.

Each needs its shared types moved down first, into the types crate below. What has moved so far:

- `zwlr_output_management_v1`: `HeadInfo`, `HeadRequest` and `ModeRequest` now live in the types crate.
- `zxdg_decoration_v1`: `DecorationWanted` and `WindowDecoration` moved from the backend's compose module, which now names them from the types crate.
- `xdg_activation_v1`: `ActivationDenial` moved from the observation module. The token rules are generic over a `Host` that says what a connection has earned and what the clock reads.
- `zwlr_layer_shell_v1`: `LayerBand`, `LayerNodeInfo` and `LayerOverride` moved from the composer. The compositor's `layer_shell` module arranges, configures and draws the surfaces.
- `WindowRequest`: the foreign-toplevel host asks for it directly, and its own `ToplevelAsk` is gone.
- `extension`: it is not a Wayland protocol but the seam compat adapters register through, so it left `protocols/` for a module of its own.
- `zwp_tablet_v2`: `TabletEvent` and the tool, axis and pad types moved from the backend, which re-exports them where they were public. The crate is generic over a `Host` that supplies the seat, the surface under a point, the keyboard focus and the lock.
- `zwp_linux_dmabuf_v1`: the crate holds the global, the table of waiting buffers and the fence blocker. The import, which asks the backend's renderer and the GPU memory ledger, stays in the glue.

The dmabuf global exists only on a session whose renderer imports GPU buffers, so its row is `conditional`. That puts it in `CONDITIONAL` instead of `ALL`, and the advertised-globals test checks that a registry never lists it above the declared version.

### The types crate

Decision (Main, reversible): a second hand-written crate, `gnoblin-wayland-types` (`crates/protocols/types`), holds the
plain-data compositor types that the blocked protocols name. It sits beside the api crate at the bottom of the tier.

- **Data only.** Structs, enums and trivial helpers. Anything that decides something about the compositor stays in
  `gnoblin-wayland`: for example `HeadRequest`'s change to an output profile is `output_config::request_change`, because it
  names `gnoblin-output`.
- **Who depends on it.** `gnoblin-wayland` and the protocol crates that list `needs = ["types"]` in their row.
  A crate elsewhere in the workspace that names a type depends on it directly, as `gnoblin-backend` does.
  There is no second definition and no shim. A caller names the type from the types crate, or `gnoblin-wayland`
  re-exports it at its root where it was already public.
- **The rule.** The `protocol-layering` rule of `xtask deps` lets a protocol crate name the api crate, the types crate and
  shared crates. Beyond those it may name only the foundation crates and the three hubs. The types crate names only
  foundation crates, no crate of the tier and no engine crate.

## Decision

**One crate per protocol.** Every module in `crates/gnoblin-wayland/src/protocols/` that serves a Wayland protocol
becomes its own crate, named `gnoblin-wayland-<protocol>`. Code that two protocols share goes into a small shared crate
that both depend on. `gnoblin-wayland` keeps only compositor glue: the state, the server, the input, frame and output
machinery, and the handlers that forward into the protocol crates.

The owner chose this granularity over one crate per family. The consequences are in
[Consequences of many small crates](#consequences-of-many-small-crates).

The dependency direction is fixed:

```text
gnoblin-wayland -> gnoblin-wayland-protocols -> gnoblin-wayland-<protocol> -> gnoblin-wayland-<shared> -> gnoblin-wayland-api, gnoblin-wayland-types
```

Every protocol crate may also name the foundation crates, as `gnoblin-wayland` does today. No protocol crate names
`gnoblin-wayland`. A protocol crate asks the compositor for what it needs through traits, and `gnoblin-wayland`
implements those traits on its `State`.

## What is there today

`protocols/` holds 56 modules and 13,934 lines, which is 61 per cent of the 22,960 lines in `gnoblin-wayland/src/`.
Smithay serves most of the protocols. This crate serves the rest, and adds its own objects to some of Smithay's.

Two modules are not extracted. Lane ProtoRetire is deleting them by the owner's decision.

- `gnoblin_session_host_v1` and `gnoblin_view_source_v1`: 871 lines, with `wire.rs`, which carries the crate's only `unsafe_code` allow.
- They are not in the inventory and no crate is made for them. This lane does not edit them or the parts of `access.rs`, `state.rs`, `server.rs` and `lib.rs` that name them.
- The root `protocols/gnoblin-blur-fade-v1.xml` was deleted (D24).

Two facts decide the design.

1. **The dispatch glue is already generic.** `dispatch.rs` implements `Dispatch` and `GlobalDispatch` once for `State`, over
   any user data that implements Smithay's `Dispatch2` or `GlobalDispatch2`. No module carries a `delegate_*!` macro.
   A protocol crate implements `Dispatch2<I, D>` for its own user data, for any `D` that meets its host trait,
   and the one blanket impl binds it. That removes the orphan-rule problem for the protocol's own objects.

### Inventory

The columns of the table mean this.

- "State f / m" counts the distinct `State` fields and methods a module uses, including methods it defines on `State`.
- "Access" is the `access::Global` that gates the module.
- `core` means no roster entry, so every client that is not sandboxed sees it.
- `open` means the roster gates it but every client that is not sandboxed sees it.
- `shared` marks a crate that serves no protocol of its own.

| Crate                                                 | Module                                | Lines | Served by             | Access                    | State f / m |
| ----------------------------------------------------- | ------------------------------------- | ----- | --------------------- | ------------------------- | ----------- |
| `gnoblin-wayland` (the engine)                        | `capture_dmabuf`                      | 370   | own code              | stays                     | 2 / 10      |
| `gnoblin-wayland` (the engine)                        | `capture_shared`                      | 1054  | own code              | stays                     | 6 / 19      |
| `gnoblin-wayland` (the engine)                        | `capture_views`                       | 215   | own code              | stays                     | 2 / 7       |
| `gnoblin-wayland-ext-background-effect-v1`            | `ext_background_effect_v1`            | 123   | Smithay + own objects | BackgroundEffect (open)   | 2 / 1       |
| `gnoblin-wayland-ext-data-control-v1`                 | `ext_data_control_v1`                 | 35    | Smithay               | DataControl               | 1 / 0       |
| `gnoblin-wayland-ext-foreign-toplevel-list-v1`        | `ext_foreign_toplevel_list_v1`        | 57    | Smithay               | ForeignToplevelList       | 1 / 0       |
| `gnoblin-wayland-ext-idle-notify-v1`                  | `ext_idle_notify_v1`                  | 355   | own code              | open                      | 4 / 6       |
| `gnoblin-wayland-ext-image-capture-source-v1`         | `ext_image_capture_source_v1`         | 72    | Smithay               | ImageCopyCapture          | 1 / 1       |
| `gnoblin-wayland-ext-image-copy-capture-v1`           | `ext_image_copy_capture_v1`           | 199   | Smithay               | ImageCopyCapture          | 1 / 13      |
| `gnoblin-wayland-ext-session-lock-v1`                 | `ext_session_lock_v1`                 | 805   | Smithay               | core                      | 7 / 31      |
| `gnoblin-wayland-foreign-toplevel-shared`             | `foreign_toplevel_shared`             | 254   | own code              | shared                    | 5 / 5       |
| `gnoblin-wayland-ext-image-copy-capture-v1`           | `image_copy_session`                  | 659   | Smithay + own objects | ImageCopyCapture          | 1 / 5       |
| `gnoblin-wayland-zwlr-layer-shell-v1`                 | `zwlr_layer_shell_v1`                 | 972   | Smithay               | LayerShell                | 8 / 22      |
| `gnoblin-wayland-wl-compositor`                       | `wl_compositor`                       | 165   | Smithay               | core                      | 2 / 7       |
| `gnoblin-wayland-wl-data-device-manager`              | `wl_data_device_manager`              | 356   | Smithay               | core                      | 6 / 6       |
| `gnoblin-wayland-wl-output`                           | `wl_output`                           | 23    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-wl-seat`                             | `wl_seat`                             | 122   | Smithay               | core                      | 1 / 1       |
| `gnoblin-wayland-wl-shm`                              | `wl_shm`                              | 28    | Smithay               | core                      | 1 / 0       |
| `gnoblin-wayland-zwlr-data-control-v1`                | `zwlr_data_control_v1`                | 30    | Smithay               | WlrDataControl            | 1 / 0       |
| `gnoblin-wayland-zwlr-foreign-toplevel-management-v1` | `zwlr_foreign_toplevel_management_v1` | 391   | own code              | ForeignToplevelManagement | 3 / 7       |
| `gnoblin-wayland-zwlr-gamma-control-v1`               | `zwlr_gamma_control_v1`               | 238   | own code              | GammaControl              | 2 / 1       |
| `gnoblin-wayland-zwlr-output-management-v1`           | `zwlr_output_management_v1`           | 614   | own code              | OutputManagement          | 2 / 3       |
| `gnoblin-wayland-zwlr-output-power-management-v1`     | `zwlr_output_power_management_v1`     | 170   | own code              | OutputPower               | 2 / 1       |
| `gnoblin-wayland-zwlr-screencopy-v1`                  | `zwlr_screencopy_v1`                  | 397   | Smithay + own objects | Screencopy                | 2 / 10      |
| `gnoblin-wayland-wp-alpha-modifier-v1`                | `wp_alpha_modifier_v1`                | 38    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-wp-content-type-v1`                  | `wp_content_type_v1`                  | 66    | Smithay               | core                      | 0 / 1       |
| `gnoblin-wayland-wp-cursor-shape-v1`                  | `wp_cursor_shape_v1`                  | 188   | Smithay               | core                      | 7 / 4       |
| `gnoblin-wayland-wp-fifo-v1`                          | `wp_fifo_v1`                          | 77    | Smithay               | core                      | 1 / 0       |
| `gnoblin-wayland-wp-fractional-scale-v1`              | `wp_fractional_scale_v1`              | 186   | Smithay + own objects | core                      | 1 / 1       |
| `gnoblin-wayland-wp-presentation-time`                | `wp_presentation_time`                | 152   | Smithay               | core                      | 2 / 3       |
| `gnoblin-wayland-wp-security-context-v1`              | `wp_security_context_v1`              | 60    | Smithay               | core                      | 1 / 0       |
| `gnoblin-wayland-wp-single-pixel-buffer-v1`           | `wp_single_pixel_buffer_v1`           | 34    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-wp-tearing-control-v1`               | `wp_tearing_control_v1`               | 170   | own code              | core                      | 1 / 1       |
| `gnoblin-wayland-wp-viewporter`                       | `wp_viewporter`                       | 316   | Smithay + own objects | core                      | 0 / 0       |
| `gnoblin-wayland-xdg-activation-v1`                   | `xdg_activation_v1`                   | 380   | Smithay               | core                      | 6 / 6       |
| `gnoblin-wayland-zxdg-decoration-v1`                  | `zxdg_decoration_v1`                  | 227   | Smithay               | core                      | 4 / 3       |
| `gnoblin-wayland-xdg-dialog-v1`                       | `xdg_dialog_v1`                       | 48    | Smithay               | core                      | 0 / 2       |
| `gnoblin-wayland-xdg-foreign`                         | `xdg_foreign`                         | 506   | Smithay + own objects | core                      | 1 / 2       |
| `gnoblin-wayland-zxdg-output-v1`                      | `zxdg_output_v1`                      | 198   | Smithay + own objects | core                      | 1 / 3       |
| `gnoblin-wayland-xdg-shell`                           | `xdg_shell`                           | 268   | Smithay               | core                      | 3 / 19      |
| `gnoblin-wayland-xdg-system-bell-v1`                  | `xdg_system_bell_v1`                  | 46    | Smithay               | core                      | 0 / 2       |
| `gnoblin-wayland-xdg-toplevel-icon-v1`                | `xdg_toplevel_icon_v1`                | 48    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-xdg-toplevel-tag-v1`                 | `xdg_toplevel_tag_v1`                 | 45    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-zwp-idle-inhibit-v1`                 | `zwp_idle_inhibit_v1`                 | 40    | Smithay               | core                      | 0 / 1       |
| `gnoblin-wayland-zwp-input-method-v2`                 | `zwp_input_method_v2`                 | 78    | Smithay               | InputMethod (open)        | 1 / 2       |
| `gnoblin-wayland-zwp-keyboard-shortcuts-inhibit-v1`   | `zwp_keyboard_shortcuts_inhibit_v1`   | 105   | Smithay               | core                      | 3 / 1       |
| `gnoblin-wayland-zwp-linux-dmabuf-v1`                 | `zwp_linux_dmabuf_v1`                 | 281   | Smithay               | core                      | 4 / 2       |
| `gnoblin-wayland-zwp-pointer-constraints-v1`          | `zwp_pointer_constraints_v1`          | 353   | Smithay               | core                      | 6 / 5       |
| `gnoblin-wayland-zwp-pointer-gestures-v1`             | `zwp_pointer_gestures_v1`             | 33    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-zwp-primary-selection-v1`            | `zwp_primary_selection_v1`            | 22    | Smithay               | core                      | 1 / 0       |
| `gnoblin-wayland-zwp-relative-pointer-v1`             | `zwp_relative_pointer_v1`             | 34    | Smithay               | core                      | 0 / 0       |
| `gnoblin-wayland-zwp-tablet-v2`                       | `zwp_tablet_v2`                       | 758   | Smithay + own objects | core                      | 6 / 11      |
| `gnoblin-wayland-zwp-text-input-v3`                   | `zwp_text_input_v3`                   | 295   | Smithay               | core                      | 3 / 0       |
| `gnoblin-wayland-zwp-virtual-keyboard-v1`             | `zwp_virtual_keyboard_v1`             | 307   | own code              | VirtualKeyboard (open)    | 1 / 6       |

`extension.rs` (499 lines) is not in the table. It is the seam a compat adapter registers a redirect through.

- It serves no protocol, and it names compositor types such as `LockAuth`, `TreeHooks` and `CursorSettings`.
- It has left `protocols/` and is `src/extension.rs` of `gnoblin-wayland`.
- `mod.rs` disappears with the last extraction, and `wire.rs` goes with ProtoRetire's deletion.

The inventory yielded 52 crates: 49 protocol crates and 3 shared crates. Two of the shared crates were not made.

- The capture engine was to fold into `gnoblin-wayland-capture-shared`. It stays in `gnoblin-wayland`.
- `image_copy_session.rs` was to be `gnoblin-wayland-image-copy-session`. Its objects are part of `gnoblin-wayland-ext-image-copy-capture-v1`.
- The one shared crate is `foreign-toplevel-shared`.

### Naming rule

The owner fixed the names. A crate is `gnoblin-wayland-` plus the protocol's own name, with underscores turned into hyphens.

- The version suffix stays when the name has one.
- The name is the interface or manager name the module already carries.
- So `zwlr_layer_shell_v1` is `gnoblin-wayland-zwlr-layer-shell-v1`, and `wp_viewporter` is `gnoblin-wayland-wp-viewporter`.
- The api crate and the shared crates take the same prefix. The compositor crate stays `gnoblin-wayland`.

The prefix means `gnoblin-wayland-*` is a family of crates the compositor crate depends on, so a reader sees at once
whose they are. `xtask deps` finds the tier through the table, not through the prefix, so `gnoblin-wayland` itself and
any later crate that happens to share the prefix are not caught by accident.

## Layout

The crates live in the top-level `protocols/` directory, which held only the Gnoblin-owned XML that is now deleted. One
subdirectory per crate, named after the protocol without the prefix:

```text
protocols/
  zwlr-layer-shell-v1/               gnoblin-wayland-zwlr-layer-shell-v1
  ext-foreign-toplevel-list-v1/      gnoblin-wayland-ext-foreign-toplevel-list-v1
  wp-viewporter/                     gnoblin-wayland-wp-viewporter
  foreign-toplevel-shared/           gnoblin-wayland-foreign-toplevel-shared (shared crates)
  api/                               gnoblin-wayland-api: the seam every protocol crate may use
  all/                               gnoblin-wayland-protocols: the aggregate, owns the globals table
  testkit/                           gnoblin-wayland-testkit: dev-only real-client harness
  ...
xtask/protocols.toml                 the table: one row per crate
```

The root `Cargo.toml` lists these members by glob, so a new protocol does not touch it.

- The table sits in `xtask/` next to the other reviewed lists, so the glob matches only directories.
- `gnoblin-wayland` names one dependency, `gnoblin-wayland-protocols`, instead of one per protocol.

### The table and the generator

`xtask/protocols.toml` is the only list. One row per crate:

```toml
[[crate]]
name = "wp-viewporter"                # gnoblin-wayland-wp-viewporter
module = "wp_viewporter"              # the old module, for the migration record
interfaces = [["wp_viewporter", 1]]   # advertised interface and its maximum version
serves = "smithay+own"                # smithay | own | smithay+own | shared
access = "core"                       # core | open | an access::Global variant
needs = []                            # shared crates this one depends on
deps = ["smithay"]                    # foundation and external crates, by workspace name
```

`cargo xtask protocol-crates` reads the table and writes:

- each crate's `Cargo.toml`, which carries nothing the row does not;
- `gnoblin-wayland-protocols/Cargo.toml`;
- `gnoblin-wayland-protocols/src/lib.rs`: `pub use gnoblin_proto_x as x;` per crate, and the `ALL` table.

With `--check` it fails when a crate directory has no row, a row has no directory, or a generated file differs. It
joins the gates that run next to `gen-sdks --check`. The sources of each crate are written by hand. Only manifests and
the aggregate are generated, which is where the churn would otherwise be.

## The seam: `gnoblin-wayland-api`

The api crate holds what two or more protocol crates need from the compositor, and the types the compositor already
shares with them. A need only one protocol has lives in that protocol's own host trait
([below](#host-traits)). The rule keeps the api crate from becoming a copy of `State`: an operation enters it when a
second crate needs it.

| Part       | What it holds                                                                                                                               |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `access`   | `Global`, `ClientRole`, `AccessPolicy`, `Access`, `Registration`, moved from `access.rs` with their unit tests                              |
| `protocol` | `Protocol`, `ProtocolInfo`, `Interface`, `Privilege` and the registry that `register` receives                                              |
| `host`     | the shared host traits, listed below                                                                                                        |
| `types`    | plain types that more than one protocol uses: `WindowRequest`, `ActivationDenial`, `ObservedLayer`, `HeadInfo`, `ModeInfo`, `OutputControl` |

The shared host traits start from the census, most shared first:

| Trait        | Operations                                                                                  | Users            |
| ------------ | ------------------------------------------------------------------------------------------- | ---------------- |
| `CoreHost`   | `display_handle`, `access`, `loop_handle`, `now`, `client_identity`, `publish`              | about 20 modules |
| `WindowHost` | `window_of`, `geometry_of`, `window_request`, `sync_window_role`, `window_flags`            | about 12 modules |
| `OutputHost` | `output_id_of`, `output_rect`, `output_ids`, `output_scale`, `monitor_of`, `damage_outputs` | about 10 modules |
| `SeatHost`   | `set_keyboard_focus`, `clear_pointer_focus`, `focused_window_surface`, `pointer_position`   | about 7 modules  |
| `LockHost`   | `lock_capture`, `lock_is_held`, `capture_allowed`                                           | about 6 modules  |

These are a first cut. Phase 2 may split a trait when its users diverge. `gnoblin-wayland-api` depends on Smithay (the
traits mention `WlSurface` and `DisplayHandle`), on the foundation crates and on `gnoblin-log`, and on no other
workspace crate.

### Host traits

Each protocol crate states what it needs in one trait, `Host`, which refines the shared traits it uses:

```rust
// protocols/zwp-idle-inhibit-v1/src/lib.rs
pub trait Host: 'static {
    /// Records or clears an inhibitor for a surface.
    fn set_idle_inhibited(&mut self, surface: &WlSurface, inhibited: bool);
}
```

`gnoblin-wayland` implements it for `State`, in one forwarding file per crate under `src/glue/`:

```rust
// crates/gnoblin-wayland/src/glue/zwp_idle_inhibit.rs
impl gnoblin_proto_zwp_idle_inhibit::Host for State { /* forwards to the existing State method */ }
impl IdleInhibitHandler for State { /* one call into the crate */ }
```

The `State` fields that hold a protocol's tables stay fields of `State`, typed.

- Several are read on the pointer and frame paths, and a type map would put a hash lookup there.
- The table type moves into its crate, and the host trait hands it out.
- A protocol that needs two fields at once gets them from one accessor, so the borrow checker still sees disjoint borrows.

### Methods that `impl State` today

Many modules add methods to `State` with `impl State { pub fn content_type(..) }`. An inherent impl has to be in the
crate that defines the type, so these cannot move as they are. Each becomes an **extension trait** in its crate, with a
blanket impl over the host trait:

```rust
pub trait ContentTypeExt: Host {
    fn content_type(&self, surface: &WlSurface) -> Kind { /* body unchanged */ }
}
impl<T: Host> ContentTypeExt for T {}
```

Call sites keep the method syntax, and a caller imports the trait. The bodies move without a rewrite, which keeps the
extraction mechanical and the behaviour fixed.

### Orphan rules and Smithay handlers

Three kinds of impl are involved, and each has one answer.

| Impl                                                                                          | Where it can live                                        | Where it goes                                     |
| --------------------------------------------------------------------------------------------- | -------------------------------------------------------- | ------------------------------------------------- |
| `Dispatch2<I, D>` or `GlobalDispatch2<I, D>` on a protocol's own user data                    | any crate: the user data is local and `D` is a parameter | the protocol crate, with `D: Host`                |
| a Smithay handler trait on the state (`IdleInhibitHandler`, `XdgShellHandler`, `SeatHandler`) | only where `State` is defined                            | `gnoblin-wayland/src/glue/`, as a forwarding body |
| `impl Host for State`                                                                         | only where `State` is defined                            | the same glue file                                |

A blanket impl of a Smithay handler for every `D: Host` is rejected by the compiler as an uncovered type parameter.

- A macro the crate exports and the glue invokes would work, but it hides the logic from the tools and from unit tests.
- So the logic is a generic function in the protocol crate, and the glue body is one line.
- The blanket `Dispatch` impls in `dispatch.rs` stay as they are. The request guard, the request tap, the memory ledger and the view-role gate run there, and the protocol crates need none of it.

## Registration and the globals table

Each protocol crate exports one constant and one function:

```rust
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-viewporter",
    interfaces: &[Interface::new("wp_viewporter", 1)],
    access: Privilege::Core,
};

pub fn register<H: Host>(registry: &mut Registry<'_, H>) -> State { .. }
```

`Registrar` carries the display handle and the access roster.

- `register` creates the globals with the filter the `Privilege` implies, and returns the Smithay state or the table struct.
- `State::new` stores that value in its typed field.
- It calls the registers in the order it called the old `global()` functions, so global numbering does not change.

`gnoblin-wayland-protocols` collects the constants:

```rust
pub const ALL: &[&ProtocolInfo] = &[&gnoblin_proto_wp_viewporter::PROTOCOL, ..];
```

The table has three users.

- `Global::is_advertised` keeps reading the configuration. The `Privilege` of each constant replaces the `is_privileged`
  match, so the roster and the table state the same fact once.
- The advertised-globals test in `gnoblin-wayland` starts a session, reads the registry on both sockets and fails when an
  advertised interface or version differs from `ALL`, or a row is advertised where its `Privilege` says it is not.
- ProtoMissing's `tests/protocol_list.rs` compares the live registry with the table in `docs/wayland-protocols.md`. It
  may read `ALL` as well, so the code, the registry and the page agree three ways.

`docs/wayland-protocols.md` stays hand-written prose, and its table is checked against `ALL` by those tests.

wlcs needs no wiring change. `gnoblin-wlcs` links `gnoblin-wayland`, which links the aggregate, so the library the suite
loads advertises the same globals. `scripts/wlcs.sh --gate` is the check, run on the final SHA.

## Privilege policy

The roster in `access` is a security decision, so it stays reviewed in one place. `Global` and `decide` move to
`gnoblin-wayland-api` with their unit tests, and `tests/access.rs`, which drives sessions, stays in `gnoblin-wayland`.

A new privileged protocol adds one `Global` variant in the api crate and names it in its own `PROTOCOL` constant. The
default stays the safe one: a privileged variant is hidden from applications until someone says otherwise. An open
protocol adds no variant. The view-role allow-list (`VIEW_INTERFACES`) and the role gate in `dispatch.rs` still work by
interface name, so no crate names a role.

## Tests

| Kind                                                                                              | Where it runs                                           |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| unit tests of a protocol's logic                                                                  | in its crate, against a mock `Host`, with no compositor |
| real-client tests of one protocol                                                                 | in its crate's `tests/`, against a real `Server`        |
| tests that cross protocols (focus, lock, capture embargo, access) and the advertised-globals test | stay in `gnoblin-wayland/tests/`                        |

A real-client test needs a session, and a session is `gnoblin-wayland`. The crate takes it as a **dev-dependency**.

- Cargo allows the cycle, because a dev-dependency is not part of the normal graph.
- The crate under test is compiled twice, once as the engine's dependency and once as the test target. The tests talk over the socket, so the two copies never meet.
- `xtask deps` exempts development edges from the engine-isolation rule, and the tier rules do the same.

The session harness is the crate `gnoblin-wayland-testkit`, so each protocol crate imports it and none copies it.

- It holds the session, the client, the stock window manager and the client halves of the protocols whose events a test reads.
- A test moves with its protocol when the protocol's name is in its file name and it needs nothing from another protocol.
- Moved so far: the tests of activation, decoration, dmabuf, foreign toplevel, the two KDE protocols, layer shell, the data-device manager, session lock, tablet, virtual pointer and ext-workspace.
- The rest stays in `gnoblin-wayland/tests/`. Some need the harness's `owned` or `nested_host` helpers, and `cursor_shape` names `gnoblin-cursor`, which the `cursor-isolation` rule keeps out of every crate but the pointer's users. Tests that define a `Dispatch` impl for the harness's `App` need that impl in the testkit, because Rust refuses an impl of a foreign trait for a foreign type.

## Gates

`xtask deps` gains one tier, `PROTOCOL`, defined as the crates in `xtask/protocols.toml`.

| Rule                 | Statement                                                                                                                                                            |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `protocol-layering`  | a protocol crate depends on `gnoblin-wayland-api`, the shared crates its row `needs`, foundation crates, the engine hubs and external crates, and nothing else       |
| `protocol-no-engine` | no protocol crate depends on `gnoblin-wayland`, `gnoblin-backend`, another engine crate, or a protocol crate that its row does not list; dev-dependencies are exempt |
| `protocol-root`      | only `gnoblin-wayland` and the aggregate depend on a protocol crate as an ordinary dependency, and the compat adapters that name a type from one                     |
| `protocol-api`       | `gnoblin-wayland-api` depends on no protocol crate                                                                                                                   |
| `protocol-lints`     | every protocol crate keeps `[lints] workspace = true`; none needs an `unsafe_code` exception, because the generated wire code leaves with ProtoRetire's deletion     |

`engine-isolation` keeps its meaning. The edge from `gnoblin-wayland` to `gnoblin-wayland-protocols` is the one engine-to-protocol
edge, and the rule names it as it names the hubs. `pipewire-export-isolation` lets only `gnoblin-wayland` name the export,
because the capture engine that uses it stays in that crate.

`logging` needs no change for the new crates.

- `logging` scans `crates/*/src`, and the protocol crates sit under it.

## Consequences of many small crates

| Effect                               | Size                                                                                 | Mitigation                                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| Workspace members                    | 55 more (52 crates, the api crate, the aggregate, the testkit), on 117 today         | members by glob; manifests generated                                                        |
| `Cargo.lock`                         | 55 more `[[package]]` entries and no new external dependency                         | none needed                                                                                 |
| `cargo metadata` and the xtask graph | a few per cent slower                                                                | measure once; each gate reads the graph once                                                |
| Cold build of the engine             | the same total work, better parallelism: 52 units build beside Smithay               | none needed                                                                                 |
| Incremental build                    | an edit recompiles one protocol, the aggregate and the glue, not the other protocols | keep logic out of the aggregate and the glue                                                |
| An edit to `gnoblin-wayland-api`     | recompiles every protocol crate                                                      | the api crate is small; an operation enters it with a second user                           |
| Test binaries                        | one more link per crate that has real-client tests                                   | one binary per crate; one shared harness                                                    |
| Review                               | a protocol change is a change in one directory                                       | the table and the generator keep manifests uniform                                          |
| Boilerplate                          | 52 manifests and `lib.rs` headers                                                    | manifests generated; the headers carry the doc sentence that `protocols/mod.rs` holds today |

Moving the "what it does here" sentence from `protocols/mod.rs` into each crate's `lib.rs` is the one place the prose
changes. The `dupes` gate is a risk: 52 crates share the `register` shape and the extension-trait pattern. The
generator keeps the boilerplate out of the sources, and a reviewed `dupes-allow.toml` entry is the fallback.

## Migration order and gates

Each step is one small commit, pushed after the gate passes on the exact SHA. A step moves one crate's code, deletes the
old module in the same commit, moves that crate's tests and updates the docs it names.

1. **Scaffolding.** Done: the api crate with `access`, the aggregate crate, the table and its generator, the member glob, the tier rules and the testkit.
2. **Smithay-only protocols with no `State` method.** Done.
3. **Small protocols with host methods.** Done, including linux-dmabuf and the foreign-toplevel pair with their shared crate.
4. **Medium protocols.** Done, including activation, output management, the data-device manager, wl-compositor and wl-seat.
5. **Large protocols.** Done for xdg-shell, layer shell, tablet and session lock. Their handlers that need the engine stay in the glue.
6. **The capture family.** Done for the three protocols; the engine stays.
    - `ext-image-capture-source-v1`, `ext-image-copy-capture-v1` (with the session and frame objects) and `zwlr-screencopy-v1` are crates. Each `register` creates its globals in the order `CaptureTables::new` always did, so global numbering is the same.
    - `CaptureTables::new` calls the three `register` functions, then creates the view-source global as before.
    - `answer_copy` and the queued screencopy frame (`PendingCopy`) are in the screencopy crate, and `capture_dmabuf` names them from it.
    - The engine (`capture_shared`, `capture_dmabuf`, `capture_views`, `capture_image_copy`) and the view-source modules wait for the owner's decision on how a View is captured.
7. **ProtoMissing's protocols.** Done: virtual pointer, the KDE server decoration and appmenu, and ext-workspace landed as crates.
8. **Close-out.** `protocols/` is gone from `gnoblin-wayland`. The docs, the changelog and the crate lists name the final set.

**Gate per step**, on one Agent37 box and the exact pushed SHA:

- `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo xtask gen-sdks --check`, `abi-check`, `dupes --deny`, `deps` and `protocol-crates --check`;
- `cargo test` for each touched crate and for `gnoblin-wayland`.

The final SHA also runs the full `gnoblin-wayland` tests, the advertised-globals test and `scripts/wlcs.sh --gate`.

### Effort

| Step                                                                  | Working days |
| --------------------------------------------------------------------- | ------------ |
| 1 Scaffolding, generator, gates, testkit                              | 4            |
| 2 Seventeen trivial crates, a half hour of edits each, plus gate runs | 3            |
| 3 Fourteen small crates                                               | 6            |
| 4 Thirteen medium crates                                              | 12           |
| 5 Four large crates                                                   | 8            |
| 6 The capture family, six crates in one cycle                         | 5            |
| 7 ProtoMissing's five protocols                                       | 2            |
| 8 Close-out and docs                                                  | 2            |
| Tests moved with their crates                                         | 5            |
| **Total**                                                             | **about 46** |

The figure is for one lane on one box.

- The box limit bounds it more than the editing does: one box, and one test target at a time on 12 GB.
- A build with Smithay takes minutes, so a step batches crates whose host traits do not overlap.
- The cheap steps come first, so the pattern is proven on trivial crates before the large ones.

## Alternatives considered

| Option                                                              | Why not                                                                                                   |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| One crate per family: twelve crates grouping the protocols by theme | the first choice; the owner then chose one crate per protocol, for isolation of a change and of its tests |
| One `gnoblin-wayland-protocols` crate with a module per protocol    | it only moves the monolith, and an edit still recompiles every protocol                                   |
| A type map for protocol state instead of typed `State` fields       | removes the accessor per crate, but puts a hash lookup on the pointer and frame paths                     |
| Make `State` generic, or move it into the api crate                 | `State` owns the scene, the seat and the backend; every protocol crate would depend on all of it          |
| A `delegate_*!`-style macro exported by each crate                  | keeps the logic in the crate but hides it from the tools and from unit tests                              |
| Fold `extension.rs` into a crate                                    | it serves no protocol and names four compositor types, which the api crate would then have to carry       |
| Write each `Cargo.toml` by hand                                     | 54 manifests drift; the table is the one place to review a dependency                                     |

## Coordination

ProtoMissing added four protocols while this lane worked, and they landed as crates in the new layout.

- The virtual pointer, the KDE server decoration, the KDE appmenu and ext-workspace are crates with their glue in `glue/`.
- `ext_transient_seat_v1` was not added.
- `tests/protocol_list.rs` reads the registry and checks `docs/wayland-protocols.md`.

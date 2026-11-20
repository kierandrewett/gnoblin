# Protocol crates

Status: design, lane ProtoCrates. Phase 1 (this note) is decided. Phase 2, the extraction, follows the order in
[Migration order and gates](#migration-order-and-gates). The numbers come from `origin/rust-compositor` at `9d20a9a6d`.

## Status

Landed in this lane (each is a crate under `crates/protocols/` and its module is gone from `gnoblin-wayland`):

- the seam and tooling: `gnoblin-wayland-api` (`ProtocolInfo`, `Registrar`, the host traits `CoreHost`, `WindowHost`,
  `OutputHost`), the aggregate `gnoblin-wayland-protocols` (the `ALL` table), `xtask/protocols.toml`,
  `cargo xtask protocol-crates [--check]`, and the deps rules `protocol-layering` and `protocol-root`;
- `access` moved into the api crate, and `Registrar` carries the roster (`Registrar::filter`);
- the plain-data crate `gnoblin-wayland-types` (see [The types crate](#the-types-crate)), and 39 protocol crates with the shared crate `foreign-toplevel-shared`: `wp-alpha-modifier-v1`, `wp-single-pixel-buffer-v1`, `zwp-pointer-gestures-v1`,
  `zwp-relative-pointer-v1`, `zwp-idle-inhibit-v1`, `xdg-system-bell-v1`, `xdg-toplevel-icon-v1`,
  `xdg-toplevel-tag-v1`, `xdg-dialog-v1`, `wl-output`, `wl-shm`, `zwp-primary-selection-v1`, `wp-content-type-v1`,
  `wp-fifo-v1`, `wp-fractional-scale-v1`, `wp-tearing-control-v1`, `wp-presentation-time`,
  `zwp-keyboard-shortcuts-inhibit-v1`, `wp-viewporter`, `wp-cursor-shape-v1`, `zxdg-output-v1`, `zwp-pointer-constraints-v1`, `ext-data-control-v1`, `zwlr-data-control-v1`, `zwp-input-method-v2`, `ext-background-effect-v1`, `wp-security-context-v1`, `zwlr-gamma-control-v1`, `zwlr-output-power-management-v1`, `xdg-foreign`, `ext-idle-notify-v1`, `zwp-text-input-v3`, `ext-foreign-toplevel-list-v1`, `zwlr-foreign-toplevel-management-v1`, `zwp-virtual-keyboard-v1`, `zwlr-output-management-v1`, `zxdg-decoration-v1`, `xdg-activation-v1`, `zwlr-layer-shell-v1`. Their forwarding
  impls are in `crates/gnoblin-wayland/src/glue/`, one file per crate.

Not yet done, in the order of [Migration order and gates](#migration-order-and-gates): every crate that needs the
access roster (ProtoRetire has removed session host from `access.rs`; view source is on hold by the owner, so the move can start), the capture family (waits for ProtoRetire's decision on
`capture_views`), the large protocols (`xdg-shell`, tablet, session lock), `wl-compositor`, `wl-seat`,
`wl-data-device-manager`, `zwp-linux-dmabuf-v1`,
ProtoMissing's five protocols, the
real-client test move and `gnoblin-wayland-testkit`, and the advertised-globals test.

Two decisions made while doing the work differ from the text below. The table lives in `xtask/protocols.toml`, not in
`crates/protocols/`, and a row carries no module name or access column, because each crate states its own `PROTOCOL`.
Shared crates must end in `-shared`, so `image_copy_session` will be `gnoblin-wayland-image-copy-shared`.

## What blocks the rest

The protocols still in `gnoblin-wayland` name types that live in the compositor, so their crates cannot depend only on the api crate. The blockers, by module:

- `xdg_shell`, `wl_compositor`, `wl_seat`: `WindowRequest`, `TreeRequest` and the seat's own tables.
- `zwp_linux_dmabuf_v1`: the backend's `Backend` and the GPU ledger.
- the capture family: the frame pixel path, the lock embargo and `gnoblin_pipewire_export`.

Each needs its shared types moved down first, into the types crate below. `zwlr_output_management_v1` was the first: its
`HeadInfo`, `HeadRequest` and `ModeRequest` now live there. `zxdg_decoration_v1` was the second: `DecorationWanted` and
`WindowDecoration` moved from `gnoblin-backend`'s compose module, which now names them from the types crate.
`xdg_activation_v1` was the third: `ActivationDenial` moved from the observation module, and the token rules are generic over a `Host`
that says what a connection has earned, what the session's clock reads and what an allowed activation does to a window.
`zwlr_layer_shell_v1` was the fourth: `LayerBand`, `LayerNodeInfo` and `LayerOverride` moved from the composer; the crate holds the
global, the table of surfaces and the arithmetic that places one, and the compositor's `layer_shell` module arranges, configures
and draws them.

### The types crate

Decision (Main, reversible): a second hand-written crate, `gnoblin-wayland-types` (`crates/protocols/types`), holds the
plain-data compositor types that the blocked protocols name. It sits beside the api crate at the bottom of the tier.

- **Data only.** Structs, enums and trivial helpers. Anything that decides something about the compositor stays in
  `gnoblin-wayland`: for example `HeadRequest`'s change to an output profile is `output_config::request_change`, because it
  names `gnoblin-output`.
- **Who depends on it.** `gnoblin-wayland` and the protocol crates that list `needs = ["types"]` in their row. A crate that
  names the type elsewhere in the workspace (`gnoblin-backend` for a compose type) depends on it directly. There is no
  second definition and no re-export shim: a caller names the type from the types crate, or `gnoblin-wayland` re-exports
  it at its public root where it was already public (`HeadInfo`, `HeadRequest`, `ModeInfo`, `ModeRequest`).
- **The rule.** `xtask deps` (`protocol-layering`) lets a protocol crate name the api crate, the types crate and shared
  crates, and nothing else of the workspace beyond the foundation crates and the three hubs. The types crate names only
  foundation crates; it names no crate of the tier and no engine crate.

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

Two modules are not extracted. `gnoblin_session_host_v1` and `gnoblin_view_source_v1` (871 lines, with `wire.rs`, which
carries the crate's only `unsafe_code` allow) are being deleted by lane ProtoRetire, by the owner's decision. They are
not in the inventory, no crate is made for them, and this lane does not edit them or the parts of `access.rs`,
`state.rs`, `server.rs` and `lib.rs` that name them. The root `protocols/gnoblin-blur-fade-v1.xml` was deleted (D24).

Two facts decide the design.

1. **The dispatch glue is already generic.** `dispatch.rs` implements `Dispatch` and `GlobalDispatch` once for `State`,
   over any user data that implements Smithay's `Dispatch2<I, State>` or `GlobalDispatch2<I, State>`. No module carries a
   `delegate_*!` macro. A protocol crate can implement `Dispatch2<I, D>` for its own user data, for any `D` that meets
   its host trait, and the one blanket impl in `gnoblin-wayland` binds it. That removes the orphan-rule problem for the
   protocol's own objects.
2. **State coupling is wide but shallow.** The modules together read 133 `State` fields and 493 `State` methods. Each
   module reads few of them, and most are read by one module only.

### Inventory

"State f / m" counts the distinct `State` fields and methods a module uses (`self.x` or `state.x`), including methods
that module defines on `State` itself. "Access" is the `access::Global` that gates the module. `core` means no roster
entry, so every client that is not sandboxed sees it. `open` means the global is gated through the roster but
advertised to every client that is not sandboxed. `shared` marks a crate that serves no protocol of its own.

| Crate                                                 | Module                                | Lines | Served by             | Access                    | State f / m |
| ----------------------------------------------------- | ------------------------------------- | ----- | --------------------- | ------------------------- | ----------- |
| `gnoblin-wayland-capture-shared`                      | `capture_dmabuf`                      | 370   | own code              | shared                    | 2 / 10      |
| `gnoblin-wayland-capture-shared`                      | `capture_shared`                      | 1054  | own code              | shared                    | 6 / 19      |
| `gnoblin-wayland-capture-shared`                      | `capture_views`                       | 215   | own code              | shared                    | 2 / 7       |
| `gnoblin-wayland-ext-background-effect-v1`            | `ext_background_effect_v1`            | 123   | Smithay + own objects | BackgroundEffect (open)   | 2 / 1       |
| `gnoblin-wayland-ext-data-control-v1`                 | `ext_data_control_v1`                 | 35    | Smithay               | DataControl               | 1 / 0       |
| `gnoblin-wayland-ext-foreign-toplevel-list-v1`        | `ext_foreign_toplevel_list_v1`        | 57    | Smithay               | ForeignToplevelList       | 1 / 0       |
| `gnoblin-wayland-ext-idle-notify-v1`                  | `ext_idle_notify_v1`                  | 355   | own code              | open                      | 4 / 6       |
| `gnoblin-wayland-ext-image-capture-source-v1`         | `ext_image_capture_source_v1`         | 72    | Smithay               | ImageCopyCapture          | 1 / 1       |
| `gnoblin-wayland-ext-image-copy-capture-v1`           | `ext_image_copy_capture_v1`           | 199   | Smithay               | ImageCopyCapture          | 1 / 13      |
| `gnoblin-wayland-ext-session-lock-v1`                 | `ext_session_lock_v1`                 | 805   | Smithay               | core                      | 7 / 31      |
| `gnoblin-wayland-foreign-toplevel-shared`             | `foreign_toplevel_shared`             | 254   | own code              | shared                    | 5 / 5       |
| `gnoblin-wayland-image-copy-session`                  | `image_copy_session`                  | 659   | Smithay + own objects | shared                    | 1 / 5       |
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

`extension.rs` (499 lines) is not in the table. It is the seam through which a compat adapter registers a redirect. It
serves no protocol, and it names `LockAuth`, `CursorSettings`, `TreeHooks` and `WindowRequest`, which are compositor
types. It leaves `protocols/` and stays in `gnoblin-wayland` as `src/extension.rs`. `mod.rs` disappears with the last
extraction, and `wire.rs` goes with ProtoRetire's deletion.

The inventory yields 52 crates: 49 protocol crates and 3 shared crates. The shared crates are the rows marked `shared`
in the Access column. `capture_dmabuf.rs` and `capture_views.rs` are parts of the capture engine and fold into
`gnoblin-wayland-capture-shared`.

### Naming rule

The owner fixed the names: `gnoblin-wayland-` plus the protocol's own name with underscores turned into hyphens,
version suffix included when the name has one. The protocol's name is the interface or manager name that the module
already carries, so the crate for `zwlr_layer_shell_v1` is `gnoblin-wayland-zwlr-layer-shell-v1`, for
`ext_foreign_toplevel_list_v1` it is `gnoblin-wayland-ext-foreign-toplevel-list-v1`, and for `wp_viewporter` it is
`gnoblin-wayland-wp-viewporter`. The api crate and the shared crates take the same prefix. The compositor crate stays
`gnoblin-wayland`.

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
  capture-shared/                    gnoblin-wayland-capture-shared (shared crates)
  foreign-toplevel-shared/
  image-copy-session/
  api/                               gnoblin-wayland-api: the seam every protocol crate may use
  all/                               gnoblin-wayland-protocols: the aggregate, owns the globals table
  testkit/                           gnoblin-wayland-testkit: dev-only real-client harness
  ...
xtask/protocols.toml                 the table: one row per crate
```

The root `Cargo.toml` lists these members by glob (`protocols/*`), so a new protocol does not touch it. The table sits
in `xtask/` next to the other reviewed lists (`layout-allow.toml`, `dupes-allow.toml`) so that the glob matches only
directories. `gnoblin-wayland` names one dependency, `gnoblin-wayland-protocols`, instead of 52.

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

The `State` fields that hold a protocol's tables (`ExtIdleNotifyV1Tables`, `CaptureTables`, `XdgForeign` and the rest)
stay fields of `State`, typed. Several are read on the pointer and frame paths, and a type map would put a hash lookup
there. The table type moves into its crate, and the host trait hands it out: `fn tables(&mut self) -> &mut Tables`. A
protocol that needs two fields at once gets them from one accessor, so the borrow checker still sees disjoint borrows.

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

`impl<D: Host> IdleInhibitHandler for D` is rejected by the compiler as an uncovered type parameter. A macro that the
crate exports and the glue invokes would work, but it hides the logic from the tools and from unit tests. Forwarding is
the choice. The logic is a generic function in the protocol crate, `pub fn inhibit<H: Host>(host: &mut H, surface:
WlSurface)`, and the glue body is one line. The blanket `Dispatch` and `GlobalDispatch` impls in `dispatch.rs` stay as
they are: `request_guard`, `request_tap`, the memory ledger and the `view` role gate run there, and the protocol crates
need none of it.

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

`Registry` carries the `DisplayHandle`, the `Access` roster, the `Protocols` configuration and the host's loop handle.
`register` creates the globals with the filter that the `Privilege` implies, and returns the Smithay state or the table
struct, which `State` stores in its typed field. `State::new` calls the registers in the order it calls today's
`global()` functions, so global numbering does not change.

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
Cargo allows the cycle, because a dev-dependency is not part of the normal graph. The crate under test is compiled
twice, once as `gnoblin-wayland`'s dependency and once as the test target. The tests talk over the socket, so the two
copies never meet. `xtask deps` already exempts development edges from `engine-isolation`, and the new rules do the
same.

The session harness (`tests/stock`, `tests/owned`, `tests/session` and the helpers at the top of `tests/protocols.rs`)
is shared today by `#[path]` modules. It moves into one dev-only crate, `gnoblin-wayland-testkit`, so each protocol crate
imports it and none copies it. A test moves with its protocol when the protocol's name is in its file or test name and
it needs nothing from another protocol. The rest stays. To limit link time, a crate has one integration-test binary,
`tests/real.rs`, with submodules.

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
edge, and the rule names it as it names the hubs. `pipewire-export-isolation` lists `gnoblin-wayland-capture-shared` next
to `gnoblin-wayland`, because the capture engine that names the export moves with that crate.

`logging` scans `crates/*/src` today. It is extended to the table's directories, and `cargo xtask logging` is run to
prove the new crates are read. `layout` scans `crates` and `xtask` for root `src/` and `patches/` paths, and the
new crates name neither.

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

1. **Scaffolding.** `gnoblin-wayland-api` with `access` moved in, `gnoblin-wayland-protocols` with an empty table,
   `xtask/protocols.toml`, `xtask protocol-crates`, the member glob, the `deps` tier and its tests, `gnoblin-wayland-testkit`,
   `CHANGELOG`, and the crate list in `design/modular-framework.md`. `access` moves into the api crate only after
   ProtoRetire's deletion has landed, because that lane edits `access.rs` (the `SessionHost` and `View` roles). Until
   then the crates that need the roster (layer shell, the foreign-toplevel pair, data control, capture, gamma, output
   power and management, virtual keyboard, input method, background effect) wait, and steps 2 to 4 start with the
   protocols that need none of it.
2. **Smithay-only protocols with no `State` method:** alpha modifier, single-pixel buffer, pointer gestures, relative
   pointer, primary selection, both data-control crates, toplevel icon, toplevel tag, system bell, dialog, idle
   inhibit, wl-output, wl-shm, FIFO, content type, security context.
3. **Small protocols with host methods:** fractional scale, tearing control, viewporter, presentation time, cursor
   shape, xdg-output, xdg-decoration, background effect, shortcuts inhibit, linux-dmabuf, ext-foreign-toplevel with its
   shared crate, wlr-foreign-toplevel.
4. **Medium protocols:** idle notify, activation, xdg-foreign, gamma control, output power, output management,
   text-input, input-method, virtual keyboard, pointer constraints, wl-data-device, wl-compositor, wl-seat.
5. **Large protocols:** xdg-shell, layer shell, tablet, session lock.
6. **The capture family in one run,** because its modules form a cycle today: `image-copy-session`, then
   `capture-shared` with `capture_dmabuf` and `capture_views` folded in, then screencopy, image-copy-capture,
   and image-capture-source. The family waits for ProtoRetire, which removes `view_source` and may redesign
   `capture_views`. Three edges point up today: `capture_shared` creates the screencopy and
   image-copy globals, and `capture_dmabuf` calls `answer_copy` in screencopy. Global creation moves into each
   protocol's `register`, and `answer_copy` moves down into the shared crate.
7. **ProtoMissing's protocols,** each when its lane has pushed it: `zwlr-virtual-pointer-v1`, `kde-server-decoration`,
   `kde-appmenu`, `ext-workspace-v1`, `ext-transient-seat-v1`. Each moves in a commit of its own.
8. **Close-out.** `protocols/` is gone from `gnoblin-wayland`. The docs, `CHANGELOG`, the crate lists and
   `design/modular-framework.md` name the final set.

**Gate per step**, on one Agent37 box and the exact pushed SHA:

- `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo xtask gen-sdks --check`, `abi-check`, `dupes --deny`, `layout`, `deps` and `protocol-crates --check`;
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

The figure is for one lane on one box. The box limit, one box and one test target at a time on 12 GB, bounds it more
than the editing does: a `gnoblin-wayland` build with Smithay takes minutes, so a step batches crates whose host traits
do not overlap. The cheap steps come first, so the pattern is proven on trivial crates before the large ones.

## Alternatives considered

| Option                                                                                                                                                                    | Why not                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| One crate per family: 12 crates for layer shell, foreign toplevel, capture, data control, input, pointer, idle, lock, xdg extras, surface hints and the private protocols | the first choice; the owner then chose one crate per protocol, for isolation of a change and of its tests |
| One `gnoblin-wayland-protocols` crate with a module per protocol                                                                                                          | it only moves the monolith, and an edit still recompiles every protocol                                   |
| A type map for protocol state instead of typed `State` fields                                                                                                             | removes the accessor per crate, but puts a hash lookup on the pointer and frame paths                     |
| Make `State` generic, or move it into the api crate                                                                                                                       | `State` owns the scene, the seat and the backend; every protocol crate would depend on all of it          |
| A `delegate_*!`-style macro exported by each crate                                                                                                                        | keeps the logic in the crate but hides it from the tools and from unit tests                              |
| Fold `extension.rs` into a crate                                                                                                                                          | it serves no protocol and names four compositor types, which the api crate would then have to carry       |
| Write each `Cargo.toml` by hand                                                                                                                                           | 54 manifests drift; the table is the one place to review a dependency                                     |

## Coordination

ProtoMissing adds `zwlr_virtual_pointer_v1`, `kde_server_decoration`, `kde_appmenu`, `ext_workspace_v1` and
`ext_transient_seat_v1` as modules in `protocols/`, and a `tests/protocol_list.rs` that reads the registry and checks
`docs/wayland-protocols.md`. This lane extracts the existing modules first and moves theirs when they are pushed, so
neither lane edits a file the other is rewriting. Until step 7, `protocols/mod.rs` shrinks by deletions from this lane
and grows by `pub mod` lines from that one.

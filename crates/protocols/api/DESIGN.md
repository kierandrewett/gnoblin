# Protocol crates

How the Wayland protocol tier is split into crates, and the rules each crate follows. The list of crates is
`xtask/protocols.toml`; `cargo xtask protocol-crates` generates the manifests from it.

The handlers of `xdg_shell`, `wl_compositor`, `wl_seat`, `wl_data_device_manager` and `ext_session_lock_v1` stay in
`crates/gnoblin-wayland/src/glue/`, and the capture engine stays in `gnoblin-wayland`. Their crates hold the global,
what it advertises and the code that is the protocol's alone; the answers are the engine's own work (the popup manager
and its grabs, interactive move and resize, the commit path, the seat's focus, the clipboard, the lock card), so a host
trait that gave a crate that access would be the whole of `State` behind a trait with one implementor.

A global that exists only on some sessions (the dmabuf global, on a renderer that imports GPU buffers) has a
`conditional` row. It is in `CONDITIONAL` instead of `ALL`, and the advertised-globals test checks that a registry
never lists it above the declared version.

## The types crate

A second hand-written crate, `gnoblin-wayland-types` (`crates/protocols/types`), holds the plain-data compositor types
that protocol crates name. It sits beside the api crate at the bottom of the tier.

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

## One crate per protocol

Every Wayland protocol the compositor serves itself is its own crate, named `gnoblin-wayland-<protocol>`. Code that two
protocols share goes into a small shared crate that both depend on. `gnoblin-wayland` keeps only compositor glue: the
state, the server, the input, frame and output machinery, and the handlers that forward into the protocol crates. An
edit to one protocol recompiles that crate, the aggregate and the glue, not the other protocols.

The dependency direction is fixed:

```text
gnoblin-wayland -> gnoblin-wayland-protocols -> gnoblin-wayland-<protocol> -> gnoblin-wayland-<shared> -> gnoblin-wayland-api, gnoblin-wayland-types
```

Every protocol crate may also name the foundation crates, as `gnoblin-wayland` does. No protocol crate names
`gnoblin-wayland`. A protocol crate asks the compositor for what it needs through traits, and `gnoblin-wayland`
implements those traits on its `State`.

The dispatch glue is generic. `dispatch.rs` implements `Dispatch` and `GlobalDispatch` once for `State`, over any user
data that implements Smithay's `Dispatch2` or `GlobalDispatch2`, and no module carries a `delegate_*!` macro. A protocol
crate implements `Dispatch2<I, D>` for its own user data, for any `D` that meets its host trait, and the one blanket
impl binds it. That removes the orphan-rule problem for the protocol's own objects.

## Naming rule

A crate is `gnoblin-wayland-` plus the protocol's own name, with underscores turned into hyphens.

- The version suffix stays when the name has one.
- The name is the interface or manager name the module already carries.
- So `zwlr_layer_shell_v1` is `gnoblin-wayland-zwlr-layer-shell-v1`, and `wp_viewporter` is `gnoblin-wayland-wp-viewporter`.
- The api crate and the shared crates take the same prefix. The compositor crate stays `gnoblin-wayland`.

The prefix means `gnoblin-wayland-*` is a family of crates the compositor crate depends on, so a reader sees at once
whose they are. `xtask deps` finds the tier through the table, not through the prefix, so `gnoblin-wayland` itself and
any later crate that happens to share the prefix are not caught by accident.

## Layout

The crates live in `crates/protocols/`, one subdirectory per crate, named after the protocol without the prefix:

```text
crates/protocols/
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

A trait may be split when its users diverge. `gnoblin-wayland-api` depends on Smithay (the
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

### Methods on `State`

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

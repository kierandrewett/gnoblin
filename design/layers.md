# Layers

Companion to D28 and D29 in [rust-compositor.md](rust-compositor.md). It defines a runtime layer registry. Gnoblin registers its own layers: `portals`, `alerts` and `window_menu`. A built-in Wayland provider registers the client surfaces: `surface_toplevel`, `surface_layer_shell` and `surface_x11`. Extensions register theirs. Decisions live in the main document; this file holds the detail.

Status: proposed. Nothing here is implemented.

Citations are `path:line`, checked in this lane on 2026-11-06. `[INFERENCE]`
marks a claim not checked against code or a primary source.

## Today

### Portal placement

- The portal dialog is placed by an output rectangle, not by a window. The
  compositor's `pump_portal` takes the first output in the snapshot and its
  rectangle (`crates/gnoblin-compositor/src/ipc/server.rs:1530-1546`).
- `PortalService::pump` receives that `Option<(OutputId, Rect)>`
  (`crates/gnoblin-compositor/src/ipc/portal.rs:348`).
- `placement` returns `dialog_placement(rect, width, height)` (`portal.rs:370-377`).
  `dialog_placement` centres the frame on that rectangle (`portal.rs:310-317`).
- **Answer:** `placement: Option<Rect>` derives only from the output rectangle and
  the dialog size. No window is read.
- **Answer:** portals do not know their requesting window. `submit` takes no window
  argument (`portal.rs:91`).

### parent_window

- The portal interfaces take `parent_window` (`crates/xdg-desktop-portal-gnoblin/src/backends.rs:85`).
- The spec gives three forms: `x11:<XID>` in hexadecimal, `wayland:<HANDLE>` from
  xdg_foreign v2, or an empty string
  ([window identifiers](https://flatpak.github.io/xdg-desktop-portal/docs/window-identifiers.html)).
- Every interface stores the string and drops it: `iface/capture.rs:88`, `:111`,
  `:177`, `:317`; `iface/remote.rs:165`, `:490`, `:527`; `iface/shortcuts.rs:103`, `:171`.
- A grep of `crates/` finds no reader in `gnoblin-compositor`, `gnoblin-proto` or
  `gnoblin-schema`. `gnoblin-wm/src/wm.rs:995` has its own `parent_window`, for a
  transient window, not for a portal request.

### xdg-foreign and the handle

- xdg-foreign is served: `zxdg_exporter_v2` and `zxdg_importer_v2`, from Smithay's
  `XdgForeignState` (`crates/gnoblin-wayland/src/protocols/zxdg_foreign_v2.rs:1-38`;
  `state.rs:323-325`, `:651`, `:704`). `xdg_dialog_v1` is served too (`state.rs:327-329`).
- A parent relation reaches the WM as `WindowRoleChanged`. The WM floats an imported
  dialog above its parent (`crates/gnoblin-wm/src/wm.rs:994-998`).
- **`wayland:<handle>` has no route to a window today.** Smithay mints the handle in
  its exporter (`smithay/src/wayland/xdg_foreign/handlers.rs:65-75`, pinned rev
  `65aa86f`). The map `exported` is private to `XdgForeignState` (`mod.rs:98`).
  The only lookup is the importer's `import_toplevel`, which needs a client to set its
  own surface's parent (`handlers.rs:146-156`).
- `x11:<XID>`: Gnoblin has `gnoblin-xwayland` (`xtask/src/deps.rs:61`). XID lookup
  against its window map is possible [INFERENCE: not checked].

### Stacking and the frame

- The window stack has two layers, `StackLayer::Normal` and `Fullscreen`. They never
  interleave (`crates/gnoblin-wm/src/stack.rs:1-13`, `:21-26`).
- The output frame has a `layers` list, one `overlay` slot and one `cursor` slot
  (`crates/gnoblin-backend/src/render/frame.rs:235-237`). `set_overlay` replaces the
  one overlay (`frame.rs:358-360`), drawn "above every layer and below the cursor"
  (`frame.rs:351`).
- wlr layer shell has four layers: background, bottom, top and overlay [INFERENCE:
  from the protocol, not checked in this lane].

### Modal, lock and capture

- `Modal` is `{ output, overlay }` (`crates/gnoblin-wayland/src/modal.rs:14-24`).
  While set, no key or pointer button reaches a client on that output (`modal.rs:1-4`).
- `ModalEvent` routes the input (`modal.rs:44`).
- On lock, `pump` clears the modal and settles pending requests with `error(locked)`
  (`portal.rs:348-356`).
- Screen captures leave the overlay out (`Modal` doc).

### Not-responding dialog and window menu

- The `unresponsive` property sits on the window object
  (`crates/gnoblin-compositor/src/ipc/session.rs:72-73`, `:113`, `:332`). Changes are
  tracked at `session.rs:359-361`.
- `window.kill` sends SIGKILL through a pidfd (`crates/gnoblin-compositor/src/ipc/server.rs:1260-1343`).
- No Rust code draws a not-responding dialog. `design/gnome-parity.md:13` says Bingux
  draws it, with Wait and Force Quit. That code is out of tree.
- The window menu: Mutter's hook is `patches/mutter/gnoblin-plugin.c:389`
  (`show_window_menu`). Bingux draws it in `WindowMenu.qml`. No Rust code draws it.

### Object model and Lua

- "Everything a shell touches is an object" (`design/rust-compositor.md:92`).
- The Lua declaration has a `LayerSurface` class (`docs/public/gnoblin.d.lua:758`).
  Its `:on` and `:watch` pattern is the model for handles.

### Dependency rules

- Only the compositor may depend on `gnoblin-portal` (`xtask/src/deps.rs:454-488`,
  `portal_layering`).
- Foundation crates are listed at `xtask/src/deps.rs:34`.

## Model: a layer is a registered object

A layer is a runtime object with an identity and a lifetime. `register` creates it,
`remove` ends it, and a provider disconnect ends it too. Every Gnoblin claim uses
the same registry as an extension claim. The compositor enforces the same rules,
with trust levels only where stated.

| Field      | Meaning                                                                      |
| ---------- | ---------------------------------------------------------------------------- |
| `name`     | Unique while live. Gnoblin names are bare. Extensions use `ext.<id>.`.       |
| `provider` | Set by the compositor from the registering crate or extension id. Read-only. |
| `scope`    | `output` or `window`. A `window` layer names one target.                     |
| `target`   | Window id for `window` scope. Read-only. The layer follows it.               |
| `band`     | Ordering substrate: one of the closed bands below.                           |
| `order`    | `above` or `below` a named layer in the same band.                           |
| `input`    | `passive`, `modal-to-target`, `modal-to-output` or `grab`.                   |
| `focus`    | `takes` or `none`.                                                           |
| `capture`  | `include` or `exclude`.                                                      |
| `lock`     | `hide` (removed on lock) or `keep`. Gnoblin providers only.                  |
| `effects`  | `inherit` or `none`, per D24 and D26.                                        |
| `content`  | A surface the provider owns, or a named compositor painter.                  |
| `state`    | `mapped`, `hidden` or `ended`, with a reason when ended.                     |

### Bands: ordering substrate, not identity

The band set is closed. The compositor owns it, and configuration cannot add to it.
Bottom to top: `background`, `bottom`, `windows`, `top`, `overlay`, `system`, `lock`,
`cursor`.

Why keep bands. A registered layer needs one total order, and the compositor needs a
trust boundary: no extension may sort above the lock or the cursor. A band is a
default slot. A layer's identity is its name, and its place is `order` inside the band.

- `windows` holds `surface_toplevel` and `surface_x11` records, and window-scope layers, each directly above its target.
- `background`, `bottom`, `top` and `overlay` hold `surface_layer_shell` records, one per wlr layer. `top` and `overlay` also hold extension output layers.
- `system` holds output-scope Gnoblin claims, such as a portal with no parent.
- `lock` and `cursor` take no registrations from anyone but the compositor.

Fullscreen placement is Needs Kieran 13. The wlr bands match wlr's own order.

## Client surfaces are layers

Every client surface that takes a place on screen is a layer record. A built-in Wayland provider registers three kinds through the same `LayerRegistry` that extensions use. The stack, the frame builder and `layers.list` do not special-case any of them.

| Kind                  | Scope    | Band                                | Provider           | Record covers                           |
| --------------------- | -------- | ----------------------------------- | ------------------ | --------------------------------------- |
| `surface_toplevel`    | `window` | `windows`                           | `gnoblin-wayland`  | One xdg_toplevel window                 |
| `surface_layer_shell` | `output` | `background` to `overlay`, by layer | `gnoblin-wayland`  | One zwlr_layer_surface_v1               |
| `surface_x11`         | `window` | `windows`                           | `gnoblin-xwayland` | One managed or override-redirect window |

**Names.** Built-in per-object records are named `<kind>:<key>`. The key is the window id for `surface_toplevel` and `surface_x11`, and the client namespace for `surface_layer_shell`, as in `surface_layer_shell:org.gnoblin.TestBar`. A second live surface with the same namespace gets `#2`. The parity test changes its lookup key only, not its assertions [INFERENCE: check the lookup in `tests/layer_shell.rs`].

**Four wlr layers are four bands.** A record has one band, and the band is the trust boundary. So `surface_layer_shell` is four band positions, not one record across four bands. The closed band set already holds the four wlr layers in the same order, so nothing sorts inside the kind. `layers.list` reports the wlr layer as `band`, which keeps the parity test's `layer = "top"`.

**Toplevels and fullscreen.** A fullscreen toplevel stays in `windows`, and the stack keeps `Fullscreen` as its order there. A `top` bar stays above it. The WM hides the bars of an output while a fullscreen window is on that output (proposed; Needs Kieran 13).

**Popups are not records.** An xdg popup or zwlr popup is drawn directly above its root record, in the root's band, and ends with it. A popup of a popup follows the same rule. Popups are transient and numerous, so listing them would bury `layers.list`.

**Subsurfaces** are part of their root's content. They have no band and no record.

**The cursor is not a record.** A client cursor surface is drawn in the frame's `cursor` slot, above every band, because a pointer must never be covered. The `cursor` band stays compositor-owned. The cursor is the one thing outside `layers.list`.

**Override-redirect X11 windows** are records in `windows`, ordered by X's own stacking, which the WM does not change. Managed X11 windows follow the toplevel rule [INFERENCE: X's stacking is what Xwayland reports; not checked].

**Stacking inside `windows` stays the WM's job**: `StackLayer`, raise, focus modes and the D25 layouts. A layer holds one kind of content and one ordering policy. The registry places a record in its band and orders it against its target, never against other toplevels.

**Input.** The `input` field does not apply to surface records. The seat routes input to the surface. Only `top` and `overlay` layer-shell surfaces may take the keyboard exclusively (`zwlr_layer_shell_v1.rs:166`).

**One list.** `layers.list` returns every record, built in or not, with `provider`, `scope`, `band` and `state`. The frame builder walks that list in order. A window-scope layer such as `portals` sits directly above its target's `surface_toplevel` record and that record's popups, so a raise of the target takes the layer with it. The `cursor` slot is the only frame slot outside the list.

Fullscreen aside, the wlr bands match wlr's own order.

### Ordering and follow rules

- Within a band, the newest claim sits on top, unless `order` names a layer in the
  same band.
- A window-scope layer sits directly above its target in the stack. Raising the target
  raises the layer with it.
- A window-scope layer follows its target through move, minimise, workspace switch
  and output move. It is hidden while the target is minimised or on another workspace,
  and shown when the target returns.
- When the target closes or unmaps, the layer ends with reason `target-gone`.
- An output-scope layer follows its output. When an output is removed, it ends with
  reason `output-gone`.
- Gnoblin claims stack above extension claims on the same target, whatever their
  declared order.

### Input policy

- `passive`: takes no input. Clicks pass through.
- `modal-to-target`: while mapped, the target window gets no key or button. Other
  windows still get input.
- `modal-to-output`: while mapped, no client on the output gets input. This is
  today's `Modal` (`modal.rs:1-4`).
- `grab`: takes the pointer and keyboard until dismissed. A click outside is consumed.
  It does not block other outputs.

## Gnoblin dogfood

Three Gnoblin claims and three built-in surface kinds use the registry. Each replaces existing code or is new.

| Layer                 | Scope                                      | Band and order                         | Input                        | Provider                                                                         | Replaces or adds                                                                            |
| --------------------- | ------------------------------------------ | -------------------------------------- | ---------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `portals`             | `window` if parent resolves, else `output` | `windows`, above target; else `system` | `modal-to-output` (Kieran 2) | `gnoblin-portal`, registered by the compositor's portal service                  | `Modal` and overlay slot (`modal.rs`, `frame.rs:236`); `dialog_placement` (`portal.rs:310`) |
| `alerts`              | `window`                                   | `windows`, above target                | `modal-to-target`            | `gnoblin-compositor` [INFERENCE]; content per Kieran 6                           | New. Detection exists (`session.rs:72-73`); kill exists (`server.rs:1260`).                 |
| `window_menu`         | `window`                                   | `windows`, above target                | `grab`                       | `gnoblin-window-menu` (D29), or a claimant                                       | New in Rust. Replaces Bingux `WindowMenu.qml`.                                              |
| `surface_toplevel`    | `window`                                   | `windows`, one per window              | `gnoblin-wayland`            | Existing window stack (`stack.rs:21-26`). Adds a record per window.              |
| `surface_layer_shell` | `output`                                   | `background` to `overlay`              | `gnoblin-wayland`            | Existing wlr code (`zwlr_layer_shell_v1.rs:482-486`). Adds a record per surface. |
| `surface_x11`         | `window`                                   | `windows`, X stacking                  | `gnoblin-xwayland`           | Existing Xwayland windows (`xwayland.rs:501`). Adds a record per window.         |

### portals

- The compositor's portal service registers `portals` for the request. The relay
  `xdg-desktop-portal-gnoblin` does not register. It passes `parent_window` over the
  existing portal IPC, typed (see Cutover).
- Output scope (today's behaviour) applies while no parent resolves, and in `system`.
- With a resolved parent, the layer sits above that window and follows it. The
  dialog is centred on the parent's rectangle, not the output's.
- `x11:` resolves through `gnoblin-xwayland` [INFERENCE]. `wayland:` has no resolver
  today. It is blocked until Phase 6 chooses one (Kieran 8).
- If the parent closes while the dialog is open, the layer ends `target-gone`, and
  the request continues on output scope, logged.
- One live `portals` dialog per output. Others queue (today: one `active_request`).

### alerts

`alerts` is the compositor's generic layer for any alert it raises. The property and the reason stay `unresponsive`, because they say why the alert exists.

- The compositor registers `alerts` when a window's `unresponsive` becomes true
  (`session.rs:359-361`), and removes it when the window responds or closes.
- The content offers Wait and Force Quit. Force Quit calls `window.kill`
  (`server.rs:1260`). The content is drawn by a provider, not by the compositor
  alone (Kieran 6).
- It blocks its target only (proposed; Kieran 3). A hung window cannot take input,
  so the rest of the desktop keeps working.

### window_menu

- The layer hosts the D29 model. The layer does not change what a menu contains,
  only where it sits and what it blocks. The built-in provider is stock egui (D29), with no custom painting or theme.
- `grab` on the target until dismissed. The Mutter hook (`gnoblin-plugin.c:389`)
  becomes a request for this layer.

## Registration

Three entry points share one registry and one set of rules. Built-in Rust providers use the same `register` as extensions.

### Rust registry

A new foundation crate, `gnoblin-layers`, holds `LayerRegistry`, `LayerSpec`,
`Band`, `Scope`, `InputPolicy`, and the ordering and follow rules. It depends only on
`gnoblin-types` [INFERENCE: add it to `FOUNDATION` at `xtask/src/deps.rs:34`].
`gnoblin-wm` depends on it for stack entries. `gnoblin-compositor` registers the
Gnoblin claims. The portal crates do not depend on it. The compositor's portal
service registers `portals`, so the portal-layering rule holds (`xtask/src/deps.rs:454-488`).

`gnoblin-wayland` and `gnoblin-xwayland` also depend on it, to register the surface kinds [INFERENCE: check that `xtask/src/deps.rs` allows the edge].

```rust
// gnoblin-compositor only. The registry sets `provider` from the crate.
let spec = LayerSpec {
    name: "portals".into(),
    scope: Scope::Window(parent),          // or Scope::Output(output) when no parent resolves
    band: Band::Windows,
    order: Order::AboveTarget,
    input: InputPolicy::ModalToOutput,
    ..LayerSpec::default()
};
let handle = registry.register(Provider::Gnoblin("gnoblin-portal"), spec)?;
```

### Lua

`gnoblin.layers.register{...}` returns a handle object, with the object model's three
kinds of member:

- Properties, read-only: `name`, `provider`, `band`, `scope`, `target`, `state`.
- Signals: `closed(layer, reason)` and `changed(layer, property)`.
- Method: `remove()`, which returns an `Operation` per the object model.

The pattern matches `LayerSurface` (`docs/public/gnoblin.d.lua:758`).

```lua
local note = gnoblin.layers.register {
  name = "ext.example.note",
  scope = "window",
  target = gnoblin.windows.focused(),
  input = "passive",
  order = { above = "ext.example.base" },
}
print(note.name, note.provider, note.state)
note:on("closed", function(layer, reason)
  print("ended", reason)
end)
note:remove()
```

Lua may register `passive` or `modal-to-target` layers, window-scope in `windows`
or output-scope in `top` or `overlay`. It cannot register in `system`, `lock` or
`cursor`, and cannot use `grab` or `modal-to-output` (Kieran 7).

The surface kinds are Rust-only. Their records follow Wayland objects, not Lua calls.

### Extension manifest (D22)

An extension declares its layer powers in `extension.toml`. Host configuration grants
a subset.

```toml
[powers.layers]
names = ["ext.example.panel"]        # must start with ext.<id>.
band_max = "top"                     # "top" (default) or "overlay" for output scope
window_scope = true                  # may register window-scope layers in `windows`
input = ["passive"]                  # "modal-to-target" only by host grant
order_relative_to = ["own"]          # Gnoblin layers only by host grant
```

Who may register above whom:

- An extension layer sits in `windows` (window scope, above its target), `top` or
  `overlay`, up to `band_max`.
- It orders only against its own layers, unless the host grant names more.
- It never sits above a Gnoblin claim on the same target. Gnoblin claims order above
  extension claims, by the rule in Ordering.
- It never enters `system`, `lock` or `cursor`.
- `grab` and `modal-to-output` need a host grant. None is granted by default.
- A power the manifest does not declare is absent, and its code is not loaded (D22).

## Failure modes

| Case                               | Behaviour                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------- |
| Provider crash or disconnect       | Its layers end `provider-gone`. Pending portal requests settle `error(...)`.    |
| Extension unload                   | As a disconnect. All its layers end in one step.                                |
| Name collision                     | `register` refuses with `name-taken`. Names are global while live.              |
| Parent never maps                  | Unresolved `wayland:` or `x11:` falls back to output scope, logged.             |
| Parent closes, dialog open         | Layer ends `target-gone`. Request continues on output scope.                    |
| Parent closes, alert open          | Layer ends `target-gone`. No alert remains.                                     |
| Target unresponsive, then responds | `alerts` ends with reason `target-responsive`.                                  |
| Target moves to another output     | Window-scope layers move with it.                                               |
| Layer requested above the lock     | Refused. Nothing registers in `lock`.                                           |
| Session locks                      | `hide` layers are removed. Pending portal requests settle `error(locked)`.      |
| Client surface destroyed           | Its record ends `surface-gone`. Its popups end with it.                         |
| Many simultaneous requests         | One live `portals` per output. Others queue.                                    |
| Limits                             | Proposed: 32 live layers per provider, 8 per target. [INFERENCE: not measured.] |

## Security

- The compositor sets `provider` from the registering crate or the connection
  identity. A caller cannot set or spoof it.
- The compositor draws the attribution label ("requested by ...") in its own frame,
  on every `modal-*` and `grab` layer. Provider content sits in its own surface and
  cannot cover the label [INFERENCE: not built yet].
- `portals`, `alerts`, `window_menu` and the `surface_` prefix are reserved. The `ext.<id>.` prefix stops an extension from claiming the first three. Only built-in kinds use `surface_`.
- `parent_window` is a client's hint. The compositor places a portal dialog over a
  resolved window only when that window belongs to the requesting client (proposed;
  Kieran 11). Otherwise it falls back to output scope.
- Nothing sits above the lock, except layers on the compositor's allow-list. The lock screen is the only entry. A provider cannot draw over the lock.
- Capture of a secret-bearing layer is never provider-controlled. Other capture inclusion is an option (see Invariants and options).

## Invariants and options

Two kinds of rule. An invariant is enforced by the compositor, and no provider or extension can change it. An option is a value a provider sets on its own layer, within its grant. Invariants are policy for Kieran to decide. Options only need defaults.

### Invariants (A)

- **Parent ownership.** A `parent_window` must name a window of the requesting client. Otherwise the request is refused and falls back to output scope.
- **Band ceiling.** A built-in provider uses the bands its kind allows. An extension stays at or below `band_max`, and never enters `system`, `lock` or `cursor`.
- **Input grant.** An extension requests `passive`. `modal-to-target`, `modal-to-output` and `grab` need a manifest power and a host grant. The compositor draws the attribution label on every modal or grab layer.
- **Consent blocks its output.** A `portals` layer blocks its whole output, so a consent dialog cannot be bypassed by clicking elsewhere (Kieran 2).
- **Secret-bearing capture.** A secret-bearing layer, such as any `portals` dialog, is never captured. No grant, manifest power or provider setting overrides this.
- **Lock.** Nothing draws above the lock except the compositor's allow-list. Extensions cannot join it.
- **Count limits.** Each provider and each target has a maximum number of live layers. The numbers are proposed; the limits themselves are not optional.

### Options (B)

| Option                         | Set by   | Default                                     | Bounded by                                                |
| ------------------------------ | -------- | ------------------------------------------- | --------------------------------------------------------- |
| Scope: `output` or `window`    | Provider | The kind's default                          | The kind's allowed scopes                                 |
| Order within its band          | Provider | Per the Ordering rules                      | Band ceiling; Gnoblin claims stay above                   |
| `passive` or `modal-to-target` | Provider | `passive`                                   | Invariant "Input grant"                                   |
| Capture inclusion              | Provider | Excluded                                    | Manifest power and visible indicator                      |
| Dim the target                 | Provider | Off (proposed)                              | Compositor draws the dim; target untouched                |
| Content kind                   | Provider | Surface, or compositor-painted for `alerts` | Lua: compositor-painted unless `wayland` power (Kieran 7) |

An option never widens an invariant. A request outside the grant is refused and logged.

## Cutover

Each path below moves to the registry, with no shim.

- `crates/gnoblin-backend/src/render/frame.rs:236`: the `overlay` slot goes. `layers` (`:235`) becomes the one ordered list, walked in registry order. `cursor` (`:237`) stays. A breaking change to the frame type, shared by every renderer.
- `crates/gnoblin-wayland/src/modal.rs:1-54`: `Modal` and `ModalEvent` become the
  input policy of the `portals` and `alerts` layers.
- `crates/gnoblin-compositor/src/ipc/portal.rs:91` (`submit`) gains the typed
  parent. `:310-317` (`dialog_placement`), `:348` and `:370-377` (`pump`,
  `placement`) take the target's rectangle; the output rectangle is the fallback.
- `crates/gnoblin-compositor/src/ipc/server.rs:1530-1546` (`pump_portal`) and its
  `set_modal` call register and remove layers through the registry.
- `crates/gnoblin-compositor/src/ipc/session.rs:359-361`: an `unresponsive` change
  registers or removes `alerts`.
- `crates/xdg-desktop-portal-gnoblin/src/backends.rs:85` and the `iface/` sites
  listed under Today: `parent_window` becomes a typed value (`None`, `x11`,
  `wayland`) and is passed on, not dropped.
- `crates/gnoblin-wm/src/stack.rs:21-26`: `StackLayer` keeps `Normal` and `Fullscreen` as the WM's order inside `windows`. Window-scope layers enter the stack as entries directly above their target.
- `crates/gnoblin-wayland/src/protocols/zwlr_layer_shell_v1.rs` and `state.rs:376`: each zwlr surface registers a `surface_layer_shell` record, and its wlr layer selects the band. The four-layer sort moves into the registry.
- `xdg_shell.rs` (toplevels; popups at `:59`) and the Xwayland window code (`xwayland.rs:501`): windows register `surface_toplevel` and `surface_x11`. Popups follow their root instead of their own stacking.
- Docs: `docs/shell-api/layer-surfaces.md` (the `gnoblin.layers` section) and
  `docs/public/gnoblin.d.lua` (`register` and the handle). `layers.list` keeps its
  record shape. A `band` filter is Kieran 9.
- Bingux, out of tree: `WindowMenu.qml` and the not-responding dialog move to
  providers.

## Tests

- Band order: `top` stays below `system`, and `system` stays below `lock`.
- A window-scope layer sits directly above its target. A target raised above other
  windows takes the layer with it.
- A window-scope layer follows its target through move, minimise, workspace switch
  and close, and ends `target-gone`.
- One live `portals` dialog per output; a second request queues.
- A name collision is refused with `name-taken`.
- A provider disconnect ends its layers and sends `closed`.
- A capture omits `system` and Gnoblin claims by default.
- The lock removes `hide` layers and settles pending portal requests.
- An extension cannot register in `system`, `lock` or `cursor`, cannot use `grab`
  without a grant, and cannot order above a Gnoblin claim.
- `alerts` appears when `unresponsive` is set and ends when it clears.
- A `surface_toplevel` record exists for each window, in `windows`, with the window as `target`.
- A `surface_layer_shell` record's band is its wlr layer. A bar on `top` is listed with `band = "top"`.
- A popup sits directly above its root and ends with it.
- A fullscreen window leaves a `top` bar in `top`, and the bar is hidden on that output.
- An override-redirect X11 window is not raised by the WM.
- A window-scope layer sits above its target and the target's popups.

## Phases

1. `gnoblin-layers` crate: the registry, bands, ordering and follow rules, and their tests. No behaviour change.
2. `gnoblin-wm` stack gains window-scope entries above their targets. `StackLayer` keeps `Normal` and `Fullscreen`. Follow rules and stack tests.
3. The frame takes one ordered list. The `overlay` slot goes; `cursor` stays. `Modal` becomes the `portals` input policy.
4. Built-in surface kinds in `gnoblin-wayland` and `gnoblin-xwayland`: `surface_toplevel`, `surface_layer_shell` (four bands) and `surface_x11`. Popups follow their root. The parity test changes its lookup key only.
5. `parent_window` is typed. `x11:` resolves through `gnoblin-xwayland`. `wayland:` needs one of: (a) a Smithay change exposing handle-to-surface lookup on `XdgForeignState`; (b) a Gnoblin table filled when the exporter mints a handle, which needs a wrapper around Smithay's exporter; (c) the dialog becomes a Wayland child toplevel that imports the handle, which changes it from an overlay to a surface. [INFERENCE: (a) is smallest.]
6. `window_menu` (D29), built in.
7. `alerts`, with its content provider.
8. Lua `gnoblin.layers.register`, its handle and signals, and the docs.
9. Extension `[powers.layers]` through D22.

Each phase names the tests that fail without its rule.

## Needs Kieran

Real choices only. Each item is marked (A) if it is a policy choice about an invariant, or (B) if it only sets a default for an option. Do not read (A) as a customisation hook. Naming items are neither.

1. Closed (Main, 2026-11-06): the not-responding layer is `alerts`. Naming only. `unresponsive` stays the property and the reason.
2. (A) Whether a window-targeted `portals` modal blocks the whole output (proposed) or only its target. Proposed: the whole output, so a consent dialog cannot be bypassed.
3. (B) Default input policy for a window-scope `alerts`: blocks its target only (proposed: yes).
4. (A) for `portals`: never captured, by invariant. (B) for `alerts` and `window_menu`: default capture excluded (proposed), as the portal dialog is today.
5. (B) Default ordering of window-scope layers: in `windows` and below top bars (proposed), or above the top bars too.
6. (B) Default content kind for `alerts`: the compositor's built-in painter (proposed), or a provider only.
7. (A) Lua content: compositor-painted only, or a manifest `wayland` power for extensions that need a surface. A `wayland` power is a capability grant.
8. (A) How to resolve `wayland:<handle>`. It enforces parent ownership (invariant 1). Phase 5 options a, b, c. Option (a) is a Smithay change; choose whether Gnoblin carries a Smithay patch.
9. Neither: whether `layers.list` gains a `band` filter in the same change (proposed: yes). This is API shape.
10. (A) Default `band_max` for extensions: `top` (proposed) or `overlay`. The ceiling is invariant 2; this is its value.
11. (A) Whether the compositor checks that a `parent_window` belongs to the requesting client before placing a dialog over it. Invariant 1. Proposed: yes, and it is not optional.
12. Neither: name for the toplevel kind, `surface_toplevel` (proposed, matching `xdg_toplevel`). Kieran suggested `surface_top_level`, "or something".
13. (B) Bar visibility while a fullscreen window is on its output: hidden (proposed). The alternative lifts fullscreen windows above `top`, as wlroots does, which breaks one band per record.
14. (B) Override-redirect X11 windows: X's own stacking inside `windows` (proposed), or lifted to `top` as menus.

# Layers

Companion to D28 and D29 in [rust-compositor.md](rust-compositor.md). It defines a
runtime layer registry. Gnoblin registers its own layers: `portals`, `alerts` and
`window_menu`. Extensions register theirs. Decisions live in the main document;
this file holds the detail.

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

- `windows` holds window-scope layers. Each sits directly above its target in the
  window stack.
- `top` and `overlay` hold wlr surfaces and extension output layers.
- `system` holds output-scope Gnoblin claims, such as a portal with no parent.
- `lock` and `cursor` take no registrations from anyone but the compositor.

[INFERENCE: the wlr bands match wlr's own order. Confirm `top` against fullscreen
windows in the reference session.]

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

Three Gnoblin claims use the registry. Each replaces existing code or is new.

| Layer         | Scope                                      | Band and order                         | Input                        | Provider                                                        | Replaces or adds                                                                            |
| ------------- | ------------------------------------------ | -------------------------------------- | ---------------------------- | --------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `portals`     | `window` if parent resolves, else `output` | `windows`, above target; else `system` | `modal-to-output` (Kieran 2) | `gnoblin-portal`, registered by the compositor's portal service | `Modal` and overlay slot (`modal.rs`, `frame.rs:236`); `dialog_placement` (`portal.rs:310`) |
| `alerts`      | `window`                                   | `windows`, above target                | `modal-to-target`            | `gnoblin-compositor` [INFERENCE]; content per Kieran 6          | New. Detection exists (`session.rs:72-73`); kill exists (`server.rs:1260`).                 |
| `window_menu` | `window`                                   | `windows`, above target                | `grab`                       | `gnoblin-window-menu` (D29), or a claimant                      | New in Rust. Replaces Bingux `WindowMenu.qml`.                                              |

### portals

- The compositor's portal service registers `portals` for the request. The relay
  `xdg-desktop-portal-gnoblin` does not register. It passes `parent_window` over the
  existing portal IPC, typed (see Cutover).
- Output scope (today's behaviour) applies while no parent resolves, and in `system`.
- With a resolved parent, the layer sits above that window and follows it. The
  dialog is centred on the parent's rectangle, not the output's.
- `x11:` resolves through `gnoblin-xwayland` [INFERENCE]. `wayland:` has no resolver
  today. It is blocked until Phase 4 chooses one (Kieran 8).
- If the parent closes while the dialog is open, the layer ends `target-gone`, and
  the request continues on output scope, logged.
- One live `portals` dialog per output. Others queue (today: one `active_request`).

### alerts

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

Three forms share one registry and one set of rules.

### Rust registry

A new foundation crate, `gnoblin-layers`, holds `LayerRegistry`, `LayerSpec`,
`Band`, `Scope`, `InputPolicy`, and the ordering and follow rules. It depends only on
`gnoblin-types` [INFERENCE: add it to `FOUNDATION` at `xtask/src/deps.rs:34`].
`gnoblin-wm` depends on it for stack entries. `gnoblin-compositor` registers the
Gnoblin claims. The portal crates do not depend on it. The compositor's portal
service registers `portals`, so the portal-layering rule holds (`xtask/src/deps.rs:454-488`).

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
| Many simultaneous requests         | One live `portals` per output. Others queue.                                    |
| Limits                             | Proposed: 32 live layers per provider, 8 per target. [INFERENCE: not measured.] |

## Security

- The compositor sets `provider` from the registering crate or the connection
  identity. A caller cannot set or spoof it.
- The compositor draws the attribution label ("requested by ...") in its own frame,
  on every `modal-*` and `grab` layer. Provider content sits in its own surface and
  cannot cover the label [INFERENCE: not built yet].
- `portals`, `alerts` and `window_menu` are reserved names. The `ext.<id>.` prefix
  stops an extension from claiming them.
- `parent_window` is a client's hint. The compositor places a portal dialog over a
  resolved window only when that window belongs to the requesting client (proposed;
  Kieran 11). Otherwise it falls back to output scope.
- Nothing sits above the lock. A provider cannot draw over the lock screen.
- Captures leave `system` and Gnoblin claims out by default (Kieran 4).

## Cutover

Each path below moves to the registry, with no shim.

- `crates/gnoblin-backend/src/render/frame.rs:235-237`, `:351`, `:358-360`: the
  overlay slot becomes one ordered list per band. A breaking change to the frame
  type, shared by every renderer.
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
- `crates/gnoblin-wm/src/stack.rs:1-13`, `:21-26`: `StackLayer` gains layer entries
  that sit directly above their targets.
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

## Phases

1. `gnoblin-layers` crate: the registry, bands, ordering and follow rules, and their
   tests. No behaviour change.
2. `gnoblin-wm` stack gains layer entries above their targets. Follow rules and stack
   tests.
3. The frame takes one ordered list per band. `Modal` becomes the `portals` input
   policy. The single overlay slot goes.
4. `parent_window` is typed. `x11:` resolves through `gnoblin-xwayland`. `wayland:`
   needs one of: (a) a Smithay change exposing handle-to-surface lookup on
   `XdgForeignState`; (b) a Gnoblin table filled when the exporter mints a handle,
   which needs a wrapper around Smithay's exporter; (c) the dialog becomes a Wayland
   child toplevel that imports the handle, which changes it from an overlay to a
   surface. [INFERENCE: (a) is smallest.]
5. `window_menu` (D29), built in.
6. `alerts`, with its content provider.
7. Lua `gnoblin.layers.register`, its handle and signals, and the docs.
8. Extension `[powers.layers]` through D22.

Each phase names the tests that fail without its rule.

## Needs Kieran

Real choices only.

1. Name for the not-responding layer. `portals` and `window_menu` are fixed.
    - `alerts`: short and clear, and recommended. It may clash with notifications and the overlay OSD.
    - `attention`: says what it is for. It avoids the notification clash, but is less common.
    - `notices`: friendly. It is closest to notifications, so the clash is worst.
2. Whether a window-targeted `portals` modal blocks the whole output (proposed) or only its target. Blocking the output stops a consent dialog being bypassed by clicking elsewhere.
3. Whether a window-scope `alerts` blocks its target only (proposed: yes).
4. Capture default for `portals`, `alerts` and `window_menu`: exclude (proposed), as the portal dialog is today.
5. Whether window-scope layers sit in `windows`, above their target and below top bars (proposed), or above the top bars too.
6. Who paints the not-responding content: a compositor built-in painter (proposed), or a provider only.
7. Lua content: compositor-painted only, or a manifest `wayland` power for extensions that need a surface.
8. How to resolve `wayland:<handle>` (Phase 4 options a, b, c). Option (a) is a Smithay change; choose whether Gnoblin carries a Smithay patch.
9. Whether `layers.list` gains a `band` filter in the same change (proposed: yes).
10. Default `band_max` for extensions: `top` (proposed) or `overlay`.
11. Whether the compositor checks that a `parent_window` belongs to the requesting client before placing a dialog over it (proposed: yes).

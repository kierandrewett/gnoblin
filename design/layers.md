# Layers

Companion to D28 and D29 in [rust-compositor.md](rust-compositor.md). It defines a runtime layer registry. Gnoblin registers `gnoblin.portals` and `gnoblin.alerts`, and a Wayland provider registers the client surfaces (`gnoblin.surface_*`). Extensions register under `ext.<id>.`, and the window menu is one of them (D29). Decisions live in the main document; this file holds the detail.

Status: proposed. Phases 1 and 2 are implemented and unit-tested: the `gnoblin-layers` crate (names, bands, scopes, registry, paint order, lifecycle, owner-only secure flag), and window-scope stack entries in `gnoblin-wm`. Wiring into the compositor and Lua is not started.

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
  The relay still drops the string on every interface; nothing in the relay reads it yet.
- **Done:** `gnoblin_types::ParentWindow` parses the string into `None`, `X11(u32)` or
  `Wayland(String)`, and refuses the rest with a reason
  (`crates/gnoblin-types/src/parent_window.rs`).
- **Parent verification is not established yet.** The relay cannot supply a trustworthy caller
  identity: the D-Bus sender of an `org.freedesktop.impl.portal.*` call is the portal frontend,
  not the app, so the bus pid names the frontend. `wire::VerifiedCaller` (pid and app id) and
  `verified_caller` in `iface/mod.rs` remain as written and are not yet a basis for ownership.
  Every request therefore uses output scope today; ownership design pending (Needs Kieran 16).
  Sources: the frontend passes `app_id` but it is empty for unsandboxed host apps
  ([xdg-desktop-portal#579](https://github.com/flatpak/xdg-desktop-portal/issues/579)), and some
  interfaces historically omitted it
  ([#963](https://github.com/flatpak/xdg-desktop-portal/issues/963)). The impl docs do not state
  `app_id` as a security attribution
  ([design considerations](https://flatpak.github.io/xdg-desktop-portal/docs/design-considerations.html)).
  [INFERENCE: no attribution source exists that the compositor can trust for host apps.]
- **Done:** the compositor resolves a parent before the dialog opens. `ipc/server.rs` `portal_ui`
  calls `resolve_portal_scope` (`ipc/portal.rs`), which resolves `wayland:` through
  `exported_window` and `window_process`, then `parent_owner_check` compares that pid with the
  bus pid. It returns `PortalScope::Parent` or `PortalScope::Output { reason }`, and the handler
  logs the scope used. A parent that fails any check (no parent, X11, unknown or revoked handle,
  another process) does not refuse the request: the dialog opens on the output. The test
  `a_payload_claiming_another_pid_is_checked_against_the_bus_pid` covers a forged pid.
- **Scope today:** every request uses output scope. The compositor code that resolves a parent
  exists, but it is not end-to-end parent-window support: parent verification is not established
  (above), the relay does not send the parent, and modal-to-parent has not landed (Needs Kieran 15).
- **Not built:** portal.ui claimants are ignored, built-in only. Claimant-first routing (D19) is
  unbuilt.
- **Gaps, in order:**
    1. Nothing sends `portal.ui` in production. The relay has no path from its portal calls to
       the compositor, so `verified_caller` has no caller yet and only tests send the frame.
    2. The consent prompt does not carry the parent. The relay offers prompts to the `portal.ui`
       slot with `Client::offer` (`wire.rs` `REQUEST_OFFERED`), which has no parent field.
       Changing that frame changes what external slot providers receive.
    3. `x11:` is refused with a reason. `gnoblin-xwayland` has no XID-to-window lookup yet.
    4. Placement over the parent and modal-to-parent input need the layer registry wired into
       the compositor (`gnoblin-layers`, not yet wired).
- The `portal_ui` schema (`crates/gnoblin-schema/schema/portal_ui.toml`) does not
  drive the wire. `wire.rs` is hand-written, so the schema was not edited.

### xdg-foreign and the handle

- xdg-foreign is served: `zxdg_exporter_v2` and `zxdg_importer_v2`, from `crates/gnoblin-wayland/src/protocols/xdg_foreign.rs`. The handle table is `State::xdg_foreign`. `xdg_dialog_v1` is served too.
- A parent relation reaches the WM as `WindowRoleChanged`. The WM floats an imported dialog above its parent (`crates/gnoblin-wm/src/wm.rs:994-998`).
- **`wayland:<handle>` resolves in the compositor.** Each handle is 128 random bits mapped to its
  exported surface. `XdgForeign::exported_surface` gives the surface; `ControlHandle::exported_window`
  gives its window, or `None` when revoked or unknown. The owner check in `check_portal_parent`
  uses it; no relay path reaches that check yet (gap 1 above).
- `x11:<XID>`: Gnoblin has `gnoblin-xwayland` (`xtask/src/deps.rs:61`). XID lookup
  against its window map is possible [INFERENCE: not checked].

### Stacking and the frame

- The window stack has two layers, `StackLayer::Normal` and `Fullscreen`. They never
  interleave (`crates/gnoblin-wm/src/stack.rs:1-13`, `:21-26`).
- The output frame has a `layers` list, one `overlay` slot and one `cursor` slot
  (`crates/gnoblin-render-frame/src/frame.rs:235-237`). `set_overlay` replaces the
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
- Today, screen captures leave the overlay out (`Modal` doc). That is the only capture rule in the code. It becomes `secure = true` on `portals` (see Capture paths).

### Not-responding dialog and window menu

- The `unresponsive` property sits on the window object
  (`crates/gnoblin-compositor/src/ipc/session.rs:72-73`, `:113`, `:332`). Changes are
  tracked at `session.rs:359-361`.
- `window.kill` sends SIGKILL through a pidfd (`crates/gnoblin-compositor/src/ipc/server.rs:1260-1343`).
- No Rust code draws a not-responding dialog. `design/gnome-parity.md:13` says Bingux
  draws it, with Wait and Force Quit. That code is out of tree.
- The window menu is a signal, `menu_requested`, on the `window` object, with a `request_menu` method (D29). Any extension may subscribe and show it. The shipped extension `gnoblin.window_menu` is one such subscriber, and it is enabled by default.

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

In prose, a bare name such as `portals` means `gnoblin.portals`. The registry builds the full name from the provider; the provider supplies only the local part.

| Field                 | Meaning                                                                                                   |
| --------------------- | --------------------------------------------------------------------------------------------------------- |
| `name`                | Full name `<provider>.<local>`, unique while live. Built-ins: `gnoblin.`. Extensions: `ext.<id>.`.        |
| `provider`            | Namespace: `gnoblin` for built-ins, `ext.<id>` for extensions. Set by the compositor. Read-only.          |
| `scope`               | `output` or `window`. A `window` layer names one target.                                                  |
| `target`              | Window id for `window` scope. Read-only. The layer follows it.                                            |
| `band`                | Ordering substrate: one of the closed bands below.                                                        |
| `order`               | `above` or `below` a named layer in the same band.                                                        |
| `input`               | `passive`, `modal-to-target`, `modal-to-output` or `grab`.                                                |
| `paint`, `input_area` | Rectangles the layer paints and takes input in, relative to the layer. Default: the whole layer for both. |
| `focus`               | `takes` or `none`.                                                                                        |
| `secure`              | `false` (default) or `true`. Secure content is never captured. See Capture paths.                         |
| `lock`                | `hide` (removed on lock) or `keep`. Gnoblin providers only.                                               |
| `effects`             | `inherit` or `none`, per D24 and D26.                                                                     |
| `content`             | A surface the provider owns, or a named compositor painter.                                               |
| `state`               | `mapped`, `hidden` or `ended`, with a reason when ended.                                                  |

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

Fullscreen placement is Needs Kieran 11. The wlr bands match wlr's own order.

## Client surfaces are layers

Every client surface that takes a place on screen is a layer record. A built-in Wayland provider registers three kinds through the same `LayerRegistry` that extensions use. The stack, the frame builder and `layers.list` do not special-case any of them.

| Kind                  | Scope    | Band                                | Provider           | Record covers                           |
| --------------------- | -------- | ----------------------------------- | ------------------ | --------------------------------------- |
| `surface_toplevel`    | `window` | `windows`                           | `gnoblin-wm`       | One xdg_toplevel window                 |
| `surface_layer_shell` | `output` | `background` to `overlay`, by layer | `gnoblin-wayland`  | One zwlr_layer_surface_v1               |
| `surface_x11`         | `window` | `windows`                           | `gnoblin-xwayland` | One managed or override-redirect window |

The window manager registers `surface_toplevel` records, not the Wayland provider, because it owns the window stack and the records mirror that stack.

**Names.** Built-in per-object records are named `gnoblin.<kind>:<key>`. The key is the window id for `surface_toplevel` and `surface_x11`, and the client namespace for `surface_layer_shell`, as in `gnoblin.surface_layer_shell:org.gnoblin.TestBar`. A second live surface with the same namespace gets `#2`. The parity test changes its lookup key only, not its assertions [INFERENCE: check the lookup in `tests/layer_shell.rs`].

**Four wlr layers are four bands.** A record has one band, and the band is the trust boundary. So `surface_layer_shell` is four band positions, not one record across four bands. The closed band set already holds the four wlr layers in the same order, so nothing sorts inside the kind. `layers.list` reports the wlr layer as `band`, which keeps the parity test's `layer = "top"`.

**Toplevels and fullscreen.** A fullscreen toplevel stays in `windows`, and the stack keeps `Fullscreen` as its order there. A `top` bar stays above it. The WM hides the bars of an output while a fullscreen window is on that output (proposed; Needs Kieran 11).

**Popups are not records.** An xdg popup or zwlr popup is drawn directly above its root record, in the root's band, and ends with it. A popup of a popup follows the same rule. Popups are transient and numerous, so listing them would bury `layers.list`.

**Subsurfaces** are part of their root's content. They have no band and no record.

**The cursor is not a record.** A client cursor surface is drawn in the frame's `cursor` slot, above every band, because a pointer must never be covered. The `cursor` band stays compositor-owned. The cursor is the one thing outside `layers.list`.

**Override-redirect X11 windows** are records in `windows`, ordered by X's own stacking, which the WM does not change. Managed X11 windows follow the toplevel rule [INFERENCE: X's stacking is what Xwayland reports; not checked].

**Stacking inside `windows` stays the WM's job**: `StackLayer`, raise, focus modes and the D25 layouts. A layer holds one kind of content and one ordering policy. The registry places a record in its band and orders it against its target, never against other toplevels.

**Input.** The `input` field does not apply to surface records. The seat routes input to the surface. Only `top` and `overlay` layer-shell surfaces may take the keyboard exclusively (`zwlr_layer_shell_v1.rs:166`).

**One list.** `layers.list` returns every record, built in or not, with `provider`, `scope`, `band` and `state`. The frame builder walks that list in order. A window-scope layer such as `portals` sits directly above its target's `surface_toplevel` record and that record's popups, so a raise of the target takes the layer with it. The `cursor` slot is the only frame slot outside the list.

Fullscreen aside, the wlr bands match wlr's own order.

### Window frames

A window frame is the titlebar, border and shadow ring the compositor draws around a window. It is a window-targeted layer named `gnoblin.window_frame` (Kieran 14). It sits in `windows`, above its target and below the `top` bars.

A layer has two areas. The paint area is what it draws. The input area is where it takes pointer input. The areas are separate, so a frame can paint over a client's titlebar while the drag area still moves the window.

- SSD: the compositor draws the frame around the window. Its paint and input areas are the same.
- CSD: the client draws its own titlebar. The frame may paint over it, and its input area leaves the drag region out, so a drag on the titlebar still moves the window.

The built-in frame is the default. Any provider may register its own frame through the same registry, following the role, provider, built-in default and config override pattern.

"Window frames need to use the layers system as well" (Kieran 14).

### Ordering and follow rules

- Within a band, the newest claim sits on top, unless `order` names a layer in the
  same band.
- A window-scope layer sits directly above its target in the stack, and below the `top` bars (Kieran 3). Raising the target
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

Four Gnoblin claims and three built-in surface kinds use the registry. Each replaces existing code or is new.

| Layer                         | Scope                                                                 | Band and order                         | Input                                                                   | Registered by                                                                         | Replaces or adds                                                                            |
| ----------------------------- | --------------------------------------------------------------------- | -------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `gnoblin.portals`             | `window` if parent resolves, else `output`                            | `windows`, above target; else `system` | `modal-to-target` if parent resolves, else `modal-to-output` (Kieran 1) | `gnoblin-portal`, registered by the compositor's portal service, with `secure = true` | `Modal` and overlay slot (`modal.rs`, `frame.rs:236`); `dialog_placement` (`portal.rs:310`) |
| `gnoblin.alerts`              | `window`                                                              | `windows`, above target                | `modal-to-target`                                                       | `gnoblin-compositor` [INFERENCE]; content per Kieran 4                                | New. Detection exists (`session.rs:72-73`); kill exists (`server.rs:1260`).                 |
| `gnoblin.window_menu`         | none in core; an extension's window-scope layer, `<provider>.<local>` | `windows`, above target                | `grab`                                                                  | Extension (D29). Shipped default: `gnoblin.window_menu`, Lua                          | New. Replaces Bingux `WindowMenu.qml` (which subscribes to the signal).                     |
| `gnoblin.window_frame`        | `window`                                                              | `windows`, above target, below `top`   | Paint and input areas (see Window frames)                               | `gnoblin-compositor`; any provider may register its own (Kieran 14)                   | New. Frames are not in the layer model today.                                               |
| `gnoblin.surface_toplevel`    | `window`                                                              | `windows`, one per window              | `gnoblin-wm`                                                            | Existing window stack (`stack.rs:21-26`). Adds a record per window.                   |
| `gnoblin.surface_layer_shell` | `output`                                                              | `background` to `overlay`              | `gnoblin-wayland`                                                       | Existing wlr code (`zwlr_layer_shell_v1.rs:482-486`). Adds a record per surface.      |
| `gnoblin.surface_x11`         | `window`                                                              | `windows`, X stacking                  | `gnoblin-xwayland`                                                      | Existing Xwayland windows (`xwayland.rs:501`). Adds a record per window.              |

Every built-in uses namespace `gnoblin`. The column names the crate that registers the layer.

### portals

- The compositor's portal service registers `portals` for the request. The relay
  `xdg-desktop-portal-gnoblin` does not register. It passes `parent_window` over the
  existing portal IPC, typed (see Cutover).
- Output scope (today's behaviour) applies while no parent resolves, and in `system`. It blocks that output (Kieran 1).
- With a resolved parent, the layer sits above that window and follows it. The
  dialog is centred on the parent's rectangle, not the output's.
- `x11:` resolves through `gnoblin-xwayland` [INFERENCE]. `wayland:` has no resolver
  today. It is blocked until Phase 5 chooses one (Kieran 6).
- If the parent closes while the dialog is open, the layer ends `target-gone`, and
  the request continues on output scope, logged.
- One live `portals` dialog per output. Others queue (today: one `active_request`).
- The portal service declares `secure = true` when it registers `portals`. The compositor does not force it. It is the provider's own declaration, the same flag an extension would set. The user can still change it in config (see Invariants).

### alerts

`alerts` is the compositor's generic layer for any alert it raises. The property and the reason stay `unresponsive`, because they say why the alert exists.

- The compositor registers `alerts` when a window's `unresponsive` becomes true
  (`session.rs:359-361`), and removes it when the window responds or closes.
- The content offers Wait and Force Quit. Force Quit calls `window.kill`
  (`server.rs:1260`). The built-in painter draws it by default. Any provider may
  register its own through the same registry (Kieran 4).
- It blocks its target only (Kieran 2). A hung window cannot take input,
  so the rest of the desktop keeps working.

### window_menu

- The core has no window-menu layer. It emits `menu_requested` (D29), and a subscribing extension registers a window-scope layer above the target, with input policy `grab` until dismissed.
- The shipped extension is `gnoblin.window_menu`, enabled by default. Disable it in the extension configuration (D22) to replace it.
- Two enabled subscribers both show a menu. Users enable one.

## Registration

Three entry points share one registry and one set of rules. Built-in Rust providers use the same `register` as extensions.

### Rust registry

A new foundation crate, `gnoblin-layers`, holds `LayerRegistry`, `LayerSpec`,
`Band`, `LayerScope`, `InputPolicy`, and the ordering and follow rules. It depends only on
`gnoblin-types` [INFERENCE: add it to `FOUNDATION` at `xtask/src/deps.rs:34`].
`gnoblin-wm` depends on it for stack entries. `gnoblin-compositor` registers the
Gnoblin claims. The portal crates do not depend on it. The compositor's portal
service registers `portals`, so the portal-layering rule holds (`xtask/src/deps.rs:454-488`).

`gnoblin-wayland` and `gnoblin-xwayland` also depend on it, to register the surface kinds [INFERENCE: check that `xtask/src/deps.rs` allows the edge].

```rust
// gnoblin-compositor only. The spec gives the local name; the registry adds the namespace.
let spec = LayerSpec {
    name: "portals".into(),
    scope: LayerScope::Window(parent),          // or LayerScope::Output(output) when no parent resolves
    band: Band::Windows,
    order: Order::AboveTarget,
    input: InputPolicy::ModalToOutput,
    ..LayerSpec::default()
};
let handle = registry.register(LayerProvider::Gnoblin, spec)?; // handle.name is "gnoblin.portals"
```

### Lua

`gnoblin.layers.register{...}` returns a handle object. Its `name` is the full name: an extension `example` that passes `name = "note"` reads back `ext.example.note`. The handle has the object model's three kinds of member:

- Properties, read-only: `name`, `provider`, `band`, `scope`, `target`, `state`.
- Signals: `closed(layer, reason)` and `changed(layer, property)`.
- Method: `remove()`, which returns an `Operation` per the object model.

The pattern matches `LayerSurface` (`docs/public/gnoblin.d.lua:758`).

```lua
local note = gnoblin.layers.register {
  name = "note",
  scope = "window",
  target = gnoblin.windows.focused(),
  input = "passive",
  order = { above = "base" },
}
print(note.name, note.provider, note.state)
note:on("closed", function(layer, reason)
  print("ended", reason)
end)
note:remove()
```

Lua may register `passive` or `modal-to-target` layers, window-scope in `windows`
or output-scope in `top`. `overlay` needs a host grant. It cannot register in `system`, `lock` or
`cursor`, and cannot use `grab` or `modal-to-output` (Kieran 5).

The surface kinds are Rust-only. Their records follow Wayland objects, not Lua calls.

### Extension manifest (D22)

An extension declares its layer powers in `extension.toml`. Host configuration grants
a subset.

```toml
[powers.layers]
names = ["panel"]                    # local names; full name is ext.<id>.<local>
band_max = "top"                     # "top" (default); "overlay" only by host grant (Kieran 8)
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

| Case                                | Behaviour                                                                       |
| ----------------------------------- | ------------------------------------------------------------------------------- |
| Provider crash or disconnect        | Its layers end `provider-gone`. Pending portal requests settle `error(...)`.    |
| Extension unload                    | As a disconnect. All its layers end in one step.                                |
| Repeated local name in one provider | `register` refuses with `name-taken`. Other providers are unaffected.           |
| Parent never maps                   | Unresolved `wayland:` or `x11:` falls back to output scope, logged.             |
| Parent closes, dialog open          | Layer ends `target-gone`. Request continues on output scope.                    |
| Parent closes, alert open           | Layer ends `target-gone`. No alert remains.                                     |
| Target unresponsive, then responds  | `alerts` ends with reason `target-responsive`.                                  |
| Target moves to another output      | Window-scope layers move with it.                                               |
| Layer requested above the lock      | Refused. Nothing registers in `lock`.                                           |
| Session locks                       | `hide` layers are removed. Pending portal requests settle `error(locked)`.      |
| Client surface destroyed            | Its record ends `surface-gone`. Its popups end with it.                         |
| Many simultaneous requests          | One live `portals` per output. Others queue.                                    |
| Limits                              | Proposed: 32 live layers per provider, 8 per target. [INFERENCE: not measured.] |

## Security

- The compositor sets `provider` from the registering crate or the connection
  identity. A caller cannot set or spoof it.
- The compositor draws the attribution label ("requested by ...") in its own frame,
  on every `modal-*` and `grab` layer. Provider content sits in its own surface and
  cannot cover the label [INFERENCE: not built yet].
- `gnoblin` is the only reserved provider id. An extension with that id is refused at load, so no extension name starts `gnoblin.`. Local names such as `surface_toplevel` are ordinary under an extension's own namespace.
- `parent_window` is a client's hint. The compositor places a portal dialog over a
  resolved window only when that window belongs to the requesting client (proposed;
  Kieran 9). Otherwise it falls back to output scope.
- Nothing sits above the lock, except layers on the compositor's allow-list. The lock screen is the only entry. A provider cannot draw over the lock.
- `secure` is set by the layer's provider on its own layer, or by the user in config. Nothing else sets it. The lock surface is the one built-in exception (see Invariants).

## Capture paths

The compositor sees two kinds of capture. Client protocols are open to any client. Compositor-internal capture is started by the compositor itself.

- Client protocols: `wlr-screencopy`, `ext-image-copy-capture`, and the portal `Screenshot` and `ScreenCast` requests relayed by `xdg-desktop-portal-gnoblin`. The portal knows one frame from a stream. A client on the raw protocols can do either, so the protocol cannot tell a screenshot from a recording.
- Compositor-internal: the screenshot key and `gnoblinctl screenshot`. Neither exists in code yet.

One rule covers all of them: secure content is never drawn into any capture. The user's display is unchanged. There is no "user-initiated" exemption. The compositor cannot prove who is behind a request, so an exemption would be the bypass. To screenshot a secure dialog, the user sets `secure = false` for that layer in config.

Secure nodes are replaced in a capture by one treatment, set once for the whole compositor (`capture.secure-treatment`, see Options).

- `mask` (default): fill the node's silhouette with one neutral colour. The silhouette is the node's alpha mask, as D26 defines it for shaped windows, and a full rectangle for a layer with no alpha. The capture never shows more than the display did, and a viewer can see that something is hidden.
- `omit`: draw nothing for the node. The capture shows whatever lies underneath. That can be content the display did not show, because the node covered it. The underlay is non-secure by its own flag, so the rule allows it. A secure dialog over an unsecured window therefore lets that window's content into the capture.

Decided: `mask` is the default (Kieran 13, "yeah mask"). Its cost is a neutral shape in screenshots and an alpha mask per secure node per capture frame, which D26 already keeps for shaped windows. `omit` gives clean captures with no mask cost, but a secure dialog over an unsecured window lets that window's content into a capture, which the display did not show. Config switches to `omit` when that trade is acceptable.

Effects the node draws outside its silhouette, such as a shadow, are not masked. A shadow carries the shape, not the content [INFERENCE].

Capture paths found in code, checked 2026-11-06 [INFERENCE: not traced end to end]:

| Path                     | Where (`path:line`)                                                                                    | Honours `secure` today                  | Work needed                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------------------------- | ---------------------------------------------------------------------------- |
| Backend readback         | `gnoblin-backend/src/lib.rs:463`; `render/gles/mod.rs:476`                                             | No. Reads pre-overlay pixels only.      | Compose the capture view with secure nodes masked or omitted. The main item. |
| Screenshot queue         | `gnoblin-wayland/src/frame.rs:141`; `server.rs:875`                                                    | No. Serves the readback above.          | Inherits the readback fix.                                                   |
| Capture embargo          | `gnoblin-capture/src/table.rs:277`, `:344`                                                             | Partly. Checks embargo per target only. | Give the capture a secure-free source.                                       |
| wlr-screencopy           | `gnoblin-wayland/src/protocols/zwlr_screencopy_v1.rs:79`                                               | No.                                     | Inherits the fix if it keeps reading through the table.                      |
| ext-image-copy-capture   | `protocols/ext_image_copy_capture_v1.rs:32`; global `capture_shared.rs:249`                            | No.                                     | As above.                                                                    |
| Portal Screenshot        | `xdg-desktop-portal-gnoblin/src/iface/capture.rs:62`; compositor side `gnoblin-compositor/src/portal/` | No. Compositor does not read `secure`.  | Request the secure-free frame.                                               |
| Portal ScreenCast        | `xdg-desktop-portal-gnoblin/src/iface/capture.rs`; `gnoblin-pipewire-export/src/lib.rs:292`            | No.                                     | Stream frames come from the table. Re-check `secure` each frame.             |
| Shell Screenshot compat  | `compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/service.rs:182`                                 | Refuses. Returns `unsupported` today.   | None now. If enabled, route through the portal path.                         |
| Mutter ScreenCast compat | `compat/gnoblin-compat-dbus-mutter-screencast/src/service.rs:219`                                      | No.                                     | Routes through PipeWire export; inherits the fix.                            |
| Screenshot key           | None in code                                                                                           | Not applicable.                         | Build it with the rule. No key exists yet.                                   |
| `gnoblinctl screenshot`  | None in `crates/gnoblinctl`; only `shortcut capture` exists                                            | Not applicable.                         | Build it with the rule.                                                      |
| Legacy C portal          | `src/portal/backend/gnoblin-screenshot-capture.c`                                                      | Not checked.                            | Pre-rewrite tree. Confirm it is unused, or remove it.                        |

The readback is the one path that needs real work. The others inherit its fix only if they keep reading frames through the table, which is the intent [INFERENCE].

## Invariants and options

Two kinds of rule. An invariant is enforced by the compositor, and no provider or extension can change it. An option is a value a provider sets on its own layer, within its grant. Invariants are policy for Kieran to decide. Options only need defaults.

### Invariants (A)

- **Parent ownership.** A `parent_window` must name a window of the requesting client. Otherwise the request is refused and falls back to output scope.
- **Band ceiling.** A built-in provider uses the bands its kind allows. An extension stays at or below `band_max`, and never enters `system`, `lock` or `cursor`.
- **Input grant.** An extension requests `passive`. `modal-to-target`, `modal-to-output` and `grab` need a manifest power and a host grant. The compositor draws the attribution label on every modal or grab layer.
- **Secure content is never captured.** A layer with `secure = true` is never drawn into any capture, on any path in Capture paths. No grant, manifest power, provider or config setting turns that off. The user's display still shows it.
- **Only the owner or the user changes `secure`.** The provider sets it on its own layer. The user sets it in config, per layer name or per window rule, in either direction. That includes built-in layers such as `portals`; config is the only way to change a provider's declared value. No provider can change another provider's layer, and an extension cannot set it on a Gnoblin layer.
- **Names are namespaced by provider.** A full name is `<provider>.<local>`, so collisions across providers are impossible by construction. Within one provider, a repeated local name is refused with `name-taken`. The provider `gnoblin` is reserved, and an extension with that id is refused.
- **The lock surface is secure.** It is the one built-in exception to the default of `false`, because it is compositor-owned and shows the unlock prompt over the locked session. No provider or config can remove it from capture.
- **Lock.** Nothing draws above the lock except the compositor's allow-list. Extensions cannot join it.
- **Count limits.** Each provider and each target has a maximum number of live layers. The numbers are proposed; the limits themselves are not optional.

### Options (B)

| Option                         | Set by                     | Default                                                        | Bounded by                                                                         |
| ------------------------------ | -------------------------- | -------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Scope: `output` or `window`    | Provider                   | The kind's default                                             | The kind's allowed scopes                                                          |
| Order within its band          | Provider                   | Per the Ordering rules                                         | Band ceiling; Gnoblin claims stay above                                            |
| `passive` or `modal-to-target` | Provider                   | `passive`                                                      | Invariant "Input grant"                                                            |
| `secure`                       | Provider; user in config   | `false`, unless the layer declares a value                     | Invariant "Only the owner or the user changes `secure`"; lock surface fixed `true` |
| Dim the target                 | Provider                   | Off (proposed)                                                 | Compositor draws the dim; target untouched                                         |
| Content kind                   | Provider                   | Surface, or compositor-painted for `alerts`                    | Lua: compositor-painted unless `wayland` power (Kieran 5)                          |
| Input scope of `portals`       | Provider (`portals` layer) | `modal-to-target` if a parent resolves; else `modal-to-output` | Invariant "Input grant"; Kieran 1                                                  |

An option never widens an invariant. A request outside the grant is refused and logged.

**Config shape (proposed).** `config.toml` gets a `layers` section: a `record-map` keyed by full layer name, each record with one `secure` bool. A dotted name must be quoted, or TOML reads it as nested tables: `[layers."gnoblin.portals"]` with `secure = false`. This document writes the path as `layers."gnoblin.portals".secure`.

The schema's record-map ids are `hex-pair`, `renderer` and `serial`. None takes dots, so layer names need a new id kind, `layer-name`, with the grammar `<provider>.<local>`. [INFERENCE: grep found no Rust id validator under `crates/gnoblin-config/src`.]

A window rule takes `secure = true|false`, spelled as in `docs/config/window_rule.md`, which uses snake_case rule keys.

**Capture config (proposed).** A `capture` section holds one leaf, `secure-treatment = "mask" | "omit"`, default `mask`. Schema field keys are kebab-case (`delay-ms`, `privacy-screen`), so the leaf is spelled `secure-treatment`, not `secure_treatment`.

**Default.** `secure = false` applies only to a layer that does not declare a value. The portal crates declare `secure = true` on `gnoblin.portals` when they register it. The lock surface is fixed `true`, and config cannot change it.

## Cutover

Each path below moves to the registry, with no shim.

- `crates/gnoblin-render-frame/src/frame.rs:236`: the `overlay` slot goes. `layers` (`:235`) becomes the one ordered list, walked in registry order. `cursor` (`:237`) stays. A breaking change to the frame type, shared by every renderer.
  The capture view is a second walk of the same list. It draws a masked silhouette in place of each secure node, not the node itself. Cost: one alpha-mask fill per secure node per capture frame, and the mask itself, which D26 already keeps for shaped windows. A rectangle needs no mask.
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
- `xdg_shell.rs` (toplevels; popups at `:59`): the window manager registers `surface_toplevel` for each window, so `xdg_shell.rs` only reports the surface. The Xwayland window code (`xwayland.rs:501`) registers `surface_x11`. Popups follow their root instead of their own stacking.
- Docs: `docs/shell-api/layer-surfaces.md` (the `gnoblin.layers` section) and
  `docs/public/gnoblin.d.lua` (`register` and the handle). `layers.list` keeps its
  record shape, plus a `band` filter (Kieran 7).
- Bingux, out of tree: `WindowMenu.qml` and the not-responding dialog move to subscribers and providers.

## Tests

- Band order: `top` stays below `system`, and `system` stays below `lock`.
- A window-scope layer sits directly above its target. A target raised above other
  windows takes the layer with it.
- A window-scope layer follows its target through move, minimise, workspace switch
  and close, and ends `target-gone`.
- One live `portals` dialog per output; a second request queues.
- A repeated local name within one provider is refused with `name-taken`.
- A provider disconnect ends its layers and sends `closed`.
- `secure` defaults to `false` for every layer, extension layers included.
- A `secure = true` layer is replaced under each path in Capture paths by the `secure-treatment` option, and still drawn on the display.
- A config override flips a layer both ways: `layers."<full name>".secure` hides a `false` layer, and `false` shows a `true` layer such as `gnoblin.portals`.
- A window rule's `secure` flips a window or layer surface both ways.
- A provider cannot change another provider's layer's `secure`. An extension cannot set it on a Gnoblin layer.
- The lock surface stays secure. A config override on `gnoblin.lock` is refused and logged.
- Two providers may register the same local name: `ext.a.note` and `ext.b.note` both live.
- An extension with id `gnoblin` is refused at load. No extension can register under `gnoblin.`.
- One provider cannot change another provider's layer.
- Mask follows a shaped window: a secure window with an oval alpha silhouette captures as a filled oval in the neutral colour. Pixels outside the oval match the underlay.
- Omit reveals underlay by design: a secure node over a non-secure window captures that window where the node sat. The test asserts it, so the trade-off stays visible.
- Under `mask`, a capture differs from the display only inside secure silhouettes.
- The default `capture.secure-treatment` is `mask`. An unknown value is refused.
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
- A `portals` dialog with a resolved parent blocks that parent only. With no usable parent it centres on the output and blocks that output.
- A CSD titlebar covered by a frame's paint area still moves the window when dragged, because the frame's input area excludes it.

## Phases

1. `gnoblin-layers` crate: the registry, bands, ordering and follow rules, the `secure` field (default `false`) and its owner check, and their tests. No behaviour change.
2. `gnoblin-wm` stack gains window-scope entries above their targets. `StackLayer` keeps `Normal` and `Fullscreen`. Follow rules and stack tests.

    Implemented: `Wm::register_layer` attaches a window-scope layer above its target. The entry hides, shows and ends with the target, and `Broken::LayerEntryOrphan` reports one that no longer follows its window. A Windows-below-System paint test is deferred to the registry, because the stack orders only window-scope layers.

3. The frame takes one ordered list. The `overlay` slot goes; `cursor` stays. `Modal` becomes the `portals` input policy. The capture view masks or omits secure nodes (`capture.secure-treatment`) and replaces the pre-overlay readback (Capture paths).

    Plan (step order, one slice per push):
    (a) `gnoblin-render-frame/src/frame.rs`: `OutputFrame` takes one ordered `Vec` of entries. `Layer::Surface`, `Layer::Solid` and an overlay entry kind. The `overlay` field and `set_overlay` stay until (c).
    (b) Callers build that list in bottom-to-top order from `Wm::layer_order` plus the built-in entries: `gnoblin-backend/src/render/compose.rs` (`push_layer`), `gnoblin-wayland/src/frame.rs` (`OutputFrame::new`), `gnoblin-wayland/src/state.rs` (`frame()`, the `set_overlay` call near L2716).
    (c) Remove the `overlay` slot. The cursor stays a separate slot. Update `gnoblin-render-gles/src/lib.rs` (L545, L547, L952), `gnoblin-render-software/src/lib.rs` (L93, L258, L396, L442, L476, tests L972-1030), `gnoblin-render-vulkan/src/frame.rs` (L18, L599, L769, L888), `plan.rs` (L182-187), `tests.rs` (L373).
    (d) `Modal` (`gnoblin-wayland/src/modal.rs`, `state.rs` L508, L2225, L2259, L2740, `server.rs` L929, L1113) becomes the `portals` input policy. The overlay it carries is built in `gnoblin-compositor/src/ipc/portal.rs` (L443-535), and `ipc/server.rs` L1757 and `gnoblin-wasm` bindings L604 move with it.
    (e) Capture: `gnoblin-backend/src/lib.rs` readback (L463) and `gnoblin-render-software` readback masks or omits `secure` nodes, governed by `capture.secure-treatment`. Test: a secure node never appears in a capture, on every path.
    Each slice compiles on Agent37 after rebase, then pushes. Stop at a green pushed slice.

4. Built-in surface kinds in `gnoblin-wayland` and `gnoblin-xwayland`: `surface_toplevel`, `surface_layer_shell` (four bands) and `surface_x11`. Popups follow their root. The parity test changes its lookup key only.
5. `parent_window` is typed. `x11:` resolves through `gnoblin-xwayland`. `wayland:` needs one of: (a) a Smithay change exposing handle-to-surface lookup on `XdgForeignState`; (b) a Gnoblin table filled when the exporter mints a handle, which needs a wrapper around Smithay's exporter; (c) the dialog becomes a Wayland child toplevel that imports the handle, which changes it from an overlay to a surface. [INFERENCE: (a) is smallest.]
6. `gnoblin.window_frame`, built in, with its paint and input areas (see Window frames).
7. `menu_requested` signal and `request_menu` on `window` (D29). The shipped `gnoblin.window_menu` extension follows, enabled by default. No core menu layer.
8. `gnoblin.alerts`, with its content provider.
9. Lua `gnoblin.layers.register`, its handle and signals, and the docs. `gnoblin.window_rule` gains `secure`. The config schema gains a `layers` section keyed by full layer name, with a new `layer-name` id kind, and a `capture` section with `secure-treatment`.
10. Extension `[powers.layers]` through D22.

Each phase names the tests that fail without its rule.

## Needs Kieran

Open items only. Numbers are stable, so cross-references still hold. Each open item is marked (A) if it is a policy choice about an invariant, or (B) if it sets a default for an option.

6. (A) How to resolve `wayland:<handle>`. It enforces parent ownership (invariant 1). Plain version: a portal request names its window with a token from the xdg-foreign protocol, and Gnoblin must turn that token into a window. Smithay keeps the table private. Options: (a) a small Smithay change exposing the lookup, carried until upstream takes it; (b) our own copy of the table, wrapping Smithay; (c) the dialog becomes a real child window. Lean (b), now: Kieran is leaning towards zero Smithay patches, so Gnoblin owns the handle table and wraps Smithay's exporter. Until it lands, dialogs centre on the output. Phase 5.
7. (A) Whether the compositor checks that a `parent_window` belongs to the requesting client before placing a dialog over it. Invariant 1. Proposed: yes, and it is not optional.
8. (B) Bar visibility while a fullscreen window is on its output: hidden (proposed). The alternative lifts fullscreen windows above `top`, as wlroots does, which breaks one band per record.
9. (B) Override-redirect X11 windows: X's own stacking inside `windows` (proposed), or lifted to `top` as menus.

**15.** (A) Is output-scope fallback acceptable for unverified parents, or deny? Today a dialog whose parent
cannot be verified opens on the output and the app still gets its consent answer. Denying instead
would refuse those requests. Until Kieran decides, the fallback stays.

**16.** (A) How a portal dialog may be placed over a parent window when the app cannot be verified. Today every request uses output scope. Options:

- (a) Keep output scope for every request until a real attribution source exists. Pro: no spoofing risk, nothing new to trust. Con: dialogs never centre on their window, and the parent-window code stays unused.
- (b) Treat the xdg-foreign handle as a bearer capability: random, unguessable, known only to the exporter. Honour it when it resolves, and always show the requesting app's label in the dialog. Pro: works for host apps. Con: a leaked handle lets another app place a dialog over that window. Risk: the label is the only signal, and the app id it shows may be empty.
- (c) Honour the parent only when `app_id` is non-empty and matches the exporter's toplevel `app_id` (sandboxed apps only). Pro: a real identity check where Flatpak supplies one. Con: host apps (empty `app_id`) get output scope, and the check trusts `app_id`, which the docs do not name as an attribution.

### Decided

- Kieran 1 (A): a `portals` dialog blocks its parent window only. With no usable parent, it is centred on the output and blocks that output. Kieran: "portals usually relate to the window, that's how they work on most setups". This replaces the earlier rule that consent always blocks its output.
- Kieran 2 (B): an `alerts` dialog blocks its window only. Kieran: "yes window only".
- Kieran 3 (B): window-targeted layers sit above their window and below the `top` bars.
- Kieran 4 (B): the built-in painter draws `alerts` by default. Any provider may register its own through the same registry. Kieran: "built in. But anyone can register their own".
- Kieran 7 (B): `layers.list` takes a `band` filter. A band is one of the fixed stacking levels, and the filter lists the layers in one of them. This is a one-line addition.
- Kieran 8 (A): extensions may use the `top` band. The `overlay` band needs a host grant. Kieran: "who cares?", so the lower ceiling is the default.
- Kieran 10 (B): the toplevel kind is `surface_toplevel`, matching `xdg_toplevel`. Kieran: "thats fine".
- Kieran 13 (B): secure content is masked in captures by default. `capture.secure-treatment = "omit"` switches to omit. Kieran: "yeah mask".
- Kieran 14 (new): window frames are window-targeted layers with separate paint and input areas. See Window frames. Kieran: "window frames need to use the layers system as well".
- Kieran 5 (A): a Lua extension describes what to show as a draw list of primitives (text, buttons, icons, shapes), and Gnoblin paints it. Custom pixels or rendering need a WASM extension, or a separate-process extension with its own surface. Kieran: "you'd just do a wasm extension if you wanted more power". Describing is safe, because an extension cannot fake a system dialog.

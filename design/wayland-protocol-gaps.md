# Wayland protocol gaps

Status: lane ProtoMissing. The protocols below were written but **not built or run**: no Agent37 box came free during
the lane (the quota was held by other lanes), and the rules allow builds on a box only. Everything here is on branch
`js-ProtoMissing` and needs one gate run before it goes to `rust-compositor`. The observed registry the brief asks for
is therefore not in this note; `crates/gnoblin-wayland/tests/protocol_list.rs` prints it with
`GNOBLIN_PRINT_REGISTRY=1 cargo test -p gnoblin-wayland --test protocol_list -- --nocapture`.

## What was written

| Protocol                                      | Module                                 | Test                             | Access                                   |
| --------------------------------------------- | -------------------------------------- | -------------------------------- | ---------------------------------------- |
| `zwlr_virtual_pointer_manager_v1` (v2)        | `protocols/zwlr_virtual_pointer_v1.rs` | `tests/virtual_pointer.rs`       | privileged (`Global::VirtualPointer`)    |
| `org_kde_kwin_server_decoration_manager` (v1) | `protocols/kde_server_decoration.rs`   | `tests/kde_server_decoration.rs` | everyone, behind `kde-server-decoration` |
| `org_kde_kwin_appmenu_manager` (v2)           | `protocols/kde_appmenu.rs`             | `tests/kde_appmenu.rs`           | everyone                                 |
| `ext_workspace_manager_v1` (v1)               | `protocols/ext_workspace_v1.rs`        | `tests/ext_workspace.rs`         | privileged (`Global::Workspace`)         |

- **Virtual pointer.** Smithay serves the objects. `zwlr_virtual_pointer_v1.rs` turns each event into the backend's
  `InputEvent` and calls `Host::input`, so a virtual pointer meets the lock card, a modal, pointer chords, grabs and
  constraints as a mouse does. Scroll frames that a wheel event cannot state (finger or continuous source, axis stop)
  go through the same gates and the same router as a wheel step, as `scroll::Scroll` (`State::input_scroll`,
  `State::route_scroll`). The manager is privileged, unlike the virtual keyboard: it clicks every client's buttons, and
  the brief asks that an unprivileged client be refused. A sandbox is refused either way.
- **KDE server decoration.** A request becomes the window's `decoration_wanted` through the same negotiation as
  `xdg-decoration` (`State::apply_decoration`), and each answer returns as a `mode` event. The existing
  `protocols.kde-server-decoration` toggle switches it off. The dependency `wayland-protocols-plasma` (MIT) was added.
  `Cargo.lock` was edited by hand (entry and checksum from the crate cache); `cargo` on the box may rewrite it.
- **KDE appmenu.** `set_address` is validated as a DBus name and object path and published on the window node as the
  new read-only string properties `appmenu_service` and `appmenu_path` (`window.toml`; the composer publishes them with
  the node and on change). The property change is the tree's change event. The object's end (release or disconnect)
  empties both. **Generated files have not been regenerated**: run `cargo xtask gen-sdks` and update
  `crates/gnoblin-schema/tests/golden/catalog.txt` and the two `abi.lock` files before gating.
- **ext-workspace.** The compositor keeps no workspaces, so the list is the declared workspaces (`Seat.workspaces`,
  or the one implicit workspace `1`) and the id the shell writes to the new seat property `active_workspace`. It
  advertises **no capabilities**: `activate`, `deactivate`, `remove`, `assign` and `create_workspace` are ignored,
  because a client's wish has no route to the shell (see below). The stock shell does not write `active_workspace`
  yet, so a session with several workspaces reports none active until it does (a single workspace is always active).

## What was not done, and why

- **ext-workspace requests.** Switching needs a request from the compositor to the shell. The only compositor-to-shell
  event kinds are `TreeFrameInput` variants, each of which crosses the tree wire (`tree_host.rs`), the tree client, the
  core implementation and the JS host. That is a design change, and the stock shell's own workspace switching would
  have to answer it. Estimate: a `workspace_request` event kind through those four layers plus the stock handler,
  about two days.
- **`ext_transient_seat_manager_v1`.** Not implemented and not advertised. A transient seat is a `wl_seat` global
  created per request, whose registry `name` the compositor must send in `ready`. The server API exposes no numeric
  name for a `GlobalId` (only its `Debug` text). More basically, every injection path (virtual keyboard and pointer,
  the key router, grabs, focus) is bound to the one main seat, so a second seat would receive no input; always
  answering `denied` would advertise a protocol that does nothing. Prerequisite: seat-addressed input routing. Estimate:
  three to four days.
- **`wp_drm_lease_device_v1`.** Smithay has `wayland::drm_lease` (929 lines) and nothing in this tree uses it. The
  state needs the `DrmNode` and connector list and a `DrmLeaseBuilder` made from the live `DrmDevice`, which only
  `gnoblin-backend-drm` holds, so the `Backend` trait needs a lease API (list leasable connectors, create and revoke a
  lease, give the connector back), the output policy must withdraw a leased connector, and the session must survive a
  lease ending while a client crashes. It is a VR feature (Monado, WiVRn). Estimate: five to seven days with a
  real-hardware test.
- **`wp_color_management_v1`.** Smithay has no module. The XML is in `wayland-protocols` 0.32.13 (staging). The
  protocol needs image descriptions, a render path that converts per surface (the renderers have a colour matrix, not
  colour spaces), output colour characteristics from EDID (the backend reports none), and HDR metadata. Estimate: two
  to three weeks, and it depends on the renderer work.
- **`xdg_session_management_v1`.** Smithay has no module; the XML is in `wayland-protocols` 0.32.13. It needs per-app
  session storage keyed by the session id and a restore path in window placement. Estimate: three to four days.
- **Smithay modules not asked about and not advertised:** `pointer_warp`, `commit_timing` and `drm_syncobj`. The first
  two are small; explicit sync (`drm_syncobj`) touches every renderer's buffer import.

## Gate list for the next run

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo xtask gen-sdks` (then `--check`),
`cargo xtask abi-check`, `cargo xtask dupes --deny`, `cargo test -p gnoblin-wayland` (new files: `virtual_pointer`,
`kde_server_decoration`, `kde_appmenu`, `ext_workspace`, `protocol_list`, and `access`), `cargo test -p
gnoblin-schema`, `cargo test -p gnoblin-js`. `protocol_list` fails until `docs/wayland-protocols.md` has the five-column
table its parser reads (interface, version, what it does, `Everyone` or `Restricted`, notes; `Conditional:` notes for
globals only some sessions have); that table is generated from the observed registry and was not written.

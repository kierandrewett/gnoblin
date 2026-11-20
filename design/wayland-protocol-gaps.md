# Wayland protocol gaps

Status: lane ProtoMissing. Four protocols are served as crates under `crates/protocols/`. The rest of the
list the owner asked about is not built, and this note says why and what each would cost.

## Served now

| Protocol                                      | Crate                                            | Test                             | Access                                |
| --------------------------------------------- | ------------------------------------------------ | -------------------------------- | ------------------------------------- |
| `zwlr_virtual_pointer_manager_v1` (v2)        | `gnoblin-wayland-zwlr-virtual-pointer-v1`        | `tests/virtual_pointer.rs`       | restricted (`Global::VirtualPointer`) |
| `org_kde_kwin_server_decoration_manager` (v1) | `gnoblin-wayland-org-kde-kwin-server-decoration` | `tests/kde_server_decoration.rs` | everyone, behind a setting            |
| `org_kde_kwin_appmenu_manager` (v2)           | `gnoblin-wayland-org-kde-kwin-appmenu`           | `tests/kde_appmenu.rs`           | everyone                              |
| `ext_workspace_manager_v1` (v1)               | `gnoblin-wayland-ext-workspace-v1`               | `tests/ext_workspace.rs`         | restricted (`Global::Workspace`)      |

### Virtual pointer

- Smithay serves the objects. The crate turns each event into a call on the seat's own input path, so a virtual pointer
  meets the lock card, a modal, pointer chords, grabs and constraints as a mouse does.
- Relative motion keeps the fraction it cannot yet move by. Absolute motion maps onto the layout, or onto the output
  the pointer was made for.
- A finger or continuous scroll source and an axis stop cannot be stated by the backend's scroll event. They travel as
  one `Scroll` frame through the same gates and router as a wheel step.
- The manager is restricted, unlike the virtual keyboard: a virtual pointer can click another client's buttons. A
  sandbox is refused either way.

### KDE server decoration

- A request becomes the window's `decoration_wanted` through the negotiation `xdg-decoration` uses. Each answer
  returns as a `mode` event, and the manager hears `default_mode` again when the shell starts or stops drawing frames.
- The setting `protocols.kde-server-decoration` switches it off.

### KDE appmenu

- `set_address` is checked as a DBus name and object path, then published on the window node as the read-only
  properties `appmenu_service` and `appmenu_path`. The tree's change event is the signal a shell reads.
- Releasing the object, or the client going away, empties both properties. Two objects for one surface are served:
  the latest address wins, and releasing it leaves the other's in force.

### ext-workspace

- The compositor keeps no workspaces: the shell does. The list is the declared workspaces (`Seat.workspaces`, or one
  implicit workspace, `1`) and the id the shell writes to the new seat property `active_workspace`.
- No capabilities are offered. `activate`, `deactivate`, `remove`, `assign` and `create_workspace` are ignored.
- The stock shell does not write `active_workspace` yet. A session with several workspaces reports none active until
  it does. A session with one always reports it active.

## Not built

### Workspace requests

- Switching needs a request from the compositor to the shell. The only such events are `TreeFrameInput` variants, each
  of which crosses the tree wire, the tree client, the core implementation and the JS host.
- The work is a `workspace_request` event kind through those four layers, plus the stock handler. About two days.

### `ext_transient_seat_manager_v1`

- Not implemented and not advertised. A transient seat is a `wl_seat` global made per request, whose registry name the
  compositor must send in `ready`. The server API exposes no numeric name for a `GlobalId`.
- Every injection path, and the key router, grabs and focus, is bound to the one main seat. A second seat would
  receive no input, and always answering `denied` would advertise a protocol that does nothing.
- Prerequisite: seat-addressed input routing. About three to four days.

### `wp_drm_lease_device_v1`

- Smithay has `wayland::drm_lease`. Nothing in this tree uses it.
- It needs the `DrmNode` and a `DrmLeaseBuilder` made from the live `DrmDevice`, which only `gnoblin-backend-drm`
  holds. The `Backend` trait would need a lease API, the output policy would have to withdraw a leased connector, and
  the session must survive a lease ending when a client crashes.
- It is a VR feature (Monado, WiVRn) and needs a real-hardware test. About five to seven days.

### `wp_color_management_v1`

- Smithay has no module. The XML is in `wayland-protocols` 0.32.13, staging.
- It needs image descriptions and a renderer that converts per surface. The renderers have a colour matrix, not colour
  spaces, and the backend reports no EDID colour characteristics. HDR metadata is part of it.
- About two to three weeks, and it depends on renderer work.

### `xdg_session_management_v1`

- Smithay has no module; the XML is in `wayland-protocols` 0.32.13.
- It needs per-application session storage keyed by the session id and a restore path in window placement. About three
  to four days.

### Other Smithay modules

- `pointer_warp` and `commit_timing` are small. `drm_syncobj` (explicit sync) touches every renderer's buffer import.
  None was asked for and none is advertised.

## The advertised list

`docs/wayland-protocols.md` is the published list. `crates/gnoblin-wayland/tests/protocol_list.rs` starts a headless
session and reads the registry on the privileged and the ordinary socket. It compares both with the table, and the
table with `gnoblin_wayland_protocols::ALL`. A global cannot be added, re-versioned or made restricted without the
page changing.

Observed on a headless session with the privileged socket (version, then who sees it):

- Everyone: `wl_compositor` 5, `wl_subcompositor` 1, `wl_shm` 2, `wl_output` 4, `wl_seat` 9, `wl_data_device_manager` 3,
  `xdg_wm_base` 7, `xdg_activation_v1` 1, `zxdg_decoration_manager_v1` 1, `org_kde_kwin_server_decoration_manager` 1,
  `org_kde_kwin_appmenu_manager` 2, `xdg_wm_dialog_v1` 1, `zxdg_exporter_v2` 1, `zxdg_importer_v2` 1,
  `zxdg_output_manager_v1` 3, `xdg_system_bell_v1` 1, `xdg_toplevel_icon_manager_v1` 1,
  `xdg_toplevel_tag_manager_v1` 1, `zwp_primary_selection_device_manager_v1` 1, `ext_idle_notifier_v1` 2,
  `zwp_idle_inhibit_manager_v1` 1, `ext_session_lock_manager_v1` 1, `ext_background_effect_manager_v1` 1,
  `zwp_virtual_keyboard_manager_v1` 1, `zwp_input_method_manager_v2` 1, `zwp_text_input_manager_v3` 1,
  `zwp_keyboard_shortcuts_inhibit_manager_v1` 1, `zwp_pointer_constraints_v1` 1, `zwp_relative_pointer_manager_v1` 1,
  `zwp_pointer_gestures_v1` 3, `zwp_tablet_manager_v2` 1, `wp_cursor_shape_manager_v1` 2, `wp_viewporter` 1,
  `wp_fractional_scale_manager_v1` 1, `wp_content_type_manager_v1` 1, `wp_tearing_control_manager_v1` 1,
  `wp_single_pixel_buffer_manager_v1` 1, `wp_alpha_modifier_v1` 1, `wp_fifo_manager_v1` 1, `wp_presentation` 2,
  `wp_security_context_manager_v1` 1.
- Restricted: `zwlr_layer_shell_v1` 5, `zwlr_foreign_toplevel_manager_v1` 3, `ext_foreign_toplevel_list_v1` 1,
  `zwlr_screencopy_manager_v1` 3, `ext_image_copy_capture_manager_v1` 1,
  `ext_output_image_capture_source_manager_v1` 1, `ext_foreign_toplevel_image_capture_source_manager_v1` 1,
  `ext_data_control_manager_v1` 1, `zwlr_data_control_manager_v1` 2, `zwlr_gamma_control_manager_v1` 1,
  `zwlr_output_power_manager_v1` 1, `zwlr_output_manager_v1` 4, `ext_workspace_manager_v1` 1,
  `zwlr_virtual_pointer_manager_v1` 2.
- Only some sessions: `zwp_linux_dmabuf_v1` (a render node), `xwayland_shell_v1` and
  `zwp_xwayland_keyboard_grab_manager_v1` (Xwayland). A headless session has none of them.

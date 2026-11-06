//! Smithay protocol handlers, one module per protocol.
//!
//! Each module answers exactly one protocol's requests and translates them into
//! window-manager events; none of them decides placement, focus or stacking. The
//! split is what lets a protocol be added without touching another, and it is the
//! layout `design/rust-compositor.md` ("Project structure") asks for.
//!
//! | Module | Interfaces | What it does here |
//! |---|---|---|
//! | [`xdg_activation_v1`] | `xdg_activation_v1` | mints and redeems one-shot activation tokens |
//! | [`gnoblin_session_host_v1`] | `gnoblin_session_host_v1` | the host's channel to one hosted session: readiness, protection, logout and power requests |
//! | [`ext_image_copy_capture_v1`] | `ext_image_copy_capture_v1` | frames of an output or a window, streamed under the lock embargo and the access roster |
//! | [`ext_image_capture_source_v1`] | `ext_image_capture_source_v1` | names what an image-copy session captures: an output or a toplevel |
//! | [`zwlr_screencopy_v1`] | `zwlr_screencopy_manager_v1` | one-shot screencopy of an output or a region, under the same embargo and roster |
//! | [`capture_shared`] | none of its own | the pixel path, lock embargo and session table the three capture protocols share |
//! | [`wl_compositor`] | `wl_compositor`, `wl_subcompositor` | surfaces, subsurfaces and buffer commits |
//! | [`wl_shm`] | `wl_shm`, `wl_shm_pool` | shared-memory buffer pools |
//! | [`wp_content_type_v1`] | `wp_content_type_v1` | records whether a surface shows video or a game |
//! | [`wp_cursor_shape_v1`] | `wp_cursor_shape_manager_v1` | records the cursor a client names |
//! | [`zxdg_decoration_v1`] | `zxdg_decoration_manager_v1` | answers the client's preference (client-side, this slice draws no frames) |
//! | [`extension`] | none of its own | the seam a compat adapter's redirect registers through, and the operations it may ask the compositor for |
//! | [`zwlr_foreign_toplevel_management_v1`] | `zwlr_foreign_toplevel_manager_v1` | the window list a taskbar reads, and the requests it may make on each window |
//! | [`zwlr_gamma_control_v1`] | `zwlr_gamma_control_manager_v1` | per-output gamma ramps; a second client asking for an output another holds is refused |
//! | [`zwlr_output_management_v1`] | `zwlr_output_manager_v1` | the outputs a configuration tool reads, and the layout it may test or apply through the host |
//! | [`zwlr_output_power_management_v1`] | `zwlr_output_power_manager_v1` | per-output power, on or off; reports each change to its holder |
//! | [`ext_foreign_toplevel_list_v1`] | `ext_foreign_toplevel_list_v1` | the read-only, standardised form of the same list, keyed by a stable identifier |
//! | [`foreign_toplevel_shared`] | none of its own | the window record both toplevel lists are fed from, and what changed since it was last sent |
//! | [`wp_fractional_scale_v1`] | `wp_fractional_scale_manager_v1` | tells a surface the scale its output draws it at |
//! | [`zwp_idle_inhibit_v1`] | `zwp_idle_inhibit_manager_v1` | records the surfaces that keep the session awake |
//! | [`ext_idle_notify_v1`] | `ext_idle_notifier_v1` | answers `idled`/`resumed` from the session's tick counter |
//! | [`zwlr_layer_shell_v1`] | `zwlr_layer_shell_v1` | bars and docks: anchors, sizes, exclusive zones and keyboard interactivity |
//! | [`wl_output`] | `wl_output` | the output globals the layout created |
//! | [`zwp_pointer_constraints_v1`] | `zwp_pointer_constraints_v1` | activates, confines and releases a client's lock |
//! | [`zwp_pointer_gestures_v1`] | `zwp_pointer_gestures_v1` | creates the gesture objects |
//! | [`wp_presentation_time`] | `wp_presentation` | answers feedback with the frame's present time |
//! | [`zwp_relative_pointer_v1`] | `zwp_relative_pointer_manager_v1` | the vector motion a locked pointer reports |
//! | [`wl_seat`] | `wl_seat` | keyboard and pointer focus over surfaces |
//! | [`wp_security_context_v1`] | `wp_security_context_v1` | application sandboxes, marked before their first request |
//! | [`wl_data_device_manager`] | `wl_data_device_manager` | the clipboard and what follows focus |
//! | [`zwp_primary_selection_v1`] | `zwp_primary_selection_device_manager_v1` | the primary selection |
//! | [`ext_data_control_v1`] | `ext_data_control_manager_v1` | clipboard access for privileged clients |
//! | [`zwlr_data_control_v1`] | `zwlr_data_control_manager_v1` | clipboard access for privileged clients (wlroots) |
//! | [`ext_session_lock_v1`] | `ext_session_lock_v1` | the lock request, its cover, its surfaces and the input gate |
//! | [`zwp_keyboard_shortcuts_inhibit_v1`] | `zwp_keyboard_shortcuts_inhibit_manager_v1` | grants, refuses and revokes shortcut inhibitors |
//! | [`wp_single_pixel_buffer_v1`] | `wp_single_pixel_buffer_manager_v1` | creates the global; the buffer carries its own colour |
//! | [`wp_tearing_control_v1`] | `wp_tearing_control_v1` | records the presentation hint a surface asks for |
//! | [`zwp_text_input_v3`] | `zwp_text_input_manager_v3` | the text-input conversation |
//! | [`zwp_input_method_v2`] | `zwp_input_method_manager_v2` | the privileged input-method half of a text conversation |
//! | [`zwp_virtual_keyboard_v1`] | `zwp_virtual_keyboard_manager_v1` | the privileged virtual-keyboard half |
//! | [`wp_viewporter`] | `wp_viewporter` | creates the global; Smithay validates a viewport on commit |
//! | [`gnoblin_window_frame_v1`] | `gnoblin_window_frame_manager_v1` | frames a decoration renderer draws, their configures, regions and errors |
//! | [`xdg_shell`] | `xdg_shell` | toplevels, popups, configure and ack |
//! | [`wp_alpha_modifier_v1`] | `wp_alpha_modifier_v1` | the factor a surface commits multiplies its window's opacity when the composer draws it; a factor on a subsurface has no effect because subsurfaces are not drawn |
//! | [`wp_fifo_v1`] | `wp_fifo_manager_v1` | a client's commits are shown in order; barriers are released on each frame tick |
//! | [`xdg_dialog_v1`] | `xdg_wm_dialog_v1` | a modal child takes focus from its parent and the parent's focus is redirected to it until unset; the window manager floats the dialog above its parent |
//! | [`xdg_foreign`] | `zxdg_exporter_v2`, `zxdg_importer_v2` | names a toplevel by handle; the parent reaches the window manager as a role change, which floats the child and stacks it above the parent |
//! | [`xdg_toplevel_icon_v1`] | `xdg_toplevel_icon_manager_v1` | the icon's name and buffer count are stored per window and exposed in the session snapshot; not drawn |
//! | [`xdg_toplevel_tag_v1`] | `xdg_toplevel_tag_manager_v1` | the tag is stored per window and exposed in the session snapshot; placement does not read it |

/// Pixel path shared by the three capture protocols: frames, sources and sessions.
pub mod capture_shared;
/// `ext_background_effect_v1`: blur behind a client's own surface.
pub mod ext_background_effect_v1;
/// `ext_data_control_v1`: clipboard access for privileged clients (standard protocol).
pub mod ext_data_control_v1;
/// `ext_foreign_toplevel_list_v1`: the read-only, standardised toplevel list.
pub mod ext_foreign_toplevel_list_v1;
/// `ext_idle_notify_v1`: idle and resume notifications driven by the session tick counter.
pub mod ext_idle_notify_v1;
/// `ext_image_capture_source_v1`: the output and toplevel sources a capture names.
pub mod ext_image_capture_source_v1;
/// `ext_image_copy_capture_v1`: image-copy sessions that stream frames into client buffers.
pub mod ext_image_copy_capture_v1;
/// `ext_session_lock_v1`: the session lock request, its cover and its input gate.
pub mod ext_session_lock_v1;
/// The seam a compat adapter's Wayland redirect registers through.
pub mod extension;
/// The window record both foreign toplevel lists are fed from.
pub mod foreign_toplevel_shared;
/// `gnoblin_session_host_v1`: the host compositor's channel to one hosted session.
pub mod gnoblin_session_host_v1;
/// `gnoblin_window_frame_v1`: server-side decoration frames drawn by a renderer client.
pub mod gnoblin_window_frame_v1;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
/// `wl_compositor` and `wl_subcompositor`: surfaces, subsurfaces and buffer commits.
pub mod wl_compositor;
/// `wl_data_device_manager`: the clipboard and drag-and-drop device manager.
pub mod wl_data_device_manager;
/// `wl_output`: the output globals the layout created.
pub mod wl_output;
/// `wl_seat`: keyboard and pointer focus over surfaces.
pub mod wl_seat;
/// `wl_shm`: shared-memory buffer pools.
pub mod wl_shm;
/// `wp_alpha_modifier_v1`: a per-surface opacity factor.
pub mod wp_alpha_modifier_v1;
/// `wp_content_type_v1`: whether a surface shows video or a game.
pub mod wp_content_type_v1;
/// `wp_cursor_shape_v1`: the cursor shape a client names.
pub mod wp_cursor_shape_v1;
/// `wp_fifo_v1`: in-order commit barriers released on each frame tick.
pub mod wp_fifo_v1;
/// `wp_fractional_scale_v1`: the fractional scale a surface draws at.
pub mod wp_fractional_scale_v1;
/// `wp_presentation`: presentation feedback with the frame's present time.
pub mod wp_presentation_time;
/// `wp_security_context_v1`: sandbox security contexts for applications.
pub mod wp_security_context_v1;
/// `wp_single_pixel_buffer_v1`: buffers that carry a single colour.
pub mod wp_single_pixel_buffer_v1;
/// `wp_tearing_control_v1`: the presentation hint a surface asks for.
pub mod wp_tearing_control_v1;
/// `wp_viewporter`: source crop and destination size per surface.
pub mod wp_viewporter;
/// `xdg_activation_v1`: one-shot activation tokens.
pub mod xdg_activation_v1;
/// `xdg_wm_dialog_v1`: modal dialog relationships between toplevels.
pub mod xdg_dialog_v1;
/// `zxdg_exporter_v2` and `zxdg_importer_v2`: export a toplevel handle to another client.
pub mod xdg_foreign;
/// `xdg_shell`: toplevels, popups, configure and ack.
pub mod xdg_shell;
/// `xdg_toplevel_icon_v1`: toplevel icon names and buffers, stored but not drawn.
pub mod xdg_toplevel_icon_v1;
/// `xdg_toplevel_tag_v1`: toplevel tags, stored per window.
pub mod xdg_toplevel_tag_v1;
/// `zwlr_data_control_v1`: clipboard access for privileged clients (wlroots).
pub mod zwlr_data_control_v1;
/// `zwlr_foreign_toplevel_manager_v1`: the window list a taskbar reads.
pub mod zwlr_foreign_toplevel_management_v1;
/// `zwlr_gamma_control_manager_v1`: per-output gamma ramps.
pub mod zwlr_gamma_control_v1;
/// `zwlr_layer_shell_v1`: layer surfaces for bars and docks.
pub mod zwlr_layer_shell_v1;
/// `zwlr_output_manager_v1`: the outputs and layout a configuration tool sees.
pub mod zwlr_output_management_v1;
/// `zwlr_output_power_manager_v1`: per-output power, on or off.
pub mod zwlr_output_power_management_v1;
/// `zwlr_screencopy_manager_v1`: one-shot screencopy of an output or region.
pub mod zwlr_screencopy_v1;
/// `zwp_idle_inhibit_manager_v1`: surfaces that keep the session awake.
pub mod zwp_idle_inhibit_v1;
/// `zwp_input_method_manager_v2`: the privileged input-method half of a text conversation.
pub mod zwp_input_method_v2;
/// `zwp_keyboard_shortcuts_inhibit_manager_v1`: grants, refuses and revokes shortcut inhibitors.
pub mod zwp_keyboard_shortcuts_inhibit_v1;
/// `zwp_pointer_constraints_v1`: locked and confined pointers.
pub mod zwp_pointer_constraints_v1;
/// `zwp_pointer_gestures_v1`: gesture objects for pointer input.
pub mod zwp_pointer_gestures_v1;
/// `zwp_primary_selection_device_manager_v1`: the primary selection.
pub mod zwp_primary_selection_v1;
/// `zwp_relative_pointer_manager_v1`: the vector motion a locked pointer reports.
pub mod zwp_relative_pointer_v1;
/// `zwp_text_input_manager_v3`: the text-input conversation.
pub mod zwp_text_input_v3;
/// `zwp_virtual_keyboard_manager_v1`: the privileged virtual keyboard.
pub mod zwp_virtual_keyboard_v1;
/// `zxdg_decoration_manager_v1`: the client's server-side decoration preference.
pub mod zxdg_decoration_v1;

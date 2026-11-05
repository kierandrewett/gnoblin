//! Smithay protocol handlers, one module per protocol.
//!
//! Each module answers exactly one protocol's requests and translates them into
//! window-manager events; none of them decides placement, focus or stacking. The
//! split is what lets a protocol be added without touching another, and it is the
//! layout `design/rust-compositor.md` ("Project structure") asks for.
//!
//! | Module | Protocol | What it does here |
//! |---|---|---|
//! | [`xdg_activation_v1`] | `xdg_activation_v1` | mints and redeems one-shot activation tokens |
//! | [`gnoblin_blur_fade_v1`] | `gnoblin_blur_fade_v1` | fade metadata for a surface's own items, applied with its commit |
//! | [`gnoblin_session_host_v1`] | `gnoblin_session_host_v1` | the host's channel to one hosted session: readiness, protection, logout and power requests |
//! | [`capture`] | `ext_image_copy_capture_v1`, `ext_image_capture_source_v1`, `zwlr_screencopy_manager_v1` | frames of an output or a window, under the lock embargo and the access roster |
//! | [`wl_compositor`] | `wl_compositor`, `wl_subcompositor` | surfaces, subsurfaces and buffer commits |
//! | [`wl_shm`] | `wl_shm`, `wl_shm_pool` | shared-memory buffer pools |
//! | [`wp_content_type_v1`] | `wp_content_type_v1` | records whether a surface shows video or a game |
//! | [`wp_cursor_shape_v1`] | `wp_cursor_shape_manager_v1` | records the cursor a client names |
//! | [`xdg_decoration_unstable_v1`] | `zxdg_decoration_manager_v1` | answers the client's preference (client-side, this slice draws no frames) |
//! | [`extension`] | none of its own | the seam a compat adapter's redirect registers through, and the operations it may ask the compositor for |
//! | [`foreign_toplevel`] | `zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1` | the window list a taskbar reads and the requests it may make |
//! | [`wp_fractional_scale_v1`] | `wp_fractional_scale_manager_v1` | tells a surface the scale its output draws it at |
//! | [`zwp_idle_inhibit_v1`] | `zwp_idle_inhibit_manager_v1` | records the surfaces that keep the session awake |
//! | [`ext_idle_notify_v1`] | `ext_idle_notifier_v1` | answers `idled`/`resumed` from the session's tick counter |
//! | [`wlr_layer_shell_unstable_v1`] | `zwlr_layer_shell_v1` | bars and docks: anchors, sizes, exclusive zones and keyboard interactivity |
//! | [`wl_output`] | `wl_output` | the output globals the layout created |
//! | [`zwp_pointer_constraints_v1`] | `zwp_pointer_constraints_v1` | activates, confines and releases a client's lock |
//! | [`zwp_pointer_gestures_v1`] | `zwp_pointer_gestures_v1` | creates the gesture objects |
//! | [`wp_presentation`] | `wp_presentation` | answers feedback with the frame's present time |
//! | [`zwp_relative_pointer_v1`] | `zwp_relative_pointer_manager_v1` | the vector motion a locked pointer reports |
//! | [`wl_seat`] | `wl_seat` | keyboard and pointer focus over surfaces |
//! | [`wp_security_context_v1`] | `wp_security_context_v1` | application sandboxes, marked before their first request |
//! | [`wl_data_device_manager`] | `wl_data_device_manager` | the clipboard and what follows focus |
//! | [`zwp_primary_selection_v1`] | `zwp_primary_selection_device_manager_v1` | the primary selection |
//! | [`ext_data_control_v1`] | `ext_data_control_manager_v1` | clipboard access for privileged clients |
//! | [`wlr_data_control_unstable_v1`] | `zwlr_data_control_manager_v1` | clipboard access for privileged clients (wlroots) |
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

pub mod capture;
pub mod ext_data_control_v1;
pub mod ext_idle_notify_v1;
pub mod ext_session_lock_v1;
pub mod extension;
pub mod foreign_toplevel;
pub mod gnoblin_blur_fade_v1;
pub mod gnoblin_session_host_v1;
pub mod gnoblin_window_frame_v1;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
pub mod wl_compositor;
pub mod wl_data_device_manager;
pub mod wl_output;
pub mod wl_seat;
pub mod wl_shm;
pub mod wlr_data_control_unstable_v1;
pub mod wlr_layer_shell_unstable_v1;
pub mod wp_content_type_v1;
pub mod wp_cursor_shape_v1;
pub mod wp_fractional_scale_v1;
pub mod wp_presentation;
pub mod wp_security_context_v1;
pub mod wp_single_pixel_buffer_v1;
pub mod wp_tearing_control_v1;
pub mod wp_viewporter;
pub mod xdg_activation_v1;
pub mod xdg_decoration_unstable_v1;
pub mod xdg_shell;
pub mod zwp_idle_inhibit_v1;
pub mod zwp_input_method_v2;
pub mod zwp_keyboard_shortcuts_inhibit_v1;
pub mod zwp_pointer_constraints_v1;
pub mod zwp_pointer_gestures_v1;
pub mod zwp_primary_selection_v1;
pub mod zwp_relative_pointer_v1;
pub mod zwp_text_input_v3;
pub mod zwp_virtual_keyboard_v1;

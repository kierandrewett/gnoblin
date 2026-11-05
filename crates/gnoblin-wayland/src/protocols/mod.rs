//! Smithay protocol handlers, one module per protocol.
//!
//! Each module answers exactly one protocol's requests and translates them into
//! window-manager events; none of them decides placement, focus or stacking. The
//! split is what lets a protocol be added without touching another, and it is the
//! layout `design/rust-compositor.md` ("Project structure") asks for.
//!
//! | Module | Protocol | What it does here |
//! |---|---|---|
//! | [`activation`] | `xdg_activation_v1` | mints and redeems one-shot activation tokens |
//! | [`blur_fade`] | `gnoblin_blur_fade_v1` | fade metadata for a surface's own items, applied with its commit |
//! | [`session_host`] | `gnoblin_session_host_v1` | the host's channel to one hosted session: readiness, protection, logout and power requests |
//! | [`capture`] | `ext_image_copy_capture_v1`, `ext_image_capture_source_v1`, `zwlr_screencopy_manager_v1` | frames of an output or a window, under the lock embargo and the access roster |
//! | [`compositor`] | `wl_compositor`, `wl_subcompositor`, `wl_shm` | surfaces, subsurfaces, buffer commits and `wl_shm` pools |
//! | [`content_type`] | `wp_content_type_v1` | records whether a surface shows video or a game |
//! | [`cursor_shape`] | `wp_cursor_shape_manager_v1` | records the cursor a client names |
//! | [`decoration`] | `zxdg_decoration_manager_v1` | answers the client's preference (client-side, this slice draws no frames) |
//! | [`foreign_toplevel`] | `zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1` | the window list a taskbar reads and the requests it may make |
//! | [`fractional_scale`] | `wp_fractional_scale_manager_v1` | tells a surface the scale its output draws it at |
//! | [`idle_inhibit`] | `zwp_idle_inhibit_manager_v1` | records the surfaces that keep the session awake |
//! | [`idle_notify`] | `ext_idle_notifier_v1` | answers `idled`/`resumed` from the session's tick counter |
//! | [`layer_shell`] | `zwlr_layer_shell_v1` | bars and docks: anchors, sizes, exclusive zones and keyboard interactivity |
//! | [`output`] | `wl_output` | the output globals the layout created |
//! | [`pointer_constraints`] | `zwp_pointer_constraints_v1` | activates, confines and releases a client's lock |
//! | [`pointer_gestures`] | `zwp_pointer_gestures_v1` | creates the gesture objects |
//! | [`presentation`] | `wp_presentation` | answers feedback with the frame's present time |
//! | [`relative_pointer`] | `zwp_relative_pointer_manager_v1` | the vector motion a locked pointer reports |
//! | [`seat`] | `wl_seat` | keyboard and pointer focus over surfaces |
//! | [`security_context`] | `wp_security_context_v1` | application sandboxes, marked before their first request |
//! | [`selection`] | `wl_data_device_manager`, `zwp_primary_selection_device_manager_v1`, the two data-control managers | the clipboard, what follows focus and who may read it |
//! | [`session_lock`] | `ext_session_lock_v1` | the lock request, its cover, its surfaces and the input gate |
//! | [`shortcuts_inhibit`] | `zwp_keyboard_shortcuts_inhibit_manager_v1` | grants, refuses and revokes shortcut inhibitors |
//! | [`single_pixel_buffer`] | `wp_single_pixel_buffer_manager_v1` | creates the global; the buffer carries its own colour |
//! | [`tearing_control`] | `wp_tearing_control_v1` | records the presentation hint a surface asks for |
//! | [`text_input`] | `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2`, `zwp_virtual_keyboard_v1` | the text-input conversation and its two privileged halves |
//! | [`viewporter`] | `wp_viewporter` | creates the global; Smithay validates a viewport on commit |
//! | [`window_frame`] | `gnoblin_window_frame_manager_v1` | frames a decoration renderer draws, their configures, regions and errors |
//! | [`xdg_shell`] | `xdg_shell` | toplevels, popups, configure and ack |

pub mod activation;
pub mod blur_fade;
pub mod capture;
pub mod compositor;
pub mod content_type;
pub mod cursor_shape;
pub mod decoration;
pub mod foreign_toplevel;
pub mod fractional_scale;
pub mod idle_inhibit;
pub mod idle_notify;
pub mod layer_shell;
pub mod output;
pub mod pointer_constraints;
pub mod pointer_gestures;
pub mod presentation;
pub mod relative_pointer;
pub mod seat;
pub mod security_context;
pub mod selection;
pub mod session_host;
pub mod session_lock;
pub mod shortcuts_inhibit;
pub mod single_pixel_buffer;
pub mod tearing_control;
pub mod text_input;
pub mod viewporter;
pub mod window_frame;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
pub mod xdg_shell;

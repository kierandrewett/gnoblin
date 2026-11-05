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
//! | [`decoration`] | `zxdg_decoration_manager_v1` | answers the client's preference (client-side, this slice draws no frames) |
//! | [`foreign_toplevel`] | `zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1` | the window list a taskbar reads and the requests it may make |
//! | [`layer_shell`] | `zwlr_layer_shell_v1` | bars and docks: anchors, sizes, exclusive zones and keyboard interactivity |
//! | [`output`] | `wl_output` | the output globals the layout created |
//! | [`seat`] | `wl_seat` | keyboard and pointer focus over surfaces |
//! | [`security_context`] | `wp_security_context_v1` | application sandboxes, marked before their first request |
//! | [`selection`] | `wl_data_device_manager`, `zwp_primary_selection_device_manager_v1`, the two data-control managers | the clipboard, what follows focus and who may read it |
//! | [`session_lock`] | `ext_session_lock_v1` | the lock request, its cover, its surfaces and the input gate |
//! | [`viewporter`] | `wp_viewporter` | creates the global; Smithay validates a viewport on commit |
//! | [`window_frame`] | `gnoblin_window_frame_manager_v1` | frames a decoration renderer draws, their configures, regions and errors |
//! | [`xdg_shell`] | `xdg_shell` | toplevels, popups, configure and ack |

pub mod activation;
pub mod blur_fade;
pub mod capture;
pub mod compositor;
pub mod decoration;
pub mod foreign_toplevel;
pub mod layer_shell;
pub mod output;
pub mod seat;
pub mod security_context;
pub mod selection;
pub mod session_host;
pub mod session_lock;
pub mod viewporter;
pub mod window_frame;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
pub mod xdg_shell;

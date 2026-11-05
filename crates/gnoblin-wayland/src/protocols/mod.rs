//! Smithay protocol handlers, one module per protocol.
//!
//! Each module answers exactly one protocol's requests and translates them into
//! window-manager events; none of them decides placement, focus or stacking. The
//! split is what lets a protocol be added without touching another, and it is the
//! layout `design/rust-compositor.md` ("Project structure") asks for.
//!
//! | Module | Protocol | What it does here |
//! |---|---|---|
//! | [`compositor`] | `wl_compositor`, `wl_subcompositor`, `wl_shm` | surfaces, subsurfaces, buffer commits and `wl_shm` pools |
//! | [`decoration`] | `zxdg_decoration_manager_v1` | answers the client's preference (client-side, this slice draws no frames) |
//! | [`output`] | `wl_output` | the output globals the layout created |
//! | [`seat`] | `wl_seat` | keyboard and pointer focus over surfaces |
//! | [`session_lock`] | `ext_session_lock_v1` | the lock request, its cover, its surfaces and the input gate |
//! | [`viewporter`] | `wp_viewporter` | creates the global; Smithay validates a viewport on commit |
//! | [`xdg_shell`] | `xdg_shell` | toplevels, popups, configure and ack |

pub mod compositor;
pub mod decoration;
pub mod output;
pub mod seat;
pub mod session_lock;
pub mod viewporter;
pub mod xdg_shell;

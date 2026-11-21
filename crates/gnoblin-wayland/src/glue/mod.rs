//! The compositor's half of each protocol crate.
//!
//! A protocol crate under `crates/protocols/` is written against `gnoblin-wayland-api` and its own
//! host trait, and never names this crate. The impls that need `State` are here, because a trait
//! impl has to sit where one of its types is defined: the host traits the protocol crates declare,
//! and the Smithay handler traits whose implementor is the state. Each body forwards, in one call,
//! to the function the protocol crate exports or to the `State` method that already does the work.
//!
//! One file per protocol crate, named after its module, so a protocol's glue is one file to read.

mod core;
pub(crate) use wl_compositor::ClientState;
mod ext_background_effect_v1;
mod ext_data_control_v1;
mod ext_foreign_toplevel_list_v1;
mod ext_idle_notify_v1;
mod ext_image_capture_source_v1;
mod ext_image_copy_capture_v1;
mod ext_workspace_v1;
mod org_kde_kwin_appmenu;
mod org_kde_kwin_server_decoration;
mod wl_compositor;
mod wl_data_device_manager;
pub(crate) use wl_data_device_manager::BackendSelection;
mod wl_output;
mod wl_seat;
mod wl_shm;
mod wp_fifo_v1;
mod wp_fractional_scale_v1;
mod wp_presentation_time;
mod wp_security_context_v1;
mod wp_tearing_control_v1;
mod xdg_activation_v1;
mod xdg_dialog_v1;
mod xdg_foreign;
mod xdg_shell;
mod xdg_system_bell_v1;
mod xdg_toplevel_icon_v1;
mod xdg_toplevel_tag_v1;
mod zwlr_data_control_v1;
mod zwlr_foreign_toplevel_management_v1;
mod zwlr_gamma_control_v1;
mod zwlr_layer_shell_v1;
mod zwlr_output_management_v1;
mod zwlr_output_power_management_v1;
mod zwlr_screencopy_v1;
mod zwlr_virtual_pointer_v1;
mod zwp_idle_inhibit_v1;
mod zwp_input_method_v2;
mod zwp_keyboard_shortcuts_inhibit_v1;
mod zwp_linux_dmabuf_v1;
mod zwp_pointer_constraints_v1;
mod zwp_primary_selection_v1;
mod zwp_tablet_v2;
mod zwp_text_input_v3;
mod zwp_virtual_keyboard_v1;
mod zxdg_decoration_v1;
mod zxdg_output_v1;

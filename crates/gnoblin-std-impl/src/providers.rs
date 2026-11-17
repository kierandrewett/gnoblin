//! The providers std registers with the core (`design/modular-framework.md`, 9.4 and 28).
//!
//! Core holds mechanism and a minimal safe default: with no provider it places every window at the origin (`NullWm`), refuses a
//! portal request that needs a dialog (`no-ui`) and leaves the compositor settings at their defaults. What the desktop does by
//! convention is std's, registered here as C vtables through the core's `provider_register`:
//!
//! - the window manager that places and focuses windows while no shell holds the tree, `gnoblin-std-wm`'s `Fallback`;
//! - the portal dialog UI, `gnoblin-std-portal`'s `PortalUi`;
//! - the configuration observer, which turns the `window-management`, `accessibility` and `compositor` sections of the accepted
//!   document into `setting_set` calls: the window management policy and titlebar gestures, the magnifier and colour filter,
//!   and the visual bell.
//!
//! [`register_defaults`] is the one entry. It runs from `gnoblin_std_init` (the compositor process opening std for the stock
//! session, on a context of the host's own that never retires) and from `open` (a shell that uses std, on its own generation, so a
//! reload of that shell hands the providers over to the next generation of it). A kind that is already registered is `E_BUSY`
//! and is left as it is: whoever registered first decides.
#![allow(unsafe_code)]

use core::ffi::c_void;

use gnoblin_a11y::ColourFilter;
use gnoblin_config::{ColourFilter as ConfigColourFilter, Config, FocusMode as ConfiguredFocusMode, FocusNewWindows};
use gnoblin_config::{TitlebarAction, VisualBellType};
use gnoblin_core_abi::{self as abi, GnoblinApi, GnoblinCtx, GnoblinValue, PROVIDER_CONFIG, PROVIDER_UI, PROVIDER_WM};
use gnoblin_provider_wire::{Exported, export_config, export_ui, export_wm};
use gnoblin_wm_api::{DragConstraint, FocusMode, NewWindowFocus, WmOptions};

/// The zoom used when a document turns the magnifier on without naming a factor. It is the schema's own default, so the two
/// never disagree.
const DEFAULT_ZOOM: f64 = 2.0;

/// The size of the frame a portal dialog is first drawn into, before the compositor says how large an output is.
const DIALOG_FRAME: (usize, usize) = (1280, 800);

/// A value of a compositor setting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SettingArg {
    Bool(bool),
    Uint(u64),
    Double(f64),
    Text(&'static str),
    Bytes(Vec<u8>),
}

/// The compositor settings an accepted document means, in the order they are set: every key every time, so a section the
/// document leaves out puts its settings back to the defaults.
pub(crate) fn settings_of(config: &Config) -> Vec<(&'static str, SettingArg)> {
    let wm = config.window_management.clone().unwrap_or_default();
    let defaults = WmOptions::default();
    let options = WmOptions {
        edge_tiling: wm.edge_tiling.unwrap_or(defaults.edge_tiling),
        drag_constraint: wm
            .constrain_drag_to_work_area
            .map_or(defaults.drag_constraint, |constrain| {
                if constrain {
                    DragConstraint::WorkArea
                } else {
                    DragConstraint::MinimumStrip
                }
            }),
        new_window_focus: wm
            .focus_new_windows
            .map_or(defaults.new_window_focus, |focus| match focus {
                FocusNewWindows::Allow => NewWindowFocus::Allow,
                FocusNewWindows::Prevent => NewWindowFocus::Prevent,
            }),
        focus_mode: wm.focus_mode.map_or(defaults.focus_mode, |mode| match mode {
            ConfiguredFocusMode::Click => FocusMode::Click,
            ConfiguredFocusMode::Hover => FocusMode::Hover,
            ConfiguredFocusMode::HoverStrict => FocusMode::HoverStrict,
        }),
        focus_change_on_pointer_rest: wm
            .focus_change_on_pointer_rest
            .unwrap_or(defaults.focus_change_on_pointer_rest),
        raise_on_click: wm.raise_on_click.unwrap_or(defaults.raise_on_click),
        auto_raise: wm.auto_raise.unwrap_or(defaults.auto_raise),
        auto_raise_delay: wm
            .auto_raise_delay
            .and_then(|ms| u64::try_from(ms).ok())
            .unwrap_or(defaults.auto_raise_delay),
    };
    let a11y = config.accessibility.clone().unwrap_or_default();
    let zoom = if a11y.magnifier.unwrap_or(false) {
        a11y.magnifier_zoom.unwrap_or(DEFAULT_ZOOM)
    } else {
        1.0
    };
    let filter = a11y
        .colour_filter
        .and_then(colour_matrix)
        .map_or_else(Vec::new, |rows| {
            rows.iter()
                .flatten()
                .flat_map(|cell| cell.to_le_bytes())
                .collect::<Vec<u8>>()
        });
    let compositor = config.compositor.clone().unwrap_or_default();
    let flash = if compositor.visual_bell.unwrap_or(false) {
        match compositor.visual_bell_type {
            Some(VisualBellType::FrameFlash) => "frame",
            Some(VisualBellType::FullscreenFlash) | None => "fullscreen",
        }
    } else {
        "off"
    };
    vec![
        ("wm.edge_tiling", SettingArg::Bool(options.edge_tiling)),
        ("wm.drag_constraint", SettingArg::Text(options.drag_constraint.name())),
        ("wm.new_window_focus", SettingArg::Text(options.new_window_focus.name())),
        ("wm.focus_mode", SettingArg::Text(options.focus_mode.name())),
        (
            "wm.focus_change_on_pointer_rest",
            SettingArg::Bool(options.focus_change_on_pointer_rest),
        ),
        ("wm.raise_on_click", SettingArg::Bool(options.raise_on_click)),
        ("wm.auto_raise", SettingArg::Bool(options.auto_raise)),
        ("wm.auto_raise_delay", SettingArg::Uint(options.auto_raise_delay)),
        (
            "wm.check_alive_timeout",
            SettingArg::Uint(
                wm.check_alive_timeout
                    .and_then(|ms| u64::try_from(ms).ok())
                    .unwrap_or(CHECK_ALIVE_DEFAULT_MS),
            ),
        ),
        (
            "wm.titlebar.double_click",
            SettingArg::Text(
                wm.action_double_click_titlebar
                    .unwrap_or(TitlebarAction::ToggleMaximize)
                    .as_str(),
            ),
        ),
        (
            "wm.titlebar.middle_click",
            SettingArg::Text(
                wm.action_middle_click_titlebar
                    .unwrap_or(TitlebarAction::Lower)
                    .as_str(),
            ),
        ),
        (
            "wm.titlebar.right_click",
            SettingArg::Text(wm.action_right_click_titlebar.unwrap_or(TitlebarAction::Menu).as_str()),
        ),
        ("a11y.zoom", SettingArg::Double(zoom)),
        ("a11y.filter", SettingArg::Bytes(filter)),
        ("bell.flash", SettingArg::Text(flash)),
    ]
}

/// How long a client may take to answer a ping before the compositor offers to close it, when the document does not say, in
/// milliseconds. It is the configuration reference's own default.
const CHECK_ALIVE_DEFAULT_MS: u64 = 5000;

/// The colour transform rows of a configured filter, or [`None`] when it leaves colours alone.
fn colour_matrix(filter: ConfigColourFilter) -> Option<[[f32; 5]; 4]> {
    let filter = match filter {
        ConfigColourFilter::None => return None,
        ConfigColourFilter::Invert => ColourFilter::Invert,
        ConfigColourFilter::Greyscale => ColourFilter::Greyscale,
        ConfigColourFilter::Protanopia => ColourFilter::Protanopia,
        ConfigColourFilter::Deuteranopia => ColourFilter::Deuteranopia,
        ConfigColourFilter::Tritanopia => ColourFilter::Tritanopia,
    };
    Some(filter.matrix())
}

/// Calls `setting_set` with `value`.
fn set(api: &GnoblinApi, ctx: GnoblinCtx, key: &str, value: &SettingArg) -> i32 {
    let mut raw = GnoblinValue {
        size: core::mem::size_of::<GnoblinValue>() as u32,
        kind: 0,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: core::ptr::null(),
        len: 0,
    };
    match value {
        SettingArg::Bool(flag) => {
            raw.kind = abi::VALUE_BOOL;
            raw.bits = u64::from(*flag);
        }
        SettingArg::Uint(number) => {
            raw.kind = abi::VALUE_UINT;
            raw.bits = *number;
        }
        SettingArg::Double(number) => {
            raw.kind = abi::VALUE_DOUBLE;
            raw.bits = number.to_bits();
        }
        SettingArg::Text(text) => {
            raw.kind = abi::VALUE_TEXT;
            raw.ptr = text.as_ptr().cast();
            raw.len = text.len();
        }
        SettingArg::Bytes(bytes) => {
            raw.kind = abi::VALUE_BYTES;
            raw.ptr = bytes.as_ptr().cast();
            raw.len = bytes.len();
        }
    }
    // SAFETY: `key` and the value's pointer are live for the call, `raw` is a live local, and the table is the core's.
    unsafe { (api.setting_set)(ctx, key.as_ptr(), key.len(), &raw const raw) }
}

/// The observer's body: reads the document and sets every setting it means.
fn observe(api: &'static GnoblinApi, ctx: GnoblinCtx, document: &str) -> Result<(), String> {
    let config = Config::from_json_bytes(document.as_bytes())?;
    let mut refused = Vec::new();
    for (key, value) in settings_of(&config) {
        let code = set(api, ctx, key, &value);
        if code != abi::E_OK {
            refused.push(format!("{key} ({code})"));
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(format!("the core refused {}", refused.join(", ")))
    }
}

/// Registers one provider. `Ok(true)` when it is the core's now, `Ok(false)` when another registered the kind first.
fn register<V: 'static>(
    api: &'static GnoblinApi,
    ctx: GnoblinCtx,
    kind: u32,
    what: &str,
    provider: Exported<V>,
) -> Result<bool, String> {
    let Exported {
        vtable,
        user,
        drop_user,
    } = provider;
    // SAFETY: `vtable` is a static of the kind and `user` is the provider's state. A registration the core takes owns `user`
    // and frees it with `drop_user`; one it refuses leaves it with this function, which frees it below.
    let reg = unsafe {
        (api.provider_register)(
            ctx,
            kind,
            core::ptr::from_ref(vtable).cast::<c_void>(),
            user,
            Some(drop_user),
        )
    };
    if reg != 0 {
        return Ok(true);
    }
    // SAFETY: the core did not adopt `user`, so this is its only owner, and `drop_user` frees what `user` points at.
    unsafe { drop_user(user) };
    let text = gnoblin_core::private::last_error();
    if text.starts_with("E_BUSY") {
        Ok(false)
    } else {
        Err(format!("the {what} provider was not registered: {text}"))
    }
}

/// What registering the defaults did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Registered {
    /// The kinds that are std's now.
    pub(crate) taken: Vec<&'static str>,
    /// The kinds another registrant already holds.
    pub(crate) kept_by_others: Vec<&'static str>,
}

/// Registers std's window manager, portal dialog UI and configuration observer on `ctx`.
///
/// # Errors
/// A provider the core would not take for a reason other than the kind being held: the text says which and why. The kinds
/// before it stay registered.
pub(crate) fn register_defaults(api: &'static GnoblinApi, ctx: GnoblinCtx) -> Result<Registered, String> {
    let mut taken = Vec::new();
    let mut kept = Vec::new();
    let mut note = |name: &'static str, took: bool| if took { taken.push(name) } else { kept.push(name) };
    note(
        "window manager",
        register(
            api,
            ctx,
            PROVIDER_WM,
            "window manager",
            export_wm(Box::<gnoblin_std_wm::Fallback>::default()),
        )?,
    );
    note(
        "portal dialogs",
        register(
            api,
            ctx,
            PROVIDER_UI,
            "portal dialogs",
            export_ui(Box::new(gnoblin_std_portal::PortalUi::new(
                DIALOG_FRAME.0,
                DIALOG_FRAME.1,
            ))),
        )?,
    );
    note(
        "configuration",
        register(
            api,
            ctx,
            PROVIDER_CONFIG,
            "configuration observer",
            export_config(Box::new(move |document| observe(api, ctx, document))),
        )?,
    );
    Ok(Registered {
        taken,
        kept_by_others: kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: &str) -> Vec<(&'static str, SettingArg)> {
        settings_of(&Config::from_json_bytes(json.as_bytes()).expect("a valid document"))
    }

    fn value(settings: &[(&'static str, SettingArg)], key: &str) -> SettingArg {
        settings
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{key} is not set"))
    }

    #[test]
    fn an_empty_document_puts_every_setting_back_to_its_default() {
        let all = settings("{}");
        assert_eq!(value(&all, "wm.edge_tiling"), SettingArg::Bool(false));
        assert_eq!(value(&all, "wm.drag_constraint"), SettingArg::Text("work_area"));
        assert_eq!(value(&all, "wm.new_window_focus"), SettingArg::Text("prevent"));
        assert_eq!(value(&all, "wm.focus_mode"), SettingArg::Text("click"));
        assert_eq!(value(&all, "wm.raise_on_click"), SettingArg::Bool(true));
        assert_eq!(value(&all, "wm.auto_raise_delay"), SettingArg::Uint(500));
        assert_eq!(
            value(&all, "wm.check_alive_timeout"),
            SettingArg::Uint(CHECK_ALIVE_DEFAULT_MS)
        );
        assert_eq!(
            value(&all, "wm.titlebar.double_click"),
            SettingArg::Text("toggle-maximize")
        );
        assert_eq!(value(&all, "a11y.zoom"), SettingArg::Double(1.0));
        assert_eq!(value(&all, "a11y.filter"), SettingArg::Bytes(Vec::new()));
        assert_eq!(value(&all, "bell.flash"), SettingArg::Text("off"));
    }

    #[test]
    fn a_window_management_section_maps_each_leaf() {
        let all = settings(
            r#"{"window-management": {"edge-tiling": true, "constrain-drag-to-work-area": false, "focus-mode": "hover-strict", "check-alive-timeout": 1500, "focus-new-windows": "allow"}}"#,
        );
        assert_eq!(value(&all, "wm.edge_tiling"), SettingArg::Bool(true));
        assert_eq!(value(&all, "wm.drag_constraint"), SettingArg::Text("minimum_strip"));
        assert_eq!(value(&all, "wm.focus_mode"), SettingArg::Text("hover-strict"));
        assert_eq!(value(&all, "wm.check_alive_timeout"), SettingArg::Uint(1500));
        assert_eq!(value(&all, "wm.new_window_focus"), SettingArg::Text("allow"));
    }

    #[test]
    fn a_magnifier_without_a_factor_takes_the_schema_default_and_a_switched_off_one_is_the_identity() {
        assert_eq!(
            value(&settings(r#"{"accessibility": {"magnifier": true}}"#), "a11y.zoom"),
            SettingArg::Double(DEFAULT_ZOOM)
        );
        assert_eq!(
            value(
                &settings(r#"{"accessibility": {"magnifier": false, "magnifier-zoom": 4}}"#),
                "a11y.zoom"
            ),
            SettingArg::Double(1.0)
        );
    }

    #[test]
    fn a_colour_filter_is_the_twenty_cells_of_its_matrix() {
        let all = settings(r#"{"accessibility": {"colour-filter": "invert"}}"#);
        let expected: Vec<u8> = ColourFilter::Invert
            .matrix()
            .iter()
            .flatten()
            .flat_map(|cell| cell.to_le_bytes())
            .collect();
        assert_eq!(expected.len(), 80);
        assert_eq!(value(&all, "a11y.filter"), SettingArg::Bytes(expected));
        assert_eq!(
            value(
                &settings(r#"{"accessibility": {"colour-filter": "none"}}"#),
                "a11y.filter"
            ),
            SettingArg::Bytes(Vec::new())
        );
    }

    #[test]
    fn the_visual_bell_flashes_where_the_document_says_and_the_whole_screen_by_default() {
        assert_eq!(
            value(&settings(r#"{"compositor": {"visual-bell": true}}"#), "bell.flash"),
            SettingArg::Text("fullscreen")
        );
        assert_eq!(
            value(
                &settings(r#"{"compositor": {"visual-bell": true, "visual-bell-type": "frame-flash"}}"#),
                "bell.flash"
            ),
            SettingArg::Text("frame")
        );
        assert_eq!(
            value(
                &settings(r#"{"compositor": {"visual-bell-type": "frame-flash"}}"#),
                "bell.flash"
            ),
            SettingArg::Text("off")
        );
    }
}

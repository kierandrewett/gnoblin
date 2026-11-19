//! The compositor settings a configuration observer sets by key (`setting_set`).
//!
//! The mechanism is core's (the frame's zoom and colour passes, the node effect that flashes, the seat policy the engine
//! publishes); what a document means by "invert", "2x" or "hover" is the provider's. The provider says it in these keys, and
//! this module is the only place that knows what each one does to the session.

use std::time::Duration;

use gnoblin_config::TitlebarAction;
use gnoblin_render_frame::frame::{MAX_ZOOM, MIN_ZOOM};
use gnoblin_wayland::{BellFlash, ControlHandle, MotionPolicy, Setting, TitlebarActions};
use gnoblin_wm_api::{DragConstraint, FocusMode, NewWindowFocus, WmOptions};

/// A value a provider hands `setting_set`.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A double.
    Double(f64),
    /// Text.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
}

/// Why a setting was not set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingError {
    /// No such key.
    UnknownKey,
    /// The value is not of the kind the key takes.
    WrongType(&'static str),
    /// The value is of the right kind and the compositor refused it.
    Refused(String),
}

impl std::fmt::Display for SettingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey => formatter.write_str("no such setting"),
            Self::WrongType(wanted) => write!(formatter, "the setting takes {wanted}"),
            Self::Refused(why) => formatter.write_str(why),
        }
    }
}

/// What a key changed, for the engine to be told.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SettingEffect {
    /// The window management options, titlebar gestures and check-alive timeout, as a set.
    WindowManagement {
        options: WmOptions,
        titlebar: TitlebarActions,
        check_alive_timeout: Duration,
    },
    /// One compositor setting.
    Compositor(Setting),
}

impl SettingEffect {
    /// Tells the session.
    pub(super) fn deliver(self, control: &ControlHandle) {
        match self {
            Self::WindowManagement {
                options,
                titlebar,
                check_alive_timeout,
            } => control.set_window_management(options, titlebar, check_alive_timeout),
            Self::Compositor(setting) => control.setting_set(setting),
        }
    }
}

/// The values set so far, which are applied again when the session attaches after they were set.
#[derive(Clone, Debug)]
pub(super) struct HeldSettings {
    options: WmOptions,
    titlebar: TitlebarActions,
    check_alive_timeout: Duration,
    /// Whether a `wm.*` key was ever set: before one is, the engine keeps the defaults it started with.
    window_management_set: bool,
    zoom: Option<Setting>,
    filter: Option<Setting>,
    bell: Option<Setting>,
    /// The window motion, once a `motion.*` key set a part of it: the parts not set are instant.
    motion: Option<MotionPolicy>,
}

impl Default for HeldSettings {
    fn default() -> Self {
        Self {
            options: WmOptions::default(),
            titlebar: TitlebarActions::default(),
            check_alive_timeout: gnoblin_wayland::Config::default().check_alive_timeout,
            window_management_set: false,
            zoom: None,
            filter: None,
            bell: None,
            motion: None,
        }
    }
}

fn boolean(value: &SettingValue) -> Result<bool, SettingError> {
    match value {
        SettingValue::Bool(flag) => Ok(*flag),
        _ => Err(SettingError::WrongType("a boolean")),
    }
}

fn text(value: &SettingValue) -> Result<&str, SettingError> {
    match value {
        SettingValue::Text(text) => Ok(text),
        _ => Err(SettingError::WrongType("text")),
    }
}

fn unsigned(value: &SettingValue) -> Result<u64, SettingError> {
    match value {
        SettingValue::Uint(number) => Ok(*number),
        SettingValue::Int(number) => {
            u64::try_from(*number).map_err(|_| SettingError::Refused("the setting is not negative".to_owned()))
        }
        _ => Err(SettingError::WrongType("an unsigned integer")),
    }
}

fn number(value: &SettingValue) -> Result<f64, SettingError> {
    match value {
        SettingValue::Double(number) => Ok(*number),
        SettingValue::Int(number) => Ok(*number as f64),
        SettingValue::Uint(number) => Ok(*number as f64),
        _ => Err(SettingError::WrongType("a number")),
    }
}

fn named<T>(value: &SettingValue, parse: impl Fn(&str) -> Option<T>, what: &str) -> Result<T, SettingError> {
    let name = text(value)?;
    parse(name).ok_or_else(|| SettingError::Refused(format!("`{name}` is not a {what}")))
}

impl HeldSettings {
    /// Sets `key`, and says what the session must be told.
    pub(super) fn set(&mut self, key: &str, value: &SettingValue) -> Result<SettingEffect, SettingError> {
        match key {
            "a11y.zoom" => {
                let zoom = number(value)?;
                if !zoom.is_finite() {
                    return Err(SettingError::Refused("the zoom is not a number".to_owned()));
                }
                // A zoom of one is the identity and is no magnifier; one past the frame's ceiling is clamped, not dropped.
                let zoom = zoom.clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)) as f32;
                let setting = Setting::Zoom((zoom > MIN_ZOOM).then_some(zoom));
                self.zoom = Some(setting);
                Ok(SettingEffect::Compositor(setting))
            }
            "a11y.filter" => {
                let SettingValue::Bytes(bytes) = value else {
                    return Err(SettingError::WrongType("bytes"));
                };
                let matrix = match bytes.len() {
                    0 => None,
                    80 => {
                        let mut rows = [[0.0f32; 5]; 4];
                        for (index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
                            let cell = f32::from_le_bytes(*chunk);
                            if !cell.is_finite() {
                                return Err(SettingError::Refused("a colour matrix cell is not a number".to_owned()));
                            }
                            rows[index / 5][index % 5] = cell;
                        }
                        Some(rows)
                    }
                    other => {
                        return Err(SettingError::Refused(format!(
                            "a colour matrix is 80 bytes (4 by 5 floats), not {other}"
                        )));
                    }
                };
                let setting = Setting::ColourFilter(matrix);
                self.filter = Some(setting);
                Ok(SettingEffect::Compositor(setting))
            }
            "motion.map_fade_ms" | "motion.unmap_fade_ms" | "motion.retarget_ms" => {
                let ms = u32::try_from(unsigned(value)?).map_err(|_| {
                    SettingError::Refused("a motion length is a number of milliseconds a u32 holds".to_owned())
                })?;
                let mut policy = self.motion.unwrap_or(MotionPolicy::INSTANT);
                match key {
                    "motion.map_fade_ms" => policy.map_fade_ms = ms,
                    "motion.unmap_fade_ms" => policy.unmap_fade_ms = ms,
                    _ => policy.retarget_ms = ms,
                }
                self.motion = Some(policy);
                Ok(SettingEffect::Compositor(Setting::Motion(policy)))
            }
            "bell.flash" => {
                let flash = match text(value)? {
                    "off" => BellFlash::Off,
                    "frame" => BellFlash::Frame,
                    "fullscreen" => BellFlash::Fullscreen,
                    other => {
                        return Err(SettingError::Refused(format!(
                            "`{other}` is not off, frame or fullscreen"
                        )));
                    }
                };
                let setting = Setting::BellFlash(flash);
                self.bell = Some(setting);
                Ok(SettingEffect::Compositor(setting))
            }
            "wm.edge_tiling" => {
                let on = boolean(value)?;
                self.window_management(|this| {
                    this.options.edge_tiling = on;
                    Ok(())
                })
            }
            "wm.drag_constraint" => {
                let constraint = named(value, DragConstraint::from_name, "drag constraint")?;
                self.window_management(|this| {
                    this.options.drag_constraint = constraint;
                    Ok(())
                })
            }
            "wm.new_window_focus" => {
                let focus = named(value, NewWindowFocus::from_name, "new window focus")?;
                self.window_management(|this| {
                    this.options.new_window_focus = focus;
                    Ok(())
                })
            }
            "wm.focus_mode" => {
                let mode = named(value, FocusMode::from_name, "focus mode")?;
                self.window_management(|this| {
                    this.options.focus_mode = mode;
                    Ok(())
                })
            }
            "wm.focus_change_on_pointer_rest" => {
                let on = boolean(value)?;
                self.window_management(|this| {
                    this.options.focus_change_on_pointer_rest = on;
                    Ok(())
                })
            }
            "wm.raise_on_click" => {
                let on = boolean(value)?;
                self.window_management(|this| {
                    this.options.raise_on_click = on;
                    Ok(())
                })
            }
            "wm.auto_raise" => {
                let on = boolean(value)?;
                self.window_management(|this| {
                    this.options.auto_raise = on;
                    Ok(())
                })
            }
            "wm.auto_raise_delay" => {
                let delay = unsigned(value)?;
                self.window_management(|this| {
                    this.options.auto_raise_delay = delay;
                    Ok(())
                })
            }
            "wm.check_alive_timeout" => {
                let timeout = Duration::from_millis(unsigned(value)?);
                self.window_management(|this| {
                    this.check_alive_timeout = timeout;
                    Ok(())
                })
            }
            "wm.titlebar.double_click" => {
                let action = named(value, TitlebarAction::parse, "titlebar action")?;
                self.window_management(|this| {
                    this.titlebar.double_click = action;
                    Ok(())
                })
            }
            "wm.titlebar.middle_click" => {
                let action = named(value, TitlebarAction::parse, "titlebar action")?;
                self.window_management(|this| {
                    this.titlebar.middle_click = action;
                    Ok(())
                })
            }
            "wm.titlebar.right_click" => {
                let action = named(value, TitlebarAction::parse, "titlebar action")?;
                self.window_management(|this| {
                    this.titlebar.right_click = action;
                    Ok(())
                })
            }
            _ => Err(SettingError::UnknownKey),
        }
    }

    /// Changes the window management values with `change` and answers the whole set.
    fn window_management(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), SettingError>,
    ) -> Result<SettingEffect, SettingError> {
        change(self)?;
        self.window_management_set = true;
        Ok(self.window_management_action())
    }

    fn window_management_action(&self) -> SettingEffect {
        SettingEffect::WindowManagement {
            options: self.options,
            titlebar: self.titlebar,
            check_alive_timeout: self.check_alive_timeout,
        }
    }

    /// Everything set so far, for a session that attaches after it was set.
    pub(super) fn all(&self) -> Vec<SettingEffect> {
        let mut actions = Vec::new();
        if self.window_management_set {
            actions.push(self.window_management_action());
        }
        for setting in [&self.zoom, &self.filter, &self.bell].into_iter().flatten() {
            actions.push(SettingEffect::Compositor(*setting));
        }
        if let Some(policy) = self.motion {
            actions.push(SettingEffect::Compositor(Setting::Motion(policy)));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix_bytes(matrix: &[[f32; 5]; 4]) -> Vec<u8> {
        matrix.iter().flatten().flat_map(|cell| cell.to_le_bytes()).collect()
    }

    #[test]
    fn the_motion_keys_set_the_parts_of_one_policy_and_the_parts_not_set_are_instant() {
        let mut settings = HeldSettings::default();
        assert!(settings.all().is_empty(), "nothing is set until a provider sets it");
        assert_eq!(
            settings.set("motion.map_fade_ms", &SettingValue::Uint(200)),
            Ok(SettingEffect::Compositor(Setting::Motion(MotionPolicy {
                map_fade_ms: 200,
                unmap_fade_ms: 0,
                retarget_ms: 0,
            })))
        );
        settings
            .set("motion.unmap_fade_ms", &SettingValue::Uint(150))
            .expect("a length");
        let last = settings
            .set("motion.retarget_ms", &SettingValue::Uint(100))
            .expect("a length");
        let policy = MotionPolicy {
            map_fade_ms: 200,
            unmap_fade_ms: 150,
            retarget_ms: 100,
        };
        assert_eq!(last, SettingEffect::Compositor(Setting::Motion(policy)));
        assert!(
            settings
                .all()
                .contains(&SettingEffect::Compositor(Setting::Motion(policy)))
        );
        assert!(
            settings
                .set("motion.map_fade_ms", &SettingValue::Text("fast".into()))
                .is_err()
        );
    }

    #[test]
    fn a_zoom_above_the_identity_is_a_magnifier_and_clamped_to_the_frames_ceiling() {
        let mut settings = HeldSettings::default();
        assert_eq!(
            settings.set("a11y.zoom", &SettingValue::Double(2.0)),
            Ok(SettingEffect::Compositor(Setting::Zoom(Some(2.0))))
        );
        assert_eq!(
            settings.set("a11y.zoom", &SettingValue::Double(32.0)),
            Ok(SettingEffect::Compositor(Setting::Zoom(Some(MAX_ZOOM))))
        );
        assert_eq!(
            settings.set("a11y.zoom", &SettingValue::Double(1.0)),
            Ok(SettingEffect::Compositor(Setting::Zoom(None))),
            "a zoom of one is no magnifier"
        );
        assert!(settings.set("a11y.zoom", &SettingValue::Double(f64::NAN)).is_err());
    }

    #[test]
    fn a_colour_matrix_is_eighty_bytes_or_nothing() {
        let mut settings = HeldSettings::default();
        let invert = [
            [-1.0, 0.0, 0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ];
        assert_eq!(
            settings.set("a11y.filter", &SettingValue::Bytes(matrix_bytes(&invert))),
            Ok(SettingEffect::Compositor(Setting::ColourFilter(Some(invert))))
        );
        assert_eq!(
            settings.set("a11y.filter", &SettingValue::Bytes(Vec::new())),
            Ok(SettingEffect::Compositor(Setting::ColourFilter(None)))
        );
        assert!(matches!(
            settings.set("a11y.filter", &SettingValue::Bytes(vec![0; 12])),
            Err(SettingError::Refused(_))
        ));
        let mut nan = matrix_bytes(&invert);
        nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(settings.set("a11y.filter", &SettingValue::Bytes(nan)).is_err());
        assert_eq!(
            settings.set("a11y.filter", &SettingValue::Text("invert".to_owned())),
            Err(SettingError::WrongType("bytes"))
        );
    }

    #[test]
    fn the_bell_flash_takes_three_names() {
        let mut settings = HeldSettings::default();
        for (name, flash) in [
            ("off", BellFlash::Off),
            ("frame", BellFlash::Frame),
            ("fullscreen", BellFlash::Fullscreen),
        ] {
            assert_eq!(
                settings.set("bell.flash", &SettingValue::Text(name.to_owned())),
                Ok(SettingEffect::Compositor(Setting::BellFlash(flash)))
            );
        }
        assert!(
            settings
                .set("bell.flash", &SettingValue::Text("blink".to_owned()))
                .is_err()
        );
    }

    #[test]
    fn each_window_management_key_changes_one_field_and_answers_the_whole_set() {
        let mut settings = HeldSettings::default();
        settings.set("wm.edge_tiling", &SettingValue::Bool(true)).expect("sets");
        settings
            .set("wm.focus_mode", &SettingValue::Text("hover-strict".to_owned()))
            .expect("sets");
        settings
            .set("wm.auto_raise_delay", &SettingValue::Uint(250))
            .expect("sets");
        settings
            .set("wm.titlebar.double_click", &SettingValue::Text("minimize".to_owned()))
            .expect("sets");
        let Ok(SettingEffect::WindowManagement {
            options,
            titlebar,
            check_alive_timeout,
        }) = settings.set("wm.check_alive_timeout", &SettingValue::Uint(1500))
        else {
            panic!("a window management key answers the window management set");
        };
        assert!(options.edge_tiling);
        assert_eq!(options.focus_mode, FocusMode::HoverStrict);
        assert_eq!(options.auto_raise_delay, 250);
        assert!(options.raise_on_click, "an unset key keeps its default");
        assert_eq!(titlebar.double_click, TitlebarAction::Minimize);
        assert_eq!(titlebar.right_click, TitlebarActions::default().right_click);
        assert_eq!(check_alive_timeout, Duration::from_millis(1500));
    }

    #[test]
    fn a_wrong_kind_a_bad_name_and_an_unknown_key_are_refused_apart() {
        let mut settings = HeldSettings::default();
        assert_eq!(
            settings.set("nope", &SettingValue::Bool(true)),
            Err(SettingError::UnknownKey)
        );
        assert_eq!(
            settings.set("wm.edge_tiling", &SettingValue::Text("yes".to_owned())),
            Err(SettingError::WrongType("a boolean"))
        );
        assert!(matches!(
            settings.set("wm.focus_mode", &SettingValue::Text("sideways".to_owned())),
            Err(SettingError::Refused(_))
        ));
        assert!(matches!(
            settings.set("wm.auto_raise_delay", &SettingValue::Int(-1)),
            Err(SettingError::Refused(_))
        ));
        assert!(settings.all().is_empty(), "a refused key is not kept");
    }

    #[test]
    fn what_was_set_before_the_session_attached_is_applied_when_it_does() {
        let mut settings = HeldSettings::default();
        assert!(settings.all().is_empty(), "nothing set, nothing to apply");
        settings
            .set("bell.flash", &SettingValue::Text("frame".to_owned()))
            .expect("sets");
        settings
            .set("wm.raise_on_click", &SettingValue::Bool(false))
            .expect("sets");
        let all = settings.all();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&SettingEffect::Compositor(Setting::BellFlash(BellFlash::Frame))));
    }
}

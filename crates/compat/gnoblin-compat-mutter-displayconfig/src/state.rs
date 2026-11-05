//! The display state the adapter reports, and the plan an apply turns into.
//!
//! Two directions, both pure: [`DisplayState::read`] turns the graph's `output`
//! objects into the monitors and logical monitors `GetCurrentState` answers with, and
//! [`ApplyPlan`] turns an `ApplyMonitorsConfig` argument list into one call per output.
//! Nothing here knows D-Bus or the IPC, so the mapping is tested without a bus and a
//! refusal names the connector a caller sent.
//!
//! The slice is one output per logical monitor. A logical monitor that names several
//! connectors is applied by giving each of them the same position, scale and
//! transform, which is what a mirrored layout needs and what the schema's per-output
//! `apply` can express; the compositor's own layout decides how mirrors are driven.
//!
//! # Failure modes first
//!
//! 1. **A mode id is not the shape the schema promises.** Refused, because a mode
//!    reported as 0x0 is one a caller will apply: [`MonitorMode::parse`].
//! 2. **Two outputs claim one connector.** Refused rather than merged, so two views of
//!    the desktop cannot disagree about which name means which display:
//!    [`DisplayState::read`].
//! 3. **The current mode is not one of the output's modes.** Refused, because
//!    `GetCurrentState` would otherwise report a mode no caller can select:
//!    [`DisplayState::read`].
//! 4. **An apply names a connector or a mode the graph does not have.** Refused before
//!    anything is written, so a configuration is rejected whole rather than
//!    half-applied: [`ApplyPlan::validate`].
//! 5. **An apply carries a serial from before the last change.** Refused, so a caller
//!    that raced a hotplug is told to read the state again instead of overwriting it:
//!    [`ApplyPlan::validate`].

use gnoblin_compat_core::LockState;
use gnoblin_model::{Fields, ObjectView, TypeKey, Value};

use crate::error::DisplayError;

/// The object type this adapter reads.
pub const OUTPUT: &str = "output";

/// The separator between a mode's size and its refresh rate.
const REFRESH: char = '@';

/// The separator between a mode's width and its height.
const SIZE: char = 'x';

/// One mode of one output.
///
/// The id is the whole description, which is why parsing it is the only place a mode
/// can be malformed: `<width>x<height>@<refresh>`, the encoding `output.modes`
/// documents.
#[derive(Clone, PartialEq, Debug)]
pub struct MonitorMode {
    /// The id the caller sees and sends back, which is the whole description.
    pub id: String,
    /// Width in physical pixels.
    pub width: i32,
    /// Height in physical pixels.
    pub height: i32,
    /// Refresh rate in Hz.
    pub refresh: f64,
}

impl MonitorMode {
    /// Parse a mode id.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Unreadable`] when the id is not
    /// `<width>x<height>@<refresh>` with positive sizes and a positive refresh rate.
    pub fn parse(object: &str, id: &str) -> Result<MonitorMode, DisplayError> {
        let unreadable = |reason: String| DisplayError::Unreadable {
            object: object.to_owned(),
            property: "modes".to_owned(),
            reason,
        };
        let (size, refresh) = id
            .split_once(REFRESH)
            .ok_or_else(|| unreadable(format!("`{id}` has no `{REFRESH}<refresh>` part")))?;
        let (width, height) = size
            .split_once(SIZE)
            .ok_or_else(|| unreadable(format!("`{id}` has no `<width>{SIZE}<height>` part")))?;
        let width: i32 = width
            .parse()
            .map_err(|_| unreadable(format!("`{width}` is not a width")))?;
        let height: i32 = height
            .parse()
            .map_err(|_| unreadable(format!("`{height}` is not a height")))?;
        let refresh: f64 = refresh
            .parse()
            .map_err(|_| unreadable(format!("`{refresh}` is not a refresh rate")))?;
        if width <= 0 || height <= 0 || refresh <= 0.0 {
            return Err(unreadable(format!(
                "`{id}` has a size or a refresh rate that is not positive"
            )));
        }
        Ok(MonitorMode {
            id: id.to_owned(),
            width,
            height,
            refresh,
        })
    }
}

/// One output, as `GetCurrentState` reports it and as an apply addresses it.
#[derive(Clone, PartialEq, Debug)]
pub struct DisplayOutput {
    /// The connector, which is the identity an apply uses.
    pub name: String,
    /// The vendor.
    pub make: String,
    /// The model.
    pub model: String,
    /// Every supported mode, in the order the compositor published them.
    pub modes: Vec<MonitorMode>,
    /// The current mode.
    pub mode: MonitorMode,
    /// The logical scale.
    pub scale: f64,
    /// The rotation and flip in Mutter's numbering.
    pub transform: u32,
    /// Whether this output carries the primary logical monitor.
    pub primary: bool,
    /// Whether the output drives a logical monitor.
    pub enabled: bool,
    /// The logical monitor's left edge.
    pub x: i32,
    /// The logical monitor's top edge.
    pub y: i32,
    /// The logical monitor's width in logical pixels.
    pub width: u32,
    /// The logical monitor's height in logical pixels.
    pub height: u32,
}

impl DisplayOutput {
    /// Read one `output` object.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Unreadable`] for a property that is absent or is not the value
    /// the schema declares, and [`DisplayError::UnknownMode`] when the current mode is
    /// not one of the listed modes.
    fn read(view: &ObjectView) -> Result<DisplayOutput, DisplayError> {
        let object = text(view, "name")?;
        let listed = list(view, "modes")?;
        let mut modes = Vec::with_capacity(listed.len());
        for id in &listed {
            modes.push(MonitorMode::parse(&object, id)?);
        }
        let current = text(view, "mode")?;
        let mode = modes
            .iter()
            .find(|mode| mode.id == current)
            .cloned()
            .ok_or_else(|| DisplayError::UnknownMode {
                connector: object.clone(),
                mode: current,
            })?;
        Ok(DisplayOutput {
            name: object,
            make: text(view, "make")?,
            model: text(view, "model")?,
            modes,
            mode,
            scale: double(view, "scale")?,
            transform: uint(view, "transform")? as u32,
            primary: boolean(view, "primary")?,
            enabled: boolean(view, "enabled")?,
            x: integer(view, "x")?,
            y: integer(view, "y")?,
            width: uint(view, "width")? as u32,
            height: uint(view, "height")? as u32,
        })
    }
}

/// Every output the graph publishes, in connector order.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DisplayState {
    /// The outputs, sorted by connector so a report and a caller agree on order.
    pub outputs: Vec<DisplayOutput>,
}

impl DisplayState {
    /// Read the state from every object a subscription delivered.
    ///
    /// # Errors
    ///
    /// Any [`DisplayOutput::read`] failure, and
    /// [`DisplayError::DuplicateConnector`] when two objects claim one name.
    pub fn read<'a>(views: impl IntoIterator<Item = &'a ObjectView>) -> Result<DisplayState, DisplayError> {
        let mut outputs: Vec<DisplayOutput> = Vec::new();
        for view in views {
            if view.ty() != &TypeKey::gnoblin(OUTPUT) {
                continue;
            }
            let output = DisplayOutput::read(view)?;
            if let Some(previous) = outputs.iter().find(|other| other.name == output.name) {
                return Err(DisplayError::DuplicateConnector {
                    name: previous.name.clone(),
                });
            }
            outputs.push(output);
        }
        outputs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(DisplayState { outputs })
    }

    /// One output by connector.
    pub fn output(&self, name: &str) -> Option<&DisplayOutput> {
        self.outputs.iter().find(|output| output.name == name)
    }
}

/// How an `ApplyMonitorsConfig` asks for its configuration to be handled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApplyMethod {
    /// Check that the configuration is possible, change nothing.
    Verify,
    /// Apply it until the session ends.
    Temporary,
    /// Apply it and keep it.
    Persistent,
}

impl ApplyMethod {
    /// Parse the wire value: 0 verifies, 1 is temporary, 2 is persistent.
    pub const fn parse(method: u32) -> Option<ApplyMethod> {
        match method {
            0 => Some(ApplyMethod::Verify),
            1 => Some(ApplyMethod::Temporary),
            2 => Some(ApplyMethod::Persistent),
            _ => None,
        }
    }
}

/// One monitor inside a logical monitor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MonitorAssignment {
    /// The connector.
    pub connector: String,
    /// The mode id.
    pub mode: String,
}

/// One logical monitor, as the caller asked for it.
#[derive(Clone, PartialEq, Debug)]
pub struct LogicalMonitor {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Scale.
    pub scale: f64,
    /// Rotation and flip in Mutter's numbering.
    pub transform: u32,
    /// Whether this is the primary logical monitor.
    pub primary: bool,
    /// The monitors it covers.
    pub monitors: Vec<MonitorAssignment>,
}

/// One `ApplyMonitorsConfig` call, ready to be validated.
#[derive(Clone, PartialEq, Debug)]
pub struct ApplyPlan {
    /// The serial the caller read.
    pub serial: u32,
    /// How to handle the configuration.
    pub method: ApplyMethod,
    /// The logical monitors.
    pub logical: Vec<LogicalMonitor>,
}

impl ApplyPlan {
    /// Check the plan against the state the adapter published.
    ///
    /// Nothing is written until this passes, so a caller that names one connector the
    /// graph does not have gets a refusal instead of a half-applied desktop.
    ///
    /// # Errors
    ///
    /// [`DisplayError::StaleSerial`], [`DisplayError::UnknownConnector`],
    /// [`DisplayError::UnsupportedMode`] and [`DisplayError::RepeatedConnector`].
    pub fn validate(&self, state: &DisplayState, serial: u32) -> Result<(), DisplayError> {
        if self.serial != serial {
            return Err(DisplayError::StaleSerial {
                sent: self.serial,
                current: serial,
            });
        }
        let mut seen: Vec<&str> = Vec::new();
        for logical in &self.logical {
            if logical.transform > 7 {
                return Err(DisplayError::Unreadable {
                    object: "configuration".to_owned(),
                    property: "transform".to_owned(),
                    reason: format!("{} is not a transform between 0 and 7", logical.transform),
                });
            }
            for monitor in &logical.monitors {
                if seen.contains(&monitor.connector.as_str()) {
                    return Err(DisplayError::RepeatedConnector {
                        name: monitor.connector.clone(),
                    });
                }
                seen.push(&monitor.connector);
                let output = state
                    .output(&monitor.connector)
                    .ok_or_else(|| DisplayError::UnknownConnector {
                        name: monitor.connector.clone(),
                    })?;
                if !output.modes.iter().any(|mode| mode.id == monitor.mode) {
                    return Err(DisplayError::UnsupportedMode {
                        connector: monitor.connector.clone(),
                        mode: monitor.mode.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// One call per output: the ones the plan names, and a disable for the rest.
    ///
    /// Every output gets a call because an apply is a whole configuration: a connector
    /// the caller left out is switched off, which is what Mutter does and what a caller
    /// that removed a monitor expects.
    pub fn calls(&self, state: &DisplayState) -> Vec<(String, Fields)> {
        let mut calls = Vec::with_capacity(state.outputs.len());
        for output in &state.outputs {
            let planned = self.logical.iter().find_map(|logical| {
                logical
                    .monitors
                    .iter()
                    .position(|monitor| monitor.connector == output.name)
                    .map(|position| (logical, position))
            });
            let fields = match planned {
                Some((logical, position)) => apply_fields(
                    logical.x,
                    logical.y,
                    logical.scale,
                    logical.transform,
                    logical.primary && position == 0,
                    true,
                    &logical.monitors[position].mode,
                ),
                None => apply_fields(
                    output.x,
                    output.y,
                    output.scale,
                    output.transform,
                    false,
                    false,
                    &output.mode.id,
                ),
            };
            calls.push((output.name.clone(), fields));
        }
        calls
    }
}

/// The arguments one `output.apply` call carries.
fn apply_fields(x: i32, y: i32, scale: f64, transform: u32, primary: bool, enabled: bool, mode: &str) -> Fields {
    let mut fields = Fields::new();
    fields.insert("x".to_owned(), Value::Int(i64::from(x)));
    fields.insert("y".to_owned(), Value::Int(i64::from(y)));
    fields.insert("scale".to_owned(), Value::Double(scale));
    fields.insert("transform".to_owned(), Value::Uint(u64::from(transform)));
    fields.insert("primary".to_owned(), Value::Bool(primary));
    fields.insert("enabled".to_owned(), Value::Bool(enabled));
    fields.insert("mode".to_owned(), Value::Text(mode.to_owned()));
    fields
}

fn unreadable(object: &str, property: &str, reason: &str) -> DisplayError {
    DisplayError::Unreadable {
        object: object.to_owned(),
        property: property.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The object's name, which every other error message uses.
fn object_name(view: &ObjectView) -> String {
    match view.property("name") {
        Some(Value::Text(name)) => name.clone(),
        _ => format!("output#{}", view.id().get()),
    }
}

fn text(view: &ObjectView, property: &str) -> Result<String, DisplayError> {
    match view.property(property) {
        Some(Value::Text(value)) => Ok(value.clone()),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a string, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn list(view: &ObjectView, property: &str) -> Result<Vec<String>, DisplayError> {
    let Some(Value::List(items)) = view.property(property) else {
        return Err(unreadable(&object_name(view), property, "expected a list of strings"));
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::Text(value) => values.push(value.clone()),
            other => {
                return Err(unreadable(
                    &object_name(view),
                    property,
                    &format!("expected a list of strings, found a {}", other.kind()),
                ));
            }
        }
    }
    Ok(values)
}

fn double(view: &ObjectView, property: &str) -> Result<f64, DisplayError> {
    match view.property(property) {
        Some(Value::Double(value)) => Ok(*value),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a double, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn uint(view: &ObjectView, property: &str) -> Result<u64, DisplayError> {
    match view.property(property) {
        Some(Value::Uint(value)) => Ok(*value),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a uint, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn integer(view: &ObjectView, property: &str) -> Result<i32, DisplayError> {
    match view.property(property) {
        Some(Value::Int(value)) => i32::try_from(*value)
            .map_err(|_| unreadable(&object_name(view), property, &format!("{value} does not fit an int"))),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected an int, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn boolean(view: &ObjectView, property: &str) -> Result<bool, DisplayError> {
    match view.property(property) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a bool, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

/// The lock state the graph publishes, as the gate's embargo value.
///
/// `true` is the whole embargo: the schema publishes one boolean because the locker's
/// `locking` and `locked` states both hide the desktop, and a caller that missed the
/// transition must not read the finer value as "visible".
pub fn read_lock<'a>(views: impl IntoIterator<Item = &'a ObjectView>) -> Result<LockState, DisplayError> {
    for view in views {
        if view.ty() == &TypeKey::gnoblin("lock") {
            return Ok(if boolean(view, "locked")? {
                LockState::Locked
            } else {
                LockState::Unlocked
            });
        }
    }
    Err(DisplayError::Unreadable {
        object: "lock".to_owned(),
        property: "locked".to_owned(),
        reason: "the graph publishes no `lock` object".to_owned(),
    })
}

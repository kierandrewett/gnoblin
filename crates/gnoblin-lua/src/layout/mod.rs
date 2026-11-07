//! The layout host: Lua layouts run on a worker thread that owns their VM.
//!
//! A layout is a Lua module that the host loads by name, through
//! [`Host::register`]. The host runs it on the worker, and
//! [`LuaEngine`] drives it through the wm's [`gnoblin_wm::layout::LayoutEngine`]
//! contract: a run is submitted without waiting and collected against the frame deadline.
//! Every failure becomes a [`gnoblin_wm::layout::LayoutFailure`], so the slot falls back
//! to the built-in floating placement.
//!
//! # LayoutFailure modes
//!
//! Each row is a [`LayoutReason`] and the test in `tests/layout.rs` that proves it.
//!
//! - `stalled`: a spinning layout exhausts its instruction budget and is aborted.
//! - `refused-reply`: a pathological pattern in the reply is refused before use.
//! - `memory`: a memory bomb hits the VM's memory cap.
//! - `refused-reply`: a bad return (NaN, unknown window id, wrong types) fails validation.
//! - `error`: a layout that raises an error is reported with its Lua message.
//! - `refused-reply`: a declined request (`nil`) falls through to the next handler.
//! - `deadline`: a layout that does not answer by the frame deadline is discarded.
//! - Stale generation: a reply for an older generation is discarded, never applied.
//! - `vm-wedged`: a worker that stops responding is abandoned and replaced through the
//!   test-only wedge hook; after the leak cap (3) layouts are disabled with this reason.
//! - `missing-module` / `invalid-layout`: a module that cannot load, or does not
//!   define a layout, is refused at registration.
//! - Require sandbox: path traversal is refused; only the configured directory loads.
//! - Registration: a duplicate name is refused, and a failed re-register restores the
//!   previous registration.
//! - Determinism: the same snapshot yields the same placements.
//! - State persistence: `data` values persist across calls on one worker.

mod data;
mod engine;
mod host;
mod lua;
mod reference;
mod reply;
mod require;
mod snapshot;
mod worker;

use std::collections::BTreeMap;

use gnoblin_wm::layout::Arg;

pub use data::{Data, DataError, LayoutKey};
pub use engine::LuaEngine;
pub use host::{Host, HostConfig, HostFailure, WedgeHook, default_prefix, installed_module_dir};
pub(crate) use reference::{LayoutModuleLoader, is_layout_name};
pub use require::ModuleRoots;
pub(crate) use require::Modules;
pub use snapshot::{Gaps, LayoutRequest, LayoutSnapshot, SizeHints, SnapshotWindow};

/// The arguments of a layout command, from the JSON object a client sends as `params`.
/// Each value must be a number, a string or a boolean; an empty text is no arguments.
///
/// # Errors
/// A message naming what is wrong when the text is not a JSON object of scalars.
pub fn command_arguments(json: &str) -> Result<BTreeMap<String, Arg>, String> {
    if json.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| format!("params: {error}"))?;
    let serde_json::Value::Object(map) = value else {
        return Err("params must be a JSON object".to_owned());
    };
    map.into_iter()
        .map(|(key, value)| {
            let arg = match value {
                serde_json::Value::Number(number) => Arg::Number(
                    number
                        .as_f64()
                        .ok_or_else(|| format!("params.{key} is not a finite number"))?,
                ),
                serde_json::Value::String(text) => Arg::Text(text),
                serde_json::Value::Bool(flag) => Arg::Bool(flag),
                _ => return Err(format!("params.{key} must be a number, a string or a boolean")),
            };
            Ok((key, arg))
        })
        .collect()
}

/// Why a layout did not produce a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutReason {
    /// The layout did not answer before the frame deadline.
    Deadline,
    /// The layout ran out of memory.
    Memory,
    /// The layout raised an error.
    Error,
    /// The layout exhausted its instruction budget without finishing.
    Stalled,
    /// The layout answered with a reply that failed validation or had the wrong shape.
    RefusedReply,
    /// A worker stopped responding and the leak cap was reached; layouts are disabled.
    VmWedged,
    /// The worker's Lua VM is gone.
    VmDown,
    /// The module that defines the layout could not be loaded.
    MissingModule,
    /// The module loaded but does not define a layout.
    InvalidLayout,
    /// The configuration names a layout module that is not under the root it names, or
    /// that does not exist.
    UnknownModule,
    /// The module's source no longer has the hash the configuration was checked with.
    HashMismatch,
}

impl LayoutReason {
    /// The stable name the compositor reports for this reason.
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutReason::Deadline => "deadline",
            LayoutReason::Memory => "memory",
            LayoutReason::Error => "error",
            LayoutReason::Stalled => "stalled",
            LayoutReason::RefusedReply => "refused-reply",
            LayoutReason::VmWedged => "vm-wedged",
            LayoutReason::VmDown => "vm-down",
            LayoutReason::MissingModule => "missing-module",
            LayoutReason::InvalidLayout => "invalid-layout",
            LayoutReason::UnknownModule => "unknown-module",
            LayoutReason::HashMismatch => "hash-mismatch",
        }
    }
}

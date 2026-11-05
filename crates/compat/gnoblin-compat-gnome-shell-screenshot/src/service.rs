//! The served interface: `org.gnome.Shell.Screenshot`.
//!
//! This is the interface GNOME Shell exports at `/org/gnome/Shell/Screenshot`, and the one
//! `xdg-desktop-portal-gnome`'s screenshot backend and a handful of shell helpers call. Every
//! member is a [`gnoblin_compat_core::OperationClass::Capture`], so the gate decides before this
//! module's mapping does, and the mapping refuses as well: the IPC publishes no image-capture
//! member, so there is nothing to serve even in `unmediated` mode.
//!
//! Three refusals, each distinguishable, which is the point of serving the interface at all
//! rather than not owning the name:
//!
//! | Call | Answer |
//! | ---- | ------ |
//! | any member, `portal` mode | `capture-portal-only`, naming the portal as the path |
//! | any member, locked session, any mode | `locked-capture`, naming the lock |
//! | any member, `unmediated` and unlocked | `unsupported`, naming the missing IPC member |
//!
//! # Failure modes first
//!
//! 1. **Capture is allowed in the default mode.** It is refused with `capture-portal-only`:
//!    `a_screenshot_is_refused_in_portal_mode`.
//! 2. **Capture is allowed while the session is locked.** It is refused with `locked-capture`
//!    even in `unmediated` mode: `a_screenshot_is_refused_while_the_session_is_locked`.
//! 3. **An unsupported method fails silently.** In `unmediated` mode, unlocked, every member
//!    answers `unsupported`: `a_screenshot_is_unsupported_when_capture_is_unmediated` and
//!    `every_method_is_refused_by_name`.
//! 4. **The adapter registers twice.** The second export is refused: `an_adapter_registers_once`.

use std::sync::{Arc, Mutex, MutexGuard};

use gnoblin_compat_core::{Gate, LockState, Mode, Refusal};
use zbus::{Connection, fdo};

use crate::compat::{ScreenshotAdmission, ScreenshotCall, default_gate};
use crate::error::ScreenshotError;

/// The bus name GNOME Shell owns for screenshots, which this adapter takes when it serves.
pub const BUS_NAME: &str = "org.gnome.Shell.Screenshot";

/// The object path the interface is exported on.
pub const PATH: &str = "/org/gnome/Shell/Screenshot";

/// The interface name.
pub const INTERFACE: &str = "org.gnome.Shell.Screenshot";

/// The interface's version. Upstream carries no version member, so this is the one the name has
/// always meant.
pub const VERSION: u32 = 1;

/// Why every member refuses when the gate lets the call through.
const NO_CAPTURE: &str =
    "the IPC publishes no image-capture member, so capture reaches the desktop only through the portal's frame export";

/// The state the object server and the host share.
struct ScreenshotShared {
    admission: ScreenshotAdmission,
    gate: Mutex<Gate>,
    lock: Mutex<LockState>,
    exported: Mutex<bool>,
}

impl std::fmt::Debug for ScreenshotShared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenshotShared")
            .field("admission", &self.admission)
            .field("gate", &self.gate)
            .field("lock", &self.lock)
            .field("exported", &self.exported)
            .finish()
    }
}

/// The `Screenshot` object: exportable, clonable and cheap.
#[derive(Clone, Debug)]
pub struct ScreenshotService {
    shared: Arc<ScreenshotShared>,
}

impl ScreenshotService {
    /// An adapter with the default gate.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn new() -> Result<ScreenshotService, gnoblin_compat_core::MatrixError> {
        ScreenshotService::with_gate(default_gate())
    }

    /// An adapter with a gate the composition root chose.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn with_gate(gate: Gate) -> Result<ScreenshotService, gnoblin_compat_core::MatrixError> {
        Ok(ScreenshotService {
            shared: Arc::new(ScreenshotShared {
                admission: ScreenshotAdmission::load()?,
                gate: Mutex::new(gate),
                lock: Mutex::new(LockState::Unlocked),
                exported: Mutex::new(false),
            }),
        })
    }

    /// Set the mode the gate screens with, which is `gnoblin.configure.compat.capture`.
    pub fn set_mode(&self, mode: Mode) {
        let mut gate = self.gate_lock();
        *gate = Gate::new(mode, gate.caller(), gate.lock());
    }

    /// The mode the gate screens with.
    pub fn mode(&self) -> Mode {
        self.gate_lock().mode()
    }

    /// Record the lock state the link read.
    ///
    /// There is no signal to raise: this adapter has nothing to announce, and the embargo is
    /// read per call rather than pushed.
    pub fn publish(&self, lock: LockState) {
        *self.shared.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = lock;
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// # Errors
    ///
    /// [`ScreenshotError::Exported`] when this service was already exported.
    pub fn take_export(&self) -> Result<(), ScreenshotError> {
        let mut exported = self
            .shared
            .exported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *exported {
            return Err(ScreenshotError::Exported);
        }
        *exported = true;
        Ok(())
    }

    fn gate_lock(&self) -> MutexGuard<'_, Gate> {
        self.shared.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The gate for one call: the configured mode, an unsandboxed caller and the lock state the
    /// link last read.
    fn gate(&self) -> Gate {
        let mode = self.gate_lock().mode();
        let lock = *self.shared.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Gate::new(mode, Default::default(), lock)
    }

    fn admit(&self, call: ScreenshotCall) -> Result<(), fdo::Error> {
        self.shared
            .admission
            .admit(call, self.gate())
            .map_err(|refusal: Refusal| fdo::Error::Failed(refusal.to_string()))
    }

    /// The answer every admitted member gives: the matrix's `unsupported`.
    fn unsupported(&self, call: ScreenshotCall) -> fdo::Error {
        match self.admit(call) {
            Ok(()) => fdo::Error::Failed(format!(
                "{} is unsupported: {NO_CAPTURE}",
                <ScreenshotCall as gnoblin_compat_core::Call>::member(&call)
            )),
            Err(error) => error,
        }
    }
}

/// The interface itself.
///
/// The methods are private on purpose: the object server calls them, and a refusal is the whole
/// of what this adapter does with a call. Each declares the signature upstream declares, so a
/// caller's own deserialization would work if one ever succeeded.
#[zbus::interface(name = "org.gnome.Shell.Screenshot")]
impl ScreenshotService {
    /// The whole screen, saved to `filename`.
    fn screenshot(&self, _include_cursor: bool, _flash: bool, _filename: String) -> fdo::Result<(bool, String)> {
        Err(self.unsupported(ScreenshotCall::Screenshot))
    }

    /// A rectangle of the screen, saved to `filename`.
    fn screenshot_area(
        &self,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
        _flash: bool,
        _filename: String,
    ) -> fdo::Result<(bool, String)> {
        Err(self.unsupported(ScreenshotCall::ScreenshotArea))
    }

    /// The focused window, saved to `filename`.
    fn screenshot_window(
        &self,
        _include_frame: bool,
        _include_cursor: bool,
        _flash: bool,
        _filename: String,
    ) -> fdo::Result<(bool, String)> {
        Err(self.unsupported(ScreenshotCall::ScreenshotWindow))
    }

    /// An interactive selection, answered with its rectangle.
    fn select_area(&self) -> fdo::Result<(i32, i32, i32, i32)> {
        Err(self.unsupported(ScreenshotCall::SelectArea))
    }

    /// A flash over a rectangle.
    fn flash_area(&self, _x: i32, _y: i32, _width: i32, _height: i32) -> fdo::Result<()> {
        Err(self.unsupported(ScreenshotCall::FlashArea))
    }

    /// The shell's interactive screenshot dialog.
    fn interactive_screenshot(&self) -> fdo::Result<(bool, String)> {
        Err(self.unsupported(ScreenshotCall::InteractiveScreenshot))
    }

    /// The colour under the pointer.
    fn pick_color(&self) -> fdo::Result<std::collections::HashMap<String, zbus::OwnedVariant>> {
        Err(self.unsupported(ScreenshotCall::PickColor))
    }
}

/// Export the object and own the name.
///
/// The object is exported before the name is requested, because a caller that resolves the name
/// must find an object behind it: a name without an object is a method call that never gets a
/// reply, and a caller waiting for one never learns it should have used the portal.
///
/// # Errors
///
/// [`ScreenshotError::NameTaken`] when another process owns the name, which is the ordinary case
/// while GNOME Shell is running beside this adapter.
pub async fn export(connection: &Connection, service: &ScreenshotService) -> Result<(), ScreenshotError> {
    service.take_export()?;
    connection
        .object_server()
        .at(PATH, service.clone())
        .await
        .map_err(|error| ScreenshotError::Bus {
            reason: error.to_string(),
        })?;
    connection
        .request_name_with_flags(BUS_NAME, fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
        .map_err(|_| ScreenshotError::NameTaken {
            name: BUS_NAME.to_owned(),
        })?;
    tracing::info!(name = BUS_NAME, path = PATH, "refusing screenshots on the shell's name");
    Ok(())
}

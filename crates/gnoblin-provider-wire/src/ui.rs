//! The portal dialog UI across the ABI: [`export_ui`] on std's side and [`ForeignUi`] on the core's.
//!
//! The wire form is a vtable: the trait has seventeen methods and every one of
//! them is a call with plain arguments, so each is one function of [`GnoblinUiVtable`]. What is not plain crosses as bytes the
//! portal's own crate defines (`gnoblin_portal_core::wire` for a dialog, a verdict, an interface and a key;
//! `DialogStyle::to_bytes` for the look), and what the provider hands back (a frame, the verdicts that settled) goes through a
//! sink the core passes, so neither side frees the other's memory. A node-tree description was the alternative; it would
//! have made the dialogs tree nodes the shell could restyle, which is the shell's own concern and not the provider's.
//!
//! A provider that fails (a panic, an error, bytes that do not decode) is dropped for the rest of the session. [`ForeignUi`]
//! then refuses every new request `no-ui`, and settles the requests it had admitted `error(provider-failed)`, so a consent that
//! can no longer be shown is never granted and never left waiting.

use core::ffi::c_void;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{
    self as abi, GnoblinDropFn, GnoblinStr, GnoblinUiFrameSink, GnoblinUiOutcomeSink, GnoblinUiVtable,
};
use gnoblin_portal_core::model::{Dialog, Interface, ScreenKey};
use gnoblin_portal_core::wire;
use gnoblin_portal_core::{PortalError, Verdict};
use gnoblin_portal_ui::paint::Frame;
use gnoblin_portal_ui::provider::{SubmitOutcome, UiProvider};
use gnoblin_portal_ui::style::DialogStyle;
use gnoblin_portal_ui::ui::DialogScheme;
use gnoblin_types::{RequestId, Tick};

use crate::{Exported, Report, guarded};

// ---- std's side: a Rust provider as a vtable ---------------------------------------------------------------------------

/// What `user` points at: the provider.
struct UiHost {
    ui: Box<dyn UiProvider>,
}

static VTABLE: GnoblinUiVtable = GnoblinUiVtable {
    size: size_of::<GnoblinUiVtable>() as u32,
    reserved: 0,
    frame_size: host_frame_size,
    set_scheme: host_set_scheme,
    set_style: host_set_style,
    set_bounds: host_set_bounds,
    active_request: host_active_request,
    waiting: host_waiting,
    submit: host_submit,
    key: host_key,
    pointer_press: host_pointer_press,
    pointer_release: host_pointer_release,
    tick: host_tick,
    take_frame: host_take_frame,
    redraw: host_redraw,
    take_outcomes: host_take_outcomes,
    cancel: host_cancel,
    set_locked: host_set_locked,
};

/// Turns `ui` into a provider the core can register with `provider_register(PROVIDER_UI)`.
#[must_use]
pub fn export_ui(ui: Box<dyn UiProvider>) -> Exported<GnoblinUiVtable> {
    Exported {
        vtable: &VTABLE,
        user: Box::into_raw(Box::new(UiHost { ui })).cast(),
        drop_user: host_drop,
    }
}

/// # Safety
/// `user` is the pointer [`export_ui`] made, not yet dropped, used by one thread at a time.
unsafe fn host<'a>(user: *mut c_void) -> &'a mut UiHost {
    // SAFETY: the caller guarantees `user` points at a live `UiHost`.
    unsafe { &mut *user.cast::<UiHost>() }
}

unsafe extern "C" fn host_drop(user: *mut c_void) {
    // SAFETY: the core calls this once, with the `user` pointer `export_ui` made.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(user.cast::<UiHost>()) })
    }));
}

unsafe extern "C" fn host_frame_size(user: *mut c_void, width: *mut u32, height: *mut u32) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract: `user` is ours, the out pointers are writable.
        unsafe {
            let (w, h) = host(user).ui.size();
            let (Some(width), Some(height)) = (width.as_mut(), height.as_mut()) else {
                return abi::E_ERR;
            };
            *width = u32::try_from(w).unwrap_or(u32::MAX);
            *height = u32::try_from(h).unwrap_or(u32::MAX);
        }
        abi::E_OK
    })
}

unsafe extern "C" fn host_set_scheme(user: *mut c_void, scheme: u32) -> i32 {
    guarded(|| {
        let scheme = match scheme {
            abi::UI_SCHEME_DARK => DialogScheme::Dark,
            abi::UI_SCHEME_LIGHT => DialogScheme::Light,
            _ => return abi::E_ERR,
        };
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.set_scheme(scheme);
        abi::E_OK
    })
}

/// The bytes a `(pointer, length)` pair names.
///
/// # Safety
/// `ptr` is null or readable for `len` bytes for the call.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller guarantees the range is readable.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

unsafe extern "C" fn host_set_style(user: *mut c_void, ptr: *const u8, len: usize) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        let style = DialogStyle::from_bytes(unsafe { bytes(ptr, len) });
        let Some(style) = style else {
            return abi::E_ERR;
        };
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.set_style(style);
        abi::E_OK
    })
}

unsafe extern "C" fn host_set_bounds(user: *mut c_void, width: u32, height: u32) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.set_bounds(width as usize, height as usize);
        abi::E_OK
    })
}

unsafe extern "C" fn host_active_request(user: *mut c_void, request: *mut u64) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours; `request` is writable.
        unsafe {
            match (host(user).ui.active_request(), request.as_mut()) {
                (Some(active), Some(out)) => {
                    *out = active.get();
                    1
                }
                (None, _) => 0,
                (Some(_), None) => abi::E_ERR,
            }
        }
    })
}

unsafe extern "C" fn host_waiting(user: *mut c_void) -> usize {
    // SAFETY: `user` is ours.
    catch_unwind(AssertUnwindSafe(|| unsafe { host(user) }.ui.waiting())).unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn host_submit(
    user: *mut c_void,
    request: u64,
    app_id: GnoblinStr,
    interface: u32,
    dialog: *const u8,
    dialog_len: usize,
    timeout_ms: u64,
    now: u64,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract: the strings and bytes are valid for the call.
        let (app, dialog) = unsafe {
            (
                std::str::from_utf8(bytes(app_id.ptr, app_id.len)),
                wire::decode_dialog(bytes(dialog, dialog_len)),
            )
        };
        let (Ok(app), Ok(dialog), Ok(interface)) = (app, dialog, wire::interface_of(interface)) else {
            return abi::E_ERR;
        };
        // SAFETY: `user` is ours.
        let outcome = unsafe { host(user) }.ui.submit(
            RequestId::new(request),
            app,
            interface,
            dialog,
            timeout_ms,
            Tick::new(now),
        );
        match outcome {
            Ok(SubmitOutcome::Started) => abi::UI_STARTED,
            Ok(SubmitOutcome::Queued) => abi::UI_QUEUED,
            Err(error) => abi::UI_REFUSED + i32::try_from(wire::error_code(error)).unwrap_or(0),
        }
    })
}

unsafe extern "C" fn host_key(user: *mut c_void, kind: u32, character: u32, repeat: u32) -> i32 {
    guarded(|| {
        let Ok(key) = wire::key_of(kind, character) else {
            return abi::E_ERR;
        };
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.key(key, repeat != 0);
        abi::E_OK
    })
}

unsafe extern "C" fn host_pointer_press(user: *mut c_void, x: f32, y: f32) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.pointer_press(x, y);
        abi::E_OK
    })
}

unsafe extern "C" fn host_pointer_release(user: *mut c_void, x: f32, y: f32) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.pointer_release(x, y);
        abi::E_OK
    })
}

unsafe extern "C" fn host_tick(user: *mut c_void, now: u64) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.tick(Tick::new(now));
        abi::E_OK
    })
}

unsafe extern "C" fn host_take_frame(user: *mut c_void, sink: *const GnoblinUiFrameSink) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours; `sink` is the core's, valid for the call.
        unsafe {
            let Some(sink) = sink.as_ref() else {
                return abi::E_ERR;
            };
            let Some(frame) = host(user).ui.take_frame() else {
                return abi::E_OK;
            };
            let bytes = frame.bytes();
            (sink.frame)(
                sink.sink,
                u32::try_from(frame.width()).unwrap_or(u32::MAX),
                u32::try_from(frame.height()).unwrap_or(u32::MAX),
                bytes.as_ptr(),
                bytes.len(),
            )
        }
    })
}

unsafe extern "C" fn host_redraw(user: *mut c_void) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.redraw();
        abi::E_OK
    })
}

unsafe extern "C" fn host_take_outcomes(user: *mut c_void, sink: *const GnoblinUiOutcomeSink) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours; `sink` is the core's, valid for the call.
        unsafe {
            let Some(sink) = sink.as_ref() else {
                return abi::E_ERR;
            };
            for (request, verdict) in host(user).ui.take_outcomes() {
                let encoded = wire::encode_verdict(&verdict);
                let rc = (sink.verdict)(sink.sink, request.get(), encoded.as_ptr(), encoded.len());
                if rc != abi::E_OK {
                    return rc;
                }
            }
        }
        abi::E_OK
    })
}

unsafe extern "C" fn host_cancel(user: *mut c_void, request: u64) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        i32::from(unsafe { host(user) }.ui.cancel(RequestId::new(request)))
    })
}

unsafe extern "C" fn host_set_locked(user: *mut c_void, locked: u32) -> i32 {
    guarded(|| {
        // SAFETY: `user` is ours.
        unsafe { host(user) }.ui.set_locked(locked != 0);
        abi::E_OK
    })
}

// ---- the core's side: a vtable as a Rust provider ----------------------------------------------------------------------

/// A portal UI registered through the ABI, as the compositor's [`UiProvider`].
pub struct ForeignUi {
    vtable: GnoblinUiVtable,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    report: Report,
    /// The requests admitted and not yet settled, so a provider that goes away leaves none waiting.
    admitted: BTreeSet<RequestId>,
    /// The provider was dropped: every call answers as no UI would.
    failed: Cell<bool>,
}

// SAFETY: `ForeignUi` holds a raw `user` pointer and function pointers. The registration's contract says the provider's
// functions run on the compositor's IPC thread only, after the registering thread has handed the provider over.
unsafe impl Send for ForeignUi {}

impl std::fmt::Debug for ForeignUi {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForeignUi")
            .field("failed", &self.failed.get())
            .field("admitted", &self.admitted.len())
            .finish_non_exhaustive()
    }
}

impl Drop for ForeignUi {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: `drop_user` and `user` came from one registration; this is the one call it is owed.
            unsafe { drop_user(self.user) };
        }
    }
}

/// Collects the frame the provider gives the sink.
struct FrameCatch(Option<Frame>);

unsafe extern "C" fn catch_frame(sink: *mut c_void, width: u32, height: u32, rgba: *const u8, len: usize) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `sink` is the `FrameCatch` `take_frame` made; `rgba` is readable for `len` bytes for this call.
        let (catch, rgba) = unsafe { (&mut *sink.cast::<FrameCatch>(), bytes(rgba, len)) };
        match Frame::from_rgba(width as usize, height as usize, rgba.to_vec()) {
            Some(frame) => {
                catch.0 = Some(frame);
                abi::E_OK
            }
            None => abi::E_ERR,
        }
    }))
    .unwrap_or(abi::E_PANIC)
}

/// Collects the verdicts the provider gives the sink.
#[derive(Default)]
struct OutcomeCatch {
    settled: Vec<(RequestId, Verdict)>,
    invalid: bool,
}

unsafe extern "C" fn catch_verdict(sink: *mut c_void, request: u64, ptr: *const u8, len: usize) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `sink` is the `OutcomeCatch` `take_outcomes` made; the bytes are readable for this call.
        let (catch, encoded) = unsafe { (&mut *sink.cast::<OutcomeCatch>(), bytes(ptr, len)) };
        match wire::decode_verdict(encoded) {
            Ok(verdict) => {
                catch.settled.push((RequestId::new(request), verdict));
                abi::E_OK
            }
            Err(_) => {
                catch.invalid = true;
                abi::E_ERR
            }
        }
    }))
    .unwrap_or(abi::E_PANIC)
}

impl ForeignUi {
    /// Wraps the provider a registration named. `report` says, once, why the provider was dropped. The wrapper does not own
    /// `user` yet: until [`ForeignUi::adopt`] it is the registrant's.
    ///
    /// # Errors
    /// The vtable is shorter than this build's.
    ///
    /// # Safety
    /// `vtable` points at a vtable whose functions may be called with `user` from one thread at a time for as long as the
    /// result lives.
    pub unsafe fn new(vtable: *const GnoblinUiVtable, user: *mut c_void, report: Report) -> Result<Self, String> {
        // SAFETY: the caller guarantees the pointer is null or valid for `size` bytes.
        let Some(table) = (unsafe { vtable.as_ref() }) else {
            return Err("a null portal UI vtable".to_owned());
        };
        if (table.size as usize) < size_of::<GnoblinUiVtable>() {
            return Err(format!(
                "the portal UI vtable is {} bytes; this core needs {}",
                table.size,
                size_of::<GnoblinUiVtable>()
            ));
        }
        Ok(Self {
            vtable: *table,
            user,
            drop_user: None,
            report,
            admitted: BTreeSet::new(),
            failed: Cell::new(false),
        })
    }

    /// Takes ownership of `user`: `drop_user` runs on it once, when the provider ends.
    pub fn adopt(&mut self, drop_user: Option<GnoblinDropFn>) {
        self.drop_user = drop_user;
    }

    /// Whether the provider was dropped.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.failed.get()
    }

    /// Marks the provider dropped and says why, once.
    fn fail(&self, what: &str, rc: i32) {
        if !self.failed.replace(true) {
            (self.report)(&format!(
                "the portal UI provider answered {rc} to {what}; portal requests are refused no-ui from now on"
            ));
        }
    }

    /// Whether a call that answers `E_OK` or an error went through. A failed provider answers nothing.
    fn went(&self, what: &str, rc: i32) -> bool {
        if rc >= 0 {
            return true;
        }
        self.fail(what, rc);
        false
    }

    /// Calls `body` unless the provider failed.
    fn call(&self, what: &str, body: impl FnOnce(&GnoblinUiVtable, *mut c_void) -> i32) -> bool {
        if self.failed.get() {
            return false;
        }
        let rc = body(&self.vtable, self.user);
        self.went(what, rc)
    }
}

impl UiProvider for ForeignUi {
    fn size(&self) -> (usize, usize) {
        let (mut width, mut height) = (0u32, 0u32);
        // SAFETY: the registration's contract; the out pointers are live locals.
        let went = self.call("frame_size", |vt, user| unsafe {
            (vt.frame_size)(user, &raw mut width, &raw mut height)
        });
        if went {
            (width as usize, height as usize)
        } else {
            (0, 0)
        }
    }

    fn set_scheme(&mut self, scheme: DialogScheme) {
        let scheme = match scheme {
            DialogScheme::Dark => abi::UI_SCHEME_DARK,
            DialogScheme::Light => abi::UI_SCHEME_LIGHT,
        };
        // SAFETY: the registration's contract.
        self.call("set_scheme", |vt, user| unsafe { (vt.set_scheme)(user, scheme) });
    }

    fn set_style(&mut self, style: DialogStyle) {
        let encoded = style.to_bytes();
        // SAFETY: the registration's contract; `encoded` lives for the call.
        self.call("set_style", |vt, user| unsafe {
            (vt.set_style)(user, encoded.as_ptr(), encoded.len())
        });
    }

    fn set_bounds(&mut self, width: usize, height: usize) {
        let (width, height) = (
            u32::try_from(width).unwrap_or(u32::MAX),
            u32::try_from(height).unwrap_or(u32::MAX),
        );
        // SAFETY: the registration's contract.
        self.call("set_bounds", |vt, user| unsafe { (vt.set_bounds)(user, width, height) });
    }

    fn active_request(&self) -> Option<RequestId> {
        let mut request = 0u64;
        let mut open = false;
        // SAFETY: the registration's contract; `request` is a live local.
        self.call("active_request", |vt, user| {
            let rc = unsafe { (vt.active_request)(user, &raw mut request) };
            open = rc == 1;
            rc
        });
        open.then(|| RequestId::new(request))
    }

    fn waiting(&self) -> usize {
        let mut waiting = 0;
        // SAFETY: the registration's contract.
        self.call("waiting", |vt, user| {
            waiting = unsafe { (vt.waiting)(user) };
            abi::E_OK
        });
        waiting
    }

    fn submit(
        &mut self,
        request: RequestId,
        app_id: &str,
        interface: Interface,
        dialog: Dialog,
        timeout_ms: u64,
        now: Tick,
    ) -> Result<SubmitOutcome, PortalError> {
        if self.failed.get() {
            return Err(PortalError::NoUi);
        }
        let encoded = wire::encode_dialog(&dialog);
        let app = GnoblinStr::borrowed(app_id);
        // SAFETY: the registration's contract; the borrowed string and the bytes live for the call.
        let rc = unsafe {
            (self.vtable.submit)(
                self.user,
                request.get(),
                app,
                wire::interface_code(interface),
                encoded.as_ptr(),
                encoded.len(),
                timeout_ms,
                now.get(),
            )
        };
        match rc {
            abi::UI_STARTED => {
                self.admitted.insert(request);
                Ok(SubmitOutcome::Started)
            }
            abi::UI_QUEUED => {
                self.admitted.insert(request);
                Ok(SubmitOutcome::Queued)
            }
            refused if refused >= abi::UI_REFUSED => {
                let code = u32::try_from(refused - abi::UI_REFUSED).unwrap_or(u32::MAX);
                match wire::error_of(code) {
                    Ok(error) => Err(error),
                    Err(_) => {
                        self.fail("submit", refused);
                        Err(PortalError::ProviderFailed)
                    }
                }
            }
            other => {
                self.fail("submit", other);
                Err(PortalError::ProviderFailed)
            }
        }
    }

    fn key(&mut self, key: ScreenKey, repeat: bool) {
        let (kind, character) = wire::key_code(key);
        // SAFETY: the registration's contract.
        self.call("key", |vt, user| unsafe {
            (vt.key)(user, kind, character, u32::from(repeat))
        });
    }

    fn pointer_press(&mut self, x: f32, y: f32) {
        // SAFETY: the registration's contract.
        self.call("pointer_press", |vt, user| unsafe { (vt.pointer_press)(user, x, y) });
    }

    fn pointer_release(&mut self, x: f32, y: f32) {
        // SAFETY: the registration's contract.
        self.call("pointer_release", |vt, user| unsafe {
            (vt.pointer_release)(user, x, y)
        });
    }

    fn tick(&mut self, now: Tick) {
        // SAFETY: the registration's contract.
        self.call("tick", |vt, user| unsafe { (vt.tick)(user, now.get()) });
    }

    fn take_frame(&mut self) -> Option<Frame> {
        let mut catch = FrameCatch(None);
        let sink = GnoblinUiFrameSink {
            size: size_of::<GnoblinUiFrameSink>() as u32,
            reserved: 0,
            sink: (&raw mut catch).cast(),
            frame: catch_frame,
        };
        // SAFETY: the registration's contract; `sink` and `catch` live for the call.
        self.call("take_frame", |vt, user| unsafe {
            (vt.take_frame)(user, &raw const sink)
        });
        catch.0
    }

    fn redraw(&mut self) {
        // SAFETY: the registration's contract.
        self.call("redraw", |vt, user| unsafe { (vt.redraw)(user) });
    }

    fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)> {
        if self.failed.get() {
            return self.settle_admitted(PortalError::ProviderFailed);
        }
        let mut catch = OutcomeCatch::default();
        let sink = GnoblinUiOutcomeSink {
            size: size_of::<GnoblinUiOutcomeSink>() as u32,
            reserved: 0,
            sink: (&raw mut catch).cast(),
            verdict: catch_verdict,
        };
        // SAFETY: the registration's contract; `sink` and `catch` live for the call.
        let went = self.call("take_outcomes", |vt, user| unsafe {
            (vt.take_outcomes)(user, &raw const sink)
        });
        if catch.invalid && !self.failed.get() {
            self.fail("take_outcomes", abi::E_ERR);
        }
        for (request, _) in &catch.settled {
            self.admitted.remove(request);
        }
        let mut settled = catch.settled;
        if !went {
            settled.extend(self.settle_admitted(PortalError::ProviderFailed));
        }
        settled
    }

    fn cancel(&mut self, request: RequestId) -> bool {
        let mut known = false;
        // SAFETY: the registration's contract.
        self.call("cancel", |vt, user| {
            let rc = unsafe { (vt.cancel)(user, request.get()) };
            known = rc == 1;
            rc
        });
        known
    }

    fn set_locked(&mut self, locked: bool) {
        // SAFETY: the registration's contract.
        self.call("set_locked", |vt, user| unsafe {
            (vt.set_locked)(user, u32::from(locked))
        });
    }

    fn close(&mut self) -> Vec<(RequestId, Verdict)> {
        let mut settled = if self.failed.get() {
            Vec::new()
        } else {
            self.take_outcomes()
        };
        // Nothing is called after this: whatever the provider still held is refused.
        self.failed.set(true);
        settled.extend(self.settle_admitted(PortalError::NoUi));
        settled
    }
}

impl ForeignUi {
    /// Settles every request still admitted with `error`.
    fn settle_admitted(&mut self, error: PortalError) -> Vec<(RequestId, Verdict)> {
        std::mem::take(&mut self.admitted)
            .into_iter()
            .map(|request| (request, Verdict::Error(error)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_portal_core::model::{Consent, Grant};

    use super::*;

    /// A UI that admits one request and settles it when it is cancelled, and counts what it is told.
    #[derive(Default)]
    struct Recorder {
        open: Option<RequestId>,
        settled: Vec<(RequestId, Verdict)>,
        style: Option<DialogStyle>,
        scheme: Option<DialogScheme>,
        keys: Vec<ScreenKey>,
        bounds: (usize, usize),
    }

    impl UiProvider for Recorder {
        fn size(&self) -> (usize, usize) {
            (320, 200)
        }
        fn set_scheme(&mut self, scheme: DialogScheme) {
            self.scheme = Some(scheme);
        }
        fn set_style(&mut self, style: DialogStyle) {
            self.style = Some(style);
        }
        fn set_bounds(&mut self, width: usize, height: usize) {
            self.bounds = (width, height);
        }
        fn active_request(&self) -> Option<RequestId> {
            self.open
        }
        fn waiting(&self) -> usize {
            0
        }
        fn submit(
            &mut self,
            request: RequestId,
            _app_id: &str,
            interface: Interface,
            dialog: Dialog,
            _timeout_ms: u64,
            _now: Tick,
        ) -> Result<SubmitOutcome, PortalError> {
            if interface == Interface::Print {
                return Err(PortalError::UnsupportedUi);
            }
            assert!(matches!(dialog, Dialog::Consent(_)));
            self.open = Some(request);
            Ok(SubmitOutcome::Started)
        }
        fn key(&mut self, key: ScreenKey, _repeat: bool) {
            self.keys.push(key);
            if let Some(request) = self.open.take() {
                self.settled.push((request, Verdict::Handled(Grant::Allowed)));
            }
        }
        fn pointer_press(&mut self, _x: f32, _y: f32) {}
        fn pointer_release(&mut self, _x: f32, _y: f32) {}
        fn tick(&mut self, _now: Tick) {}
        fn take_frame(&mut self) -> Option<Frame> {
            self.open.map(|_| Frame::filled(2, 2, [1, 2, 3, 255]))
        }
        fn redraw(&mut self) {}
        fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)> {
            std::mem::take(&mut self.settled)
        }
        fn cancel(&mut self, request: RequestId) -> bool {
            self.open.take_if(|open| *open == request).is_some()
        }
        fn set_locked(&mut self, _locked: bool) {}
    }

    fn through_the_abi() -> (ForeignUi, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let exported = export_ui(Box::<Recorder>::default());
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&lines);
        let report: Report = Box::new(move |line| sink.lock().expect("lock").push(line.to_owned()));
        // SAFETY: the vtable and state are `export_ui`'s; `drop_user` frees the state once.
        let mut foreign = unsafe { ForeignUi::new(exported.vtable, exported.user, report) }.expect("wraps");
        foreign.adopt(Some(exported.drop_user));
        (foreign, lines)
    }

    fn consent() -> Dialog {
        Dialog::Consent(Consent::open("org.example.App", "Share your screen?", Tick::new(5)))
    }

    #[test]
    fn a_dialog_goes_in_a_frame_and_a_verdict_come_out_across_the_vtable() {
        let (mut ui, lines) = through_the_abi();
        assert_eq!(ui.size(), (320, 200));
        ui.set_style(DialogStyle::default());
        ui.set_scheme(DialogScheme::Light);
        ui.set_bounds(640, 360);
        let started = ui.submit(
            RequestId::new(7),
            "org.example.App",
            Interface::Screenshot,
            consent(),
            1000,
            Tick::new(5),
        );
        assert_eq!(started, Ok(SubmitOutcome::Started));
        assert_eq!(ui.active_request(), Some(RequestId::new(7)));
        assert_eq!(ui.take_frame(), Some(Frame::filled(2, 2, [1, 2, 3, 255])));
        ui.key(ScreenKey::Enter, false);
        assert_eq!(
            ui.take_outcomes(),
            vec![(RequestId::new(7), Verdict::Handled(Grant::Allowed))]
        );
        assert!(ui.take_outcomes().is_empty(), "a verdict is taken once");
        assert!(lines.lock().expect("lock").is_empty());
    }

    #[test]
    fn a_refusal_is_the_providers_error_not_a_failure() {
        let (mut ui, lines) = through_the_abi();
        let refused = ui.submit(RequestId::new(1), "a", Interface::Print, consent(), 1000, Tick::ZERO);
        assert_eq!(refused, Err(PortalError::UnsupportedUi));
        assert!(!ui.failed());
        assert!(lines.lock().expect("lock").is_empty());
    }

    #[test]
    fn a_provider_that_goes_away_leaves_no_request_waiting() {
        let (mut ui, _) = through_the_abi();
        ui.submit(
            RequestId::new(3),
            "a",
            Interface::Screenshot,
            consent(),
            1000,
            Tick::ZERO,
        )
        .expect("admitted");
        let settled = ui.close();
        assert_eq!(settled, vec![(RequestId::new(3), Verdict::Error(PortalError::NoUi))]);
        assert_eq!(
            ui.submit(
                RequestId::new(4),
                "a",
                Interface::Screenshot,
                consent(),
                1000,
                Tick::ZERO
            ),
            Err(PortalError::NoUi),
            "once it is gone nothing is admitted"
        );
    }

    unsafe extern "C" fn panics(
        _: *mut c_void,
        _: u64,
        _: GnoblinStr,
        _: u32,
        _: *const u8,
        _: usize,
        _: u64,
        _: u64,
    ) -> i32 {
        abi::E_PANIC
    }

    #[test]
    fn a_provider_that_panics_is_dropped_and_the_next_request_is_refused_no_ui() {
        let exported = export_ui(Box::<Recorder>::default());
        let mut table = *exported.vtable;
        table.submit = panics;
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&lines);
        let report: Report = Box::new(move |line| sink.lock().expect("lock").push(line.to_owned()));
        // SAFETY: the table is a copy of `export_ui`'s with one function replaced; the state is `export_ui`'s.
        let mut ui = unsafe { ForeignUi::new(&raw const table, exported.user, report) }.expect("wraps");
        ui.adopt(Some(exported.drop_user));
        let first = ui.submit(
            RequestId::new(1),
            "a",
            Interface::Screenshot,
            consent(),
            1000,
            Tick::ZERO,
        );
        assert_eq!(first, Err(PortalError::ProviderFailed));
        let second = ui.submit(
            RequestId::new(2),
            "a",
            Interface::Screenshot,
            consent(),
            1000,
            Tick::ZERO,
        );
        assert_eq!(second, Err(PortalError::NoUi));
        assert_eq!(lines.lock().expect("lock").len(), 1, "said once");
    }
}

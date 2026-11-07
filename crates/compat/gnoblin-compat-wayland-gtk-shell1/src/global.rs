//! The global, its objects and the dispatch that turns a request into a host call.
//!
//! [`Redirect`] is the value a session is given, and registering it creates
//! `gtk_shell1`. Everything a request means is decided by [`crate::mapping`]; this
//! module is the wire half — which object, which user data, which answer — and the
//! only place that knows a request's name.
//!
//! # What each object remembers
//!
//! A `gtk_shell1` object remembers the [`Admission`], because every request is gated
//! before it is mapped. A `gtk_surface1` object remembers the `wl_surface` it was
//! created for, which is the identity every request resolves its window through: the
//! protocol has no other way to say *which* window, and a surface that has died is
//! answered [`gnoblin_wayland::Surface::Gone`] rather than guessed at.
//!
//! # Why the handlers are generic over the host
//!
//! The request bodies are [`handle_shell_request`] and [`handle_surface_request`],
//! generic over [`gnoblin_wayland::Shell1Host`], and the two `Dispatch2`
//! implementations below are one-liners that pass `gnoblin_wayland::State`. A test
//! binds the same global to its own state type and drives the same code: the transport
//! is identical and the only thing a test replaces is the compositor.

use std::sync::Arc;

use gnoblin_compat_core::{CallerContext, Gate, LockState, Mode};

use gnoblin_wayland::{
    Activation, Dispatch2, Extension, ExtensionError, GlobalDispatch2, Shell1Host, State, Surface, Titlebar,
    TitlebarActions,
};
use wayland_backend::protocol::WEnum;
use wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource};

use crate::compat::{Admission, Call};
use crate::mapping::UnknownGesture;
use crate::protocol::{gtk_shell1, gtk_surface1};

use crate::log;

/// The `gtk_shell1` version this adapter serves.
///
/// Version 7 is the newest upstream has, and every version below it is a subset: the
/// requests a client binds below its ceiling are simply never sent, so serving 7 serves
/// 1 through 7.
pub const SHELL_VERSION: u32 = 7;

/// The redirect, as a session is given it.
///
/// It is one value rather than a function so the composition root decides *whether* the
/// redirect exists at all: a session with no `Redirect` in its extension list advertises
/// no `gtk_shell1`, and GTK then falls back to its own titlebar handling.
#[derive(Debug)]
pub struct Redirect {
    admission: Arc<Admission>,
}

impl Redirect {
    /// A redirect that screens every request with `admission`.
    #[must_use]
    pub fn new(admission: Admission) -> Self {
        Self {
            admission: Arc::new(admission),
        }
    }

    /// A redirect for one configured mode, which is what a session's configuration says.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest cannot be read,
    /// which is a build-time bug rather than a run-time condition.
    pub fn for_mode(mode: Mode) -> Result<Self, gnoblin_compat_core::MatrixError> {
        Admission::load(mode).map(Self::new)
    }

    /// Creates the global on a display whose state is `S`.
    ///
    /// The state is a parameter so a test can hold the same global in its own state type
    /// and drive the production handler; a session passes [`gnoblin_wayland::State`],
    /// which is what [`Extension::register`] does.
    pub fn install<S>(&self, display: &DisplayHandle) -> Result<(), ExtensionError>
    where
        S: wayland_server::GlobalDispatch<gtk_shell1::GtkShell1, ShellData> + 'static,
    {
        let data = ShellData::new(Arc::clone(&self.admission));
        display.create_global::<S, gtk_shell1::GtkShell1, ShellData>(SHELL_VERSION, data);
        Ok(())
    }
}

impl Extension for Redirect {
    fn name(&self) -> &'static str {
        crate::compat::NAME
    }

    fn register(&self, display: &DisplayHandle) -> Result<(), ExtensionError> {
        self.install::<State>(display)
    }
}

/// The global's data: what every bound `gtk_shell1` starts from.
#[derive(Debug)]
pub struct ShellData {
    admission: Arc<Admission>,
}

impl ShellData {
    /// The global's data for one session's admission.
    #[must_use]
    pub fn new(admission: Arc<Admission>) -> Self {
        Self { admission }
    }

    /// The admission every bound shell is given.
    #[must_use]
    pub fn admission(&self) -> &Arc<Admission> {
        &self.admission
    }
}

/// One bound `gtk_shell1`: the admission its requests are screened with.
#[derive(Debug)]
pub struct ShellUserData {
    admission: Arc<Admission>,
}

impl ShellUserData {
    /// One bound shell's data.
    #[must_use]
    pub fn new(admission: Arc<Admission>) -> Self {
        Self { admission }
    }

    /// The admission a request on this object passes.
    #[must_use]
    pub fn admission(&self) -> &Arc<Admission> {
        &self.admission
    }
}

/// One `gtk_surface1`: the surface it was created for.
#[derive(Debug)]
pub struct SurfaceData {
    surface: WlSurface,
    admission: Arc<Admission>,
}

impl SurfaceData {
    /// The data of one created `gtk_surface1`.
    #[must_use]
    pub fn new(surface: WlSurface, admission: Arc<Admission>) -> Self {
        Self { surface, admission }
    }

    /// The `wl_surface` this object was created for.
    #[must_use]
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }
}

impl GlobalDispatch2<gtk_shell1::GtkShell1, State> for ShellData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<gtk_shell1::GtkShell1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let shell = data_init.init(resource, ShellUserData::new(Arc::clone(&self.admission)));
        // No global app menu and no global menu bar: this desktop draws neither, and a
        // client that is told otherwise exports an app menu nobody shows.
        shell.capabilities(0);
        log::debug!(client = ?client.id(), "a client bound the redirect");
    }
}

impl Dispatch2<gtk_shell1::GtkShell1, State> for ShellUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _shell: &gtk_shell1::GtkShell1,
        request: gtk_shell1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        handle_shell_request(state, self.admission(), request, data_init);
    }
}

/// One `gtk_shell1` request, mapped and performed against `host`.
///
/// Generic over the host so a test can drive exactly this code with its own state type:
/// the transport is the same `DataInit`, and the only thing a test replaces is the
/// compositor.
pub fn handle_shell_request<H>(
    host: &mut H,
    admission: &Arc<Admission>,
    request: gtk_shell1::Request,
    data_init: &mut DataInit<'_, H>,
) where
    H: Shell1Host + wayland_server::Dispatch<gtk_surface1::GtkSurface1, SurfaceData>,
{
    let state = host;
    match request {
        gtk_shell1::Request::GetGtkSurface { gtk_surface, surface } => {
            // The object is created even when the adapter is off. A request that creates
            // an object and then answers with nothing leaves the transport holding an
            // object with no data, which aborts the whole session rather than refusing
            // one call; what the gate refuses are the requests *on* it, and it carries
            // the same admission either way.
            let _ = admit(admission, Call::GetGtkSurface);
            data_init.init(gtk_surface, SurfaceData::new(surface, Arc::clone(admission)));
        }
        gtk_shell1::Request::SetStartupId { startup_id } => {
            if !admit(admission, Call::SetStartupId) {
                return;
            }
            // There is no startup-notification sequence table yet, so the id is recorded
            // in the log and completes nothing. The support matrix says so rather than
            // letting a client believe its launch was tracked.
            log::debug!(?startup_id, "a client named its startup id");
        }
        gtk_shell1::Request::SystemBell { surface } => {
            if !admit(admission, Call::SystemBell) {
                return;
            }
            // `system_bell` names the *gtk surface* that rang, so the window comes from
            // that object's own data: the protocol's surface, and then the session's
            // window for it.
            let window = surface
                .as_ref()
                .and_then(|shell_surface| shell_surface.data::<SurfaceData>())
                .and_then(|data| match state.surface(data.surface()) {
                    Surface::Window(window) => Some(window),
                    Surface::Gone => None,
                });
            Shell1Host::bell(state, window);
        }
        gtk_shell1::Request::NotifyLaunch { startup_id } => {
            if !admit(admission, Call::NotifyLaunch) {
                return;
            }
            Shell1Host::launch(state, &startup_id);
        }
    }
}

impl Dispatch2<gtk_surface1::GtkSurface1, State> for SurfaceData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &gtk_surface1::GtkSurface1,
        request: gtk_surface1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        handle_surface_request(state, self, resource, request);
    }
}

/// One `gtk_surface1` request, mapped and performed against `host`.
///
/// Generic over the host for the same reason as [`handle_shell_request`]: the transport
/// is the same, and a test supplies its own compositor.
pub fn handle_surface_request<H: Shell1Host>(
    host: &mut H,
    data: &SurfaceData,
    resource: &gtk_surface1::GtkSurface1,
    request: gtk_surface1::Request,
) {
    let state = host;
    match request {
        gtk_surface1::Request::SetDbusProperties { application_id, .. } => {
            if !admit(&data.admission, Call::SetDbusProperties) {
                return;
            }
            if let Some(app_id) = application_id {
                match Shell1Host::set_app_id(state, data.surface(), &app_id) {
                    Surface::Window(window) => log::debug!(%window, "an application id"),
                    Surface::Gone => log::debug!("an application id for a window that is gone"),
                }
            }
        }
        gtk_surface1::Request::SetModal => {
            if !admit(&data.admission, Call::SetModal) {
                return;
            }
            // `gnoblin-wm` has no kind-change event, so the modality is recorded by the
            // protocol object and not by the window manager: the matrix says `partial`
            // and this is the line that keeps it visible.
            log::debug!("a window declared itself modal");
        }
        gtk_surface1::Request::UnsetModal => {
            if !admit(&data.admission, Call::UnsetModal) {
                return;
            }
            log::debug!("a window withdrew its modality");
        }
        gtk_surface1::Request::Present { time } => {
            if !admit(&data.admission, Call::Present) {
                return;
            }
            if let Surface::Window(window) = state.surface(data.surface()) {
                match Shell1Host::present(state, window, time) {
                    Activation::Focused => log::debug!(%window, "presented"),
                    Activation::Refused(reason) => {
                        log::debug!(%window, reason, "present refused")
                    }
                }
            }
        }
        gtk_surface1::Request::RequestFocus { startup_id } => {
            if !admit(&data.admission, Call::RequestFocus) {
                return;
            }
            if let Surface::Window(window) = state.surface(data.surface()) {
                match Shell1Host::request_focus(state, window, startup_id.as_deref()) {
                    Activation::Focused => log::debug!(%window, "focus request honoured"),
                    Activation::Refused(reason) => {
                        log::debug!(%window, reason, "focus request refused")
                    }
                }
            }
        }
        gtk_surface1::Request::TitlebarGesture { gesture, .. } => {
            if !admit(&data.admission, Call::TitlebarGesture) {
                return;
            }
            let actions: TitlebarActions = Shell1Host::titlebar_actions(state);
            // A gesture the protocol does not define keeps its number for the error, and
            // a defined one is its own discriminant: the generated enum and the mapping's
            // numbers are the protocol's, not two tables.
            let raw = match gesture {
                WEnum::Value(gesture) => gesture as u32,
                WEnum::Unknown(raw) => raw,
            };
            match (crate::mapping::resolve(raw, &actions), state.surface(data.surface())) {
                (Err(UnknownGesture { raw }), _) => {
                    // The protocol's own answer: the client sent a number its own enum
                    // does not define, so no action runs and the error names it.
                    resource.post_error(
                        gtk_surface1::Error::InvalidGesture,
                        format!("the gesture {raw} is not one this protocol defines"),
                    );
                }
                (Ok(action), Surface::Window(window)) => match Shell1Host::titlebar(state, window, action) {
                    Titlebar::Performed => {
                        log::debug!(%window, %action, "a titlebar action")
                    }
                    Titlebar::Unavailable(reason) => {
                        log::debug!(%window, %action, reason, "an action the engine lacks")
                    }
                },
                (Ok(_), Surface::Gone) => {
                    log::debug!("a gesture for a window that is gone")
                }
            }
        }
        gtk_surface1::Request::Release => {
            if !admit(&data.admission, Call::Release) {
                return;
            }
            // `release` is the protocol's destructor, so the library destroys the object
            // when this handler returns: nothing here has to, and the window it named is
            // untouched either way.
            debug_assert!(
                resource.is_alive(),
                "a destructor's object is destroyed after the handler"
            );
        }
        gtk_surface1::Request::SetA11yProperties { .. } => {
            // Declared `unsupported`: the matrix's reason is that the accessibility tree is
            // not published over AT-SPI yet, so nothing is recorded rather than half-linked.
            let _ = admit(&data.admission, Call::SetA11yProperties);
        }
    }
}

/// Screens one request with the frame, logging a refusal.
///
/// The answer is a `bool` rather than an error because a refused Wayland request has no
/// reply on the wire: the protocol's own failure mode is "nothing happened", and the
/// refusal's value is the log line that names the rule. Every call is a read
/// (`crate::compat` says why), so the lock embargo has no rule to apply here and the
/// state is not read from the session.
fn admit(admission: &Admission, call: Call) -> bool {
    let gate = Gate::new(admission.mode(), CallerContext::UNSANDBOXED, LockState::Unlocked);
    if let Err(refusal) = admission.admit(call, gate) {
        log::debug!(interface = gnoblin_compat_core::Call::interface(&call),
            member = gnoblin_compat_core::Call::member(&call),
            %refusal,
            "a request was refused"
        );
        return false;
    }
    true
}

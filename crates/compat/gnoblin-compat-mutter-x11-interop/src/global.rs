//! The global, its object and the dispatch that records a parent relation.
//!
//! [`Interop`] is the value a session is given. One `mutter_x11_interop` object per client
//! (`destroy` is a destructor) is all the protocol has, and its requests are screened with
//! the [`Admission`] and then resolved through [`gnoblin_wayland::X11InteropHost`]: the
//! Wayland side through the compositor's window table, the X11 side through
//! `gnoblin-xwayland`'s. What is recorded is a [`ParentRelations`] table, which is the
//! observable part of a protocol whose applying half needs a window-manager event this
//! release does not have.

use std::sync::{Arc, Mutex, PoisonError};

use gnoblin_log::debug;
use gnoblin_wayland::{Dispatch2, Extension, ExtensionError, GlobalDispatch2, State, Surface, X11InteropHost};
use wayland_server::{Client, DataInit, DisplayHandle, New};

use crate::compat::{Admission, Call};
use crate::protocol::mutter_x11_interop;
use crate::relations::ParentRelations;

/// The protocol's version, which is the only one upstream has.
pub const INTEROP_VERSION: u32 = 1;

/// The redirect, as a session is given it.
#[derive(Debug)]
pub struct Interop {
    admission: Arc<Admission>,
    /// The relations every bound object records into.
    ///
    /// The table belongs to the global rather than to one client, because a parent
    /// relation is a fact about two windows and not about the connection that stated it:
    /// a shell that reconnects reads the same relations.
    relations: Arc<Mutex<ParentRelations>>,
}

impl Interop {
    /// An interop object that screens every request with `admission`.
    #[must_use]
    pub fn new(admission: Admission) -> Self {
        Self {
            admission: Arc::new(admission),
            relations: Arc::new(Mutex::new(ParentRelations::new())),
        }
    }

    /// The relations recorded so far.
    #[must_use]
    pub fn relations(&self) -> Arc<Mutex<ParentRelations>> {
        Arc::clone(&self.relations)
    }

    /// Creates the global on a display whose state is `S`.
    ///
    /// The state is a parameter so a test can hold the same global in its own state type
    /// and drive the production handler; a session passes [`gnoblin_wayland::State`].
    pub fn install<S>(&self, display: &DisplayHandle) -> Result<(), ExtensionError>
    where
        S: wayland_server::GlobalDispatch<mutter_x11_interop::MutterX11Interop, InteropData> + 'static,
    {
        display.create_global::<S, mutter_x11_interop::MutterX11Interop, InteropData>(
            INTEROP_VERSION,
            InteropData::with_relations(Arc::clone(&self.admission), Arc::clone(&self.relations)),
        );
        Ok(())
    }
}

impl Extension for Interop {
    fn name(&self) -> &'static str {
        crate::compat::NAME
    }

    fn register(&self, display: &DisplayHandle) -> Result<(), ExtensionError> {
        self.install::<State>(display)
    }
}

/// The global's data and every bound object's data: one admissions and one table.
#[derive(Debug)]
pub struct InteropData {
    admission: Arc<Admission>,
    relations: Arc<Mutex<ParentRelations>>,
    /// The children *this object* recorded, so a connection that goes away takes its own
    /// relations with it and nobody else's.
    own: Arc<Mutex<Vec<gnoblin_types::WindowId>>>,
}

impl InteropData {
    /// The global's data for one admission.
    #[must_use]
    pub fn new(admission: Arc<Admission>) -> Self {
        Self {
            admission,
            relations: Arc::new(Mutex::new(ParentRelations::new())),
            own: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The global's data for one admission and a table the caller keeps a handle on.
    ///
    /// A test builds its own table and holds the other handle, which is how it reads what
    /// the dispatch recorded; a session uses [`InteropData::new`] and reads the table
    /// through [`InteropData::relations`] if it wants to.
    #[must_use]
    pub fn with_relations(admission: Arc<Admission>, relations: Arc<Mutex<ParentRelations>>) -> Self {
        Self {
            admission,
            relations,
            own: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The admission every request passes.
    #[must_use]
    pub fn admission(&self) -> &Arc<Admission> {
        &self.admission
    }

    /// The relations recorded so far.
    #[must_use]
    pub fn relations(&self) -> Arc<Mutex<ParentRelations>> {
        Arc::clone(&self.relations)
    }
}

impl GlobalDispatch2<mutter_x11_interop::MutterX11Interop, State> for InteropData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<mutter_x11_interop::MutterX11Interop>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, self.clone_data());
        debug!(client = ?client.id(), "[x11-interop] a client bound the redirect");
    }
}

impl InteropData {
    /// One object's data, sharing the global's admission and table.
    fn clone_data(&self) -> InteropData {
        InteropData {
            admission: Arc::clone(&self.admission),
            relations: Arc::clone(&self.relations),
            own: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ParentRelations> {
        self.relations.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Dispatch2<mutter_x11_interop::MutterX11Interop, State> for InteropData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _interop: &mutter_x11_interop::MutterX11Interop,
        request: mutter_x11_interop::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        handle_request(state, self, request);
    }

    fn destroyed(
        &self,
        _state: &mut State,
        _client: wayland_server::backend::ClientId,
        _interop: &mutter_x11_interop::MutterX11Interop,
    ) {
        // A connection that goes away takes its own relations with it: the children it
        // recorded are its windows, and a relation for a window nothing names any more is
        // a table entry that can never be used or removed.
        let own = self.own.lock().map(|own| own.clone()).unwrap_or_default();
        for child in own {
            self.lock().forget(child);
        }
    }
}

/// One request, mapped and performed against `host`.
///
/// Generic over the host so a test drives exactly this code with its own state type.
pub fn handle_request<H: X11InteropHost>(host: &mut H, data: &InteropData, request: mutter_x11_interop::Request) {
    match request {
        mutter_x11_interop::Request::Destroy => {
            let _ = data.admission.admit(Call::Destroy);
            // `destroy` is the protocol's destructor: the library destroys the object when
            // this handler returns.
        }
        mutter_x11_interop::Request::SetX11Parent { surface, xwindow } => {
            if data.admission.admit(Call::SetX11Parent).is_err() {
                return;
            }
            let Surface::Window(child) = host.surface(&surface) else {
                debug!("[x11-interop] the child surface is not a window of this session");
                return;
            };
            let Some(parent) = host.window_for_x11(xwindow) else {
                debug!(xwindow, "[x11-interop] the X11 window is not managed by this session");
                return;
            };
            data.lock().record(child, parent);
            if let Ok(mut own) = data.own.lock()
                && !own.contains(&child)
            {
                own.push(child);
            }
            debug!(%child, %parent, xwindow, "[x11-interop] a parent relation was recorded");
        }
    }
}

//! [`Client`]: a window the compositor manages, with the desktop's conveniences on top of the core's [`Window`].
//!
//! A `Client` dereferences to the core's [`Window`] (and so to its [`Node`](gnoblin_core::Node)), which has the properties and the
//! methods the core schema gives a window: `title`, `app_id`, `geometry`, `is_focused`, `close`, `kill`, `focus`, `raise`, `lower`,
//! `set(..)`. What `Client` adds is what the std library knows: the workspace the client is on, whether it floats, who its parent is,
//! and typed access to the frame and presentation policy members.

#![allow(unsafe_code)]

use gnoblin_core::{ErrorKind, NodeId, Rect, Window};

use crate::api::Std;
use crate::collection::{Collection, Member};
use crate::error::{Error, Result};
use crate::query::Query;
use crate::subscription::Subscription;
use crate::workspace::Workspace;
use gnoblin_std_abi as abi;

/// How the compositor answers a window's frame callbacks: see [`Client::set_frame_policy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePolicy {
    /// After every frame the compositor draws that shows the window.
    Auto,
    /// Never: the client waits and draws nothing new, which is what a shell wants for a window it does not show.
    Hold,
    /// At most [`Client::frame_rate`] times a second.
    Throttle,
}

impl FramePolicy {
    fn text(self) -> &'static str {
        match self {
            FramePolicy::Auto => "auto",
            FramePolicy::Hold => "hold",
            FramePolicy::Throttle => "throttle",
        }
    }
}

/// A window of the compositor. Cheap to clone and compare (by window id).
#[derive(Clone)]
pub struct Client {
    std: Std,
    window: Window,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Client").field(&self.window.id().0).finish()
    }
}

impl PartialEq for Client {
    fn eq(&self, other: &Client) -> bool {
        self.window.id() == other.window.id()
    }
}

impl Eq for Client {}

impl std::ops::Deref for Client {
    type Target = Window;

    fn deref(&self) -> &Window {
        &self.window
    }
}

/// Anything that names a client: a [`Client`] or the core's [`Window`], by value or by reference. The methods that take a client take
/// `impl AsClient`, so `g.on_client_created(|window| workspace.add(window))` works with the core's callback type.
pub trait AsClient {
    /// The window id of the client.
    fn client_id(&self) -> NodeId;
}

impl AsClient for Client {
    fn client_id(&self) -> NodeId {
        self.window.id()
    }
}

impl AsClient for Window {
    fn client_id(&self) -> NodeId {
        self.id()
    }
}

impl<T: AsClient + ?Sized> AsClient for &T {
    fn client_id(&self) -> NodeId {
        (**self).client_id()
    }
}

impl Client {
    /// The client of `window`.
    pub fn new(std: &Std, window: Window) -> Client {
        Client {
            std: std.clone(),
            window,
        }
    }

    /// The client with window id `id`, without checking that the window exists (a window that just closed is the one an event names).
    pub(crate) fn from_raw_id(std: &Std, id: u64) -> Option<Client> {
        Some(Client {
            std: std.clone(),
            window: std.gnoblin().window_handle(NodeId(id)),
        })
    }

    /// The core window this client is.
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// The std handle of the generation this client belongs to.
    pub fn std(&self) -> &Std {
        &self.std
    }

    /// The window's rectangle, in logical pixels.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn bounds(&self) -> Result<Rect> {
        Ok(self.window.geometry()?)
    }

    /// Places and sizes the window.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn set_bounds(&self, rect: Rect) -> Result<()> {
        Ok(self.window.set_geometry(rect)?)
    }

    /// Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn is_tiled(&self) -> Result<bool> {
        Ok(self.window.get_bool("tiled")?)
    }

    /// Whether the window is drawn and hit. A workspace that is not shown hides its windows.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn is_visible(&self) -> Result<bool> {
        Ok(self.window.get_bool("visible")?)
    }

    /// How the compositor answers this window's client when it asks to be told it may draw its next frame.
    ///
    /// # Errors
    ///
    /// The window is gone, or the compositor sent a policy this SDK does not know.
    pub fn frame_policy(&self) -> Result<FramePolicy> {
        match self.window.get_text("frame_policy")?.as_str() {
            "auto" => Ok(FramePolicy::Auto),
            "hold" => Ok(FramePolicy::Hold),
            "throttle" => Ok(FramePolicy::Throttle),
            other => Err(Error::new(ErrorKind::Type, format!("unknown frame policy `{other}`"))),
        }
    }

    /// Sets how the window's client is answered. It takes effect only while a shell holds the tree, and the compositor sets every
    /// window back to `auto` when the shell lets go of it.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn set_frame_policy(&self, policy: FramePolicy) -> Result<()> {
        Ok(self.window.set("frame_policy", policy.text())?)
    }

    /// The most frame callbacks a second a throttled window's client is answered, in hertz.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn frame_rate(&self) -> Result<f64> {
        Ok(self.window.get_f64("frame_rate")?)
    }

    /// Sets the rate a throttled window's client is answered at, in hertz (above 0 and at most 1000).
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn set_frame_rate(&self, hertz: f64) -> Result<()> {
        Ok(self.window.set("frame_rate", hertz)?)
    }

    /// Asks the compositor to report, or stop reporting, when the window's pixels reach the screen.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn set_report_presented(&self, report: bool) -> Result<()> {
        Ok(self.window.set("report_presented", report)?)
    }

    /// Who draws the window's frame, as the compositor negotiated it: `server`, `client` or `none`.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn decoration(&self) -> Result<String> {
        Ok(self.window.get_text("decoration")?)
    }

    /// Whether the window opened on an activation token the session's launcher minted.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn from_launcher(&self) -> Result<bool> {
        Ok(self.window.get_bool("from_launcher")?)
    }

    /// The window this one is a dialog of, if it is one. A dialog is centred in its parent when it opens, moves with it, stays on its
    /// workspace and is left out of a layout's tiling.
    ///
    /// # Errors
    ///
    /// The window is gone, or the generation retired.
    pub fn parent_window(&self) -> Result<Option<Client>> {
        Ok(self
            .window
            .get_node("parent_window")?
            .map(|node| Client::new(&self.std, self.std.gnoblin().window_handle(node.id()))))
    }

    /// The workspace the client is on, or `None` while it is on none.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn workspace(&self) -> Result<Option<Workspace>> {
        let mut workspace = 0;
        // SAFETY: `client_workspace` writes one handle through the pointer it is given, a live local.
        self.std
            .check(unsafe { (self.std.api().client_workspace)(self.std.ctx(), self.window.id().0, &mut workspace) })?;
        Ok((workspace != 0).then(|| Workspace::from_handle(&self.std, workspace)))
    }

    /// Moves the client to `workspace`: `true` when it moved, `false` when it was refused (the client is fullscreen, a dialog, already
    /// there, or the workspace is not in the list).
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn move_to_workspace(&self, workspace: &Workspace) -> Result<bool> {
        // SAFETY: `client_move_to_workspace` takes the context, a window id and a workspace handle.
        let moved = self.std.check(unsafe {
            (self.std.api().client_move_to_workspace)(self.std.ctx(), self.window.id().0, workspace.raw())
        })?;
        Ok(moved == 1)
    }

    /// Whether the shell marked the client floating: a layout leaves it where it is.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn is_floating(&self) -> Result<bool> {
        // SAFETY: `client_is_floating` takes the context and a window id.
        let floating = self
            .std
            .check(unsafe { (self.std.api().client_is_floating)(self.std.ctx(), self.window.id().0) })?;
        Ok(floating == 1)
    }

    /// Marks the client floating, or tiled again: a layout leaves a floating client where it is.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn set_floating(&self, floating: bool) -> Result<()> {
        // SAFETY: `client_set_floating` takes the context, a window id and a flag.
        self.std
            .check(unsafe {
                (self.std.api().client_set_floating)(self.std.ctx(), self.window.id().0, u32::from(floating))
            })
            .map(|_| ())
    }

    /// A live selection that holds only this client, which the `on_*` methods of a client listen through.
    fn alone(&self) -> Result<Collection<Client>> {
        Collection::<Client>::root(&self.std)?.where_(&Query::new().is("id", self.window.id().0.to_string()))
    }

    /// Calls `f` with this client's new title when it changes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_title_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, &str) + 'static,
    {
        self.alone()?.on_title_changed(move |client, title| f(client, title))
    }

    /// Calls `f` with `true` when this client gains the keyboard and `false` when it loses it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_focus_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, bool) + 'static,
    {
        self.alone()?
            .on_focus_changed(move |client, focused| f(client, focused))
    }

    /// Calls `f` when this client's window closes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_closed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client) + 'static,
    {
        self.alone()?.on_closed(move |client| f(client))
    }
}

impl Member for Client {
    const KIND: u32 = abi::STD_COLLECTION_CLIENTS;

    fn from_raw(std: &Std, raw: u64) -> Option<Client> {
        Client::from_raw_id(std, raw)
    }

    fn raw(&self) -> u64 {
        self.window.id().0
    }

    fn key(&self) -> String {
        self.window.id().0.to_string()
    }
}

//! [`Collection`]: a live view of clients, outputs or workspaces.
//!
//! A collection is a handle on a list the library keeps and updates: `std.clients()` is every window, a workspace's `clients()` are
//! the windows on it, and [`where_`](Collection::where_) narrows one to the members whose properties match a [`Query`] and follows it as
//! they change. Reading is a snapshot (`to_vec`, `iter`, `filter`, `find`, `sorted_by`); listening is a closure or a listener object
//! (`on_added`, `on_title_changed`, [`ClientListener`]); both are told from the next turn of the shell's dispatch, never inside the
//! call that made the change.
//!
//! This is `data/js/gnoblin-core/host/collections.ts` in Rust idiom: `where` is `where_`, the callbacks `onTitleChanged` are
//! `on_title_changed`, and a `ClientListener` class is a trait with a default no-op method for each event.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core_abi as core_abi;
use gnoblin_std_abi::{self as abi, GnoblinStdCollection, GnoblinStdEvent, GnoblinStdMembers, GnoblinStdQueryTerm};

use crate::api::Std;
use crate::client::Client;
use crate::error::Result;
use crate::output::Output;
use crate::query::{Query, QueryValue};
use crate::subscription::{ListenerHandle, Subscription};
use crate::workspace::Workspace;

/// What a collection holds: a [`Client`], an [`Output`] or a [`Workspace`].
pub trait Member: Clone + 'static {
    /// The kind of collection of these (`STD_COLLECTION_*`).
    #[doc(hidden)]
    const KIND: u32;

    /// The member the library's id `raw` names.
    #[doc(hidden)]
    fn from_raw(std: &Std, raw: u64) -> Option<Self>;

    /// The library's id of the member.
    #[doc(hidden)]
    fn raw(&self) -> u64;

    /// The member's `id` as the `get` of a collection finds it: a window's or an output's node id, a workspace's own id.
    fn key(&self) -> String;
}

/// A collection event as the library delivered it.
pub(crate) struct RawEvent<'a> {
    pub(crate) flag: bool,
    pub(crate) member: u64,
    pub(crate) text: &'a str,
    pub(crate) rect: Rect,
}

/// A closure behind a `user` pointer, with the handle of the generation it belongs to.
struct Slot<F> {
    f: F,
    std: Std,
}

unsafe extern "C" fn event_tramp<F: FnMut(&Std, &RawEvent<'_>)>(
    user: *mut c_void,
    event: *const GnoblinStdEvent,
) -> i32 {
    // SAFETY: `user` is the `Box<Slot<F>>` leaked by `listen`. The library passes it back unchanged, on the shell thread, and does not
    // free it before `drop_slot` runs, so it is live and not aliased during this call.
    let slot = unsafe { &mut *user.cast::<Slot<F>>() };
    // SAFETY: the library passes a pointer that is valid until this callback returns, or null.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return core_abi::E_ERR;
    };
    let text = if event.text.ptr.is_null() || event.text.len == 0 {
        ""
    } else {
        // SAFETY: the library lends `len` bytes at `ptr` until the callback returns.
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(event.text.ptr, event.text.len) }).unwrap_or("")
    };
    let raw = RawEvent {
        flag: event.flag != 0,
        member: event.member,
        text,
        rect: Rect::new(event.rect.x, event.rect.y, event.rect.width, event.rect.height),
    };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(&slot.std, &raw))) {
        Ok(()) => core_abi::E_OK,
        Err(_) => core_abi::E_PANIC,
    }
}

/// Frees the box behind a `user` pointer. The library calls it exactly once per registration.
pub(crate) unsafe extern "C" fn drop_slot<T>(user: *mut c_void) {
    // SAFETY: `user` is the `Box<T>` leaked at registration and the library calls this exactly once.
    drop(unsafe { Box::from_raw(user.cast::<T>()) });
}

/// A live collection of `T`. Cheap to clone and compare; it belongs to the shell thread like the handle it was made from.
pub struct Collection<T: Member> {
    std: Std,
    id: GnoblinStdCollection,
    _members: PhantomData<fn() -> T>,
}

impl<T: Member> Clone for Collection<T> {
    fn clone(&self) -> Collection<T> {
        Collection {
            std: self.std.clone(),
            id: self.id,
            _members: PhantomData,
        }
    }
}

impl<T: Member> PartialEq for Collection<T> {
    fn eq(&self, other: &Collection<T>) -> bool {
        self.id == other.id && self.std == other.std
    }
}

impl<T: Member> std::fmt::Debug for Collection<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collection").field("id", &self.id).finish()
    }
}

impl<T: Member> Collection<T> {
    pub(crate) fn from_raw(std: Std, id: GnoblinStdCollection) -> Collection<T> {
        Collection {
            std,
            id,
            _members: PhantomData,
        }
    }

    /// The root collection of a kind of member.
    pub(crate) fn root(std: &Std) -> Result<Collection<T>> {
        let mut id = 0;
        // SAFETY: `collection_root` writes one handle through the pointer it is given, a live local.
        std.check(unsafe { (std.api().collection_root)(std.ctx(), T::KIND, &mut id) })?;
        Ok(Collection::from_raw(std.clone(), id))
    }

    pub(crate) fn raw_id(&self) -> GnoblinStdCollection {
        self.id
    }

    pub(crate) fn std_ref(&self) -> &Std {
        &self.std
    }

    /// The number of members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn len(&self) -> Result<usize> {
        let mut len = 0usize;
        // SAFETY: `collection_len` writes one `usize` through the pointer it is given, a live local.
        self.std
            .check(unsafe { (self.std.api().collection_len)(self.std.ctx(), self.id, &mut len) })?;
        Ok(len)
    }

    /// Whether there are no members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn is_empty(&self) -> Result<bool> {
        self.len().map(|len| len == 0)
    }

    /// The library's ids of the members, in order.
    fn raw_members(&self) -> Result<Vec<u64>> {
        let mut members = GnoblinStdMembers {
            ptr: core::ptr::null(),
            len: 0,
        };
        // SAFETY: `collection_members` fills `members`, a live local; the array it points at is copied before the next call.
        self.std
            .check(unsafe { (self.std.api().collection_members)(self.std.ctx(), self.id, &mut members) })?;
        if members.ptr.is_null() || members.len == 0 {
            return Ok(Vec::new());
        }
        // SAFETY: the library lends `len` ids at `ptr` until the next call into it on this thread.
        Ok(unsafe { std::slice::from_raw_parts(members.ptr, members.len) }.to_vec())
    }

    /// A snapshot of the members, in order. It does not follow later changes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn to_vec(&self) -> Result<Vec<T>> {
        Ok(self
            .raw_members()?
            .into_iter()
            .filter_map(|raw| T::from_raw(&self.std, raw))
            .collect())
    }

    /// The member at `index`, or `None` past the end.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn at(&self, index: usize) -> Result<Option<T>> {
        Ok(self.to_vec()?.into_iter().nth(index))
    }

    /// Whether `member` is in the collection now.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn contains(&self, member: &T) -> Result<bool> {
        // SAFETY: `collection_contains` takes the context, the collection and a member id.
        let held = self
            .std
            .check(unsafe { (self.std.api().collection_contains)(self.std.ctx(), self.id, member.raw()) })?;
        Ok(held == 1)
    }

    /// The first member whose `id` is `id` (see [`Member::key`]).
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn get(&self, id: &str) -> Result<Option<T>> {
        Ok(self.to_vec()?.into_iter().find(|member| member.key() == id))
    }

    /// The first member `predicate` accepts.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn find(&self, mut predicate: impl FnMut(&T) -> bool) -> Result<Option<T>> {
        Ok(self.to_vec()?.into_iter().find(|member| predicate(member)))
    }

    /// The members `predicate` accepts: a snapshot, which does not follow later changes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn filter(&self, mut predicate: impl FnMut(&T) -> bool) -> Result<Vec<T>> {
        Ok(self.to_vec()?.into_iter().filter(|member| predicate(member)).collect())
    }

    /// The members in the order `compare` gives: a snapshot. The collection's own order is not changed.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn sorted_by(&self, compare: impl FnMut(&T, &T) -> Ordering) -> Result<Vec<T>> {
        let mut members = self.to_vec()?;
        members.sort_by(compare);
        Ok(members)
    }

    /// A snapshot as the iterator of its members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn iter(&self) -> Result<std::vec::IntoIter<T>> {
        Ok(self.to_vec()?.into_iter())
    }

    /// A live selection of this collection: the members whose properties match `query`, following them as they change. A property
    /// that kind of member does not have is an error that names the ones it does.
    ///
    /// # Errors
    ///
    /// The query names a property the members do not have, or the generation retired.
    pub fn where_(&self, query: &Query) -> Result<Collection<T>> {
        // The strings the terms point at stay alive in `owned` until the call returns.
        let owned: Vec<(String, QueryValue)> = query.terms().to_vec();
        let terms: Vec<GnoblinStdQueryTerm> = owned
            .iter()
            .map(|(key, value)| {
                let (kind, flag, text, number) = match value {
                    QueryValue::Text(text) => (abi::STD_TERM_TEXT, 0, text.as_str(), 0.0),
                    QueryValue::Number(number) => (abi::STD_TERM_NUMBER, 0, "", *number),
                    QueryValue::Bool(flag) => (abi::STD_TERM_BOOL, u32::from(*flag), "", 0.0),
                };
                GnoblinStdQueryTerm {
                    size: size_of::<GnoblinStdQueryTerm>() as u32,
                    kind,
                    flag,
                    reserved: 0,
                    key: core_abi::GnoblinStr::borrowed(key),
                    text: core_abi::GnoblinStr::borrowed(text),
                    number,
                }
            })
            .collect();
        let mut id = 0;
        // SAFETY: `terms` and the strings they borrow from `owned` are live for the call; the library copies what it keeps.
        self.std.check(unsafe {
            (self.std.api().collection_where)(self.std.ctx(), self.id, terms.as_ptr(), terms.len(), &mut id)
        })?;
        Ok(Collection::from_raw(self.std.clone(), id))
    }

    /// Calls `f` for `event` of this collection, on the shell thread, from the next turn of the dispatch. With `replay` the listener is
    /// told of every member now, as if each had just been added.
    pub(crate) fn listen<F>(&self, event: u32, replay: bool, f: F) -> Result<Subscription>
    where
        F: FnMut(&Std, &RawEvent<'_>) + 'static,
    {
        let slot = Box::into_raw(Box::new(Slot {
            f,
            std: self.std.clone(),
        }));
        let flags = if replay { abi::STD_LISTEN_REPLAY } else { 0 };
        // SAFETY: `slot` is a leaked box the library hands back to `event_tramp::<F>` and `drop_slot::<Slot<F>>`; on failure the
        // library has not adopted it and it is reclaimed below.
        let reg = unsafe {
            (self.std.api().collection_listen)(
                self.std.ctx(),
                self.id,
                event,
                flags,
                event_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<Slot<F>>),
            )
        };
        if reg == 0 {
            // SAFETY: the library refused the registration, so it has not adopted the slot and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            return Err(self.std.failure());
        }
        Ok(Subscription::new(self.std.clone(), reg))
    }

    /// Calls `f` with each member that joins the collection.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_added<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.listen(abi::STD_EVENT_ADDED, false, move |std, event| {
            if let Some(member) = T::from_raw(std, event.member) {
                f(&member);
            }
        })
    }

    /// Calls `f` for every member now, and for each member that joins later: what a shell that reloads wants, so it sees the
    /// members that are already there. The members now are told with the next turn's events.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn watch<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.listen(abi::STD_EVENT_ADDED, true, move |std, event| {
            if let Some(member) = T::from_raw(std, event.member) {
                f(&member);
            }
        })
    }

    /// Calls `f` with each member that leaves the collection. A client that closed is already gone from the tree: reading it fails
    /// with `NoSuchNode`, and its id is what is left.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_removed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.listen(abi::STD_EVENT_REMOVED, false, move |std, event| {
            if let Some(member) = T::from_raw(std, event.member) {
                f(&member);
            }
        })
    }

    /// Registers a listener object for the membership of this collection.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn add_membership_listener(&self, listener: impl CollectionListener<T>) -> Result<ListenerHandle> {
        let listener = Rc::new(RefCell::new(listener));
        let added = Rc::clone(&listener);
        let first = self.on_added(move |member| added.borrow_mut().on_added(member))?;
        let removed = listener;
        let second = self.on_removed(move |member| removed.borrow_mut().on_removed(member))?;
        Ok(ListenerHandle::new(vec![first, second]))
    }
}

impl Collection<Client> {
    /// The window that has the keyboard, among the members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn focused(&self) -> Result<Option<Client>> {
        for client in self.to_vec()? {
            if client.is_focused().unwrap_or(false) {
                return Ok(Some(client));
            }
        }
        Ok(None)
    }

    fn on_client<F>(&self, event: u32, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, &RawEvent<'_>) + 'static,
    {
        self.listen(event, false, move |std, raw| {
            if let Some(client) = Client::from_raw_id(std, raw.member) {
                f(&client, raw);
            }
        })
    }

    /// Calls `f` with a member and its new title when a client changes it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_title_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, &str) + 'static,
    {
        self.on_client(abi::STD_EVENT_TITLE_CHANGED, move |client, raw| f(client, raw.text))
    }

    /// Calls `f` with a member and its new application id when a client changes it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_app_id_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, &str) + 'static,
    {
        self.on_client(abi::STD_EVENT_APP_ID_CHANGED, move |client, raw| f(client, raw.text))
    }

    /// Calls `f` with a member and `true` when it gains the keyboard, `false` when it loses it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_focus_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, bool) + 'static,
    {
        self.on_client(abi::STD_EVENT_FOCUS_CHANGED, move |client, raw| f(client, raw.flag))
    }

    /// Calls `f` with a member and its rectangle when its geometry changed. The writes of one turn are one event.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_geometry_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, Rect) + 'static,
    {
        self.on_client(abi::STD_EVENT_GEOMETRY_CHANGED, move |client, raw| f(client, raw.rect))
    }

    /// Calls `f` with a member and who draws its frame now (`client`, `server` or `none`).
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_decoration_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client, &str) + 'static,
    {
        self.on_client(abi::STD_EVENT_DECORATION_CHANGED, move |client, raw| {
            f(client, raw.text)
        })
    }

    /// Calls `f` with a member when its window closes. The window is already gone from reads; its id is what is left.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_closed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client) + 'static,
    {
        self.on_client(abi::STD_EVENT_CLOSED, move |client, _| f(client))
    }

    /// Calls `f` with each member that completed its initial setup: it has its final title, application id and size.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_ready<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Client) + 'static,
    {
        self.on_client(abi::STD_EVENT_READY, move |client, _| f(client))
    }

    /// Registers a listener object for the events of the clients this collection holds, now and later. Every method the listener
    /// does not override is a no-op.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn add_listener(&self, listener: impl ClientListener) -> Result<ListenerHandle> {
        let listener = Rc::new(RefCell::new(listener));
        let mut subscriptions = Vec::new();
        let l = Rc::clone(&listener);
        subscriptions.push(self.on_title_changed(move |client, title| l.borrow_mut().on_title_changed(client, title))?);
        let l = Rc::clone(&listener);
        subscriptions.push(self.on_app_id_changed(move |client, id| l.borrow_mut().on_app_id_changed(client, id))?);
        let l = Rc::clone(&listener);
        subscriptions
            .push(self.on_decoration_changed(move |client, who| l.borrow_mut().on_decoration_changed(client, who))?);
        let l = Rc::clone(&listener);
        subscriptions
            .push(self.on_focus_changed(move |client, focused| l.borrow_mut().on_focus_changed(client, focused))?);
        let l = Rc::clone(&listener);
        subscriptions
            .push(self.on_geometry_changed(move |client, rect| l.borrow_mut().on_geometry_changed(client, rect))?);
        let l = Rc::clone(&listener);
        subscriptions.push(self.on_closed(move |client| l.borrow_mut().on_closed(client))?);
        let l = listener;
        subscriptions.push(self.on_ready(move |client| l.borrow_mut().on_ready(client))?);
        Ok(ListenerHandle::new(subscriptions))
    }
}

impl Collection<Output> {
    /// Calls `f` with an output and its new mode (`WxH@Hz`) when it changes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_mode_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Output, &str) + 'static,
    {
        self.listen(abi::STD_EVENT_MODE_CHANGED, false, move |std, raw| {
            if let Some(output) = Output::from_raw(std, raw.member) {
                f(&output, raw.text);
            }
        })
    }

    /// Calls `f` with an output and its rectangle when its place or size in the compositor's coordinates changed.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_geometry_changed<F>(&self, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&Output, Rect) + 'static,
    {
        self.listen(abi::STD_EVENT_GEOMETRY_CHANGED, false, move |std, raw| {
            if let Some(output) = Output::from_raw(std, raw.member) {
                f(&output, raw.rect);
            }
        })
    }
}

impl Collection<Workspace> {
    /// Adds a workspace to the list (see [`Workspace::add_to_list`]).
    ///
    /// # Errors
    ///
    /// A workspace with the same id is in the list already.
    pub fn add(&self, workspace: &Workspace) -> Result<()> {
        workspace.add_to_list()
    }

    /// Removes a workspace from the list: `true` when it left, `false` when it was refused (the last workspace stays). Its windows
    /// move to the workspace before it; none is closed.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn remove(&self, workspace: &Workspace) -> Result<bool> {
        // SAFETY: `collection_remove` takes the context, the collection and a member id.
        let removed = self
            .std
            .check(unsafe { (self.std.api().collection_remove)(self.std.ctx(), self.id, workspace.raw()) })?;
        Ok(removed == 1)
    }
}

/// Observes the events of clients: register it on a collection with [`Collection::add_listener`], and it hears every member of it,
/// now and later. Every method is a no-op until the listener overrides it.
pub trait ClientListener: 'static {
    /// A client's title changed.
    fn on_title_changed(&mut self, _client: &Client, _title: &str) {}
    /// A client's application id changed.
    fn on_app_id_changed(&mut self, _client: &Client, _app_id: &str) {}
    /// The compositor changed who draws a client's frame: `client`, `server` or `none`.
    fn on_decoration_changed(&mut self, _client: &Client, _decoration: &str) {}
    /// A client gained or lost the keyboard.
    fn on_focus_changed(&mut self, _client: &Client, _focused: bool) {}
    /// A client's geometry changed.
    fn on_geometry_changed(&mut self, _client: &Client, _geometry: Rect) {}
    /// A client closed.
    fn on_closed(&mut self, _client: &Client) {}
    /// A client completed its initial setup.
    fn on_ready(&mut self, _client: &Client) {}
}

/// Observes membership: a member joining or leaving a collection. Register it with [`Collection::add_membership_listener`].
pub trait CollectionListener<T: Member>: 'static {
    /// A member joined the collection.
    fn on_added(&mut self, _member: &T) {}
    /// A member left the collection.
    fn on_removed(&mut self, _member: &T) {}
}

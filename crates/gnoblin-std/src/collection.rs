//! [`Collection`]: a live view of clients, outputs or workspaces.
//!
//! A collection is a handle on a list the library keeps and updates: `std.clients()` is every window, a workspace's `clients()` are
//! the windows on it, and [`where_`](Collection::where_) narrows one to the members whose properties match a [`Query`] and follows it as
//! they change. Reading is a snapshot (`to_vec`, `iter`, `filter`, `find`, `sorted_by`); listening is a closure or a listener object
//! (`on_added`, `on_title_changed`, [`ClientListener`]); both are told from the next turn of the shell's dispatch, never inside the
//! call that made the change.
//!
//! The clients of a collection are the class `client_collection` of `gnoblin.std`, the workspaces of the list are the class `workspaces`,
//! and the events of a client's own properties are the core's `client` model (`gnoblin.model`): one path serves every language. The
//! collection of outputs is still read through the library's function table.
//!
//! This is `data/js/gnoblin-core/host/collections.ts` in Rust idiom: `where` is `where_`, the callbacks `onTitleChanged` are
//! `on_title_changed`, and a `ClientListener` class is a trait with a default no-op method for each event.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core::objects as model;
use gnoblin_core_abi as core_abi;
use gnoblin_object::typed::Listener;
use gnoblin_std_abi::{self as abi, GnoblinStdCollection, GnoblinStdEvent, GnoblinStdMembers, GnoblinStdQueryTerm};

use crate::api::Std;
use crate::client::{AsClient, Client};
use crate::error::{Error, ErrorKind, Result};
use crate::objects as classes;
use crate::output::Output;
use crate::query::{Query, QueryValue};
use crate::subscription::{ListenerHandle, Subscription};
use crate::workspace::Workspace;

/// What a collection holds: a [`Client`], an [`Output`] or a [`Workspace`].
pub trait Member: Clone + 'static {
    /// The kind of collection of these (`STD_COLLECTION_*`), for the members that the library's function table serves.
    #[doc(hidden)]
    const KIND: u32 = 0;

    /// The member that `key` names (see [`key`](Member::key)).
    #[doc(hidden)]
    fn from_key(std: &Std, key: &str) -> Option<Self> {
        key.parse().ok().and_then(|raw| Self::from_raw(std, raw))
    }

    /// The member the library's number `raw` names.
    #[doc(hidden)]
    fn from_raw(std: &Std, raw: u64) -> Option<Self> {
        Self::from_key(std, &raw.to_string())
    }

    /// The library's number of the member.
    #[doc(hidden)]
    fn raw(&self) -> u64 {
        self.key().parse().unwrap_or(0)
    }

    /// The member's `id` as the `get` of a collection finds it: a window's or an output's node id, a workspace's own id.
    fn key(&self) -> String;
}

/// A collection event of the library's function table, as it delivered it.
pub(crate) struct RawEvent<'a> {
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

/// Where a collection of clients comes from, so the function table can name the same list when a feature of it still needs that.
#[derive(Clone)]
enum Origin {
    /// Every client.
    Root,
    /// The clients of a workspace, by its id.
    Workspace(String),
    /// A selection of another collection.
    Selection(Rc<Origin>, Query),
}

/// What backs a collection.
#[derive(Clone)]
enum Source {
    /// A class `client_collection`.
    Clients(Rc<classes::ClientCollection>, Origin),
    /// The list of workspaces, the class `workspaces`.
    Workspaces(Rc<classes::Workspaces>),
    /// A collection of the library's function table (outputs).
    Table(GnoblinStdCollection),
}

/// A live collection of `T`. Cheap to clone and compare; it belongs to the shell thread like the handle it was made from.
pub struct Collection<T: Member> {
    std: Std,
    source: Source,
    _members: PhantomData<fn() -> T>,
}

impl<T: Member> Clone for Collection<T> {
    fn clone(&self) -> Collection<T> {
        Collection {
            std: self.std.clone(),
            source: self.source.clone(),
            _members: PhantomData,
        }
    }
}

impl<T: Member> PartialEq for Collection<T> {
    fn eq(&self, other: &Collection<T>) -> bool {
        self.std == other.std
            && match (&self.source, &other.source) {
                (Source::Clients(a, _), Source::Clients(b, _)) => a.handle().handle() == b.handle().handle(),
                (Source::Workspaces(a), Source::Workspaces(b)) => a.handle().handle() == b.handle().handle(),
                (Source::Table(a), Source::Table(b)) => a == b,
                _ => false,
            }
    }
}

impl<T: Member> std::fmt::Debug for Collection<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut shown = f.debug_struct("Collection");
        match &self.source {
            Source::Clients(object, _) => shown.field("clients", &object.handle().handle()),
            Source::Workspaces(_) => shown.field("workspaces", &true),
            Source::Table(id) => shown.field("id", id),
        };
        shown.finish()
    }
}

/// The keys of the clients of `object`: the `id` of each core `Client` it lists, which the core keeps readable after the window closes.
fn client_keys(object: &classes::ClientCollection) -> Result<Vec<String>> {
    object.members()?.into_iter().map(|client| Ok(client.id()?)).collect()
}

fn kind_error(what: &str) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("{what} is not available on this kind of collection"),
    )
}

impl<T: Member> Collection<T> {
    /// The root collection of a kind of member.
    pub(crate) fn root(std: &Std) -> Result<Collection<T>> {
        let mut id = 0;
        // SAFETY: `collection_root` writes one handle through the pointer it is given, a live local.
        std.check(unsafe { (std.api().collection_root)(std.ctx(), T::KIND, &mut id) })?;
        Ok(Collection::from_table(std.clone(), id))
    }

    fn from_table(std: Std, id: GnoblinStdCollection) -> Collection<T> {
        Collection {
            std,
            source: Source::Table(id),
            _members: PhantomData,
        }
    }

    /// The function table's name for this collection, for the one feature that still takes it (the style of a collection of clients).
    pub(crate) fn raw_id(&self) -> Result<GnoblinStdCollection> {
        match &self.source {
            Source::Table(id) => Ok(*id),
            Source::Clients(_, origin) => table_clients(&self.std, origin),
            Source::Workspaces(_) => Err(kind_error("a table handle")),
        }
    }

    pub(crate) fn std_ref(&self) -> &Std {
        &self.std
    }

    /// The members' keys, in order.
    fn keys(&self) -> Result<Vec<String>> {
        match &self.source {
            Source::Clients(object, _) => client_keys(object),
            Source::Workspaces(list) => list
                .list()?
                .into_iter()
                .map(|workspace| Ok(workspace.id()?))
                .collect::<Result<Vec<_>>>(),
            Source::Table(id) => {
                let mut members = GnoblinStdMembers {
                    ptr: core::ptr::null(),
                    len: 0,
                };
                // SAFETY: `collection_members` fills `members`, a live local; the array it points at is copied before the next call.
                self.std
                    .check(unsafe { (self.std.api().collection_members)(self.std.ctx(), *id, &mut members) })?;
                if members.ptr.is_null() || members.len == 0 {
                    return Ok(Vec::new());
                }
                // SAFETY: the library lends `len` ids at `ptr` until the next call into it on this thread.
                Ok(unsafe { std::slice::from_raw_parts(members.ptr, members.len) }
                    .iter()
                    .map(u64::to_string)
                    .collect())
            }
        }
    }

    /// The number of members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn len(&self) -> Result<usize> {
        match &self.source {
            Source::Clients(object, _) => Ok(usize::try_from(object.length()?).unwrap_or(usize::MAX)),
            Source::Workspaces(list) => Ok(usize::try_from(list.length()?).unwrap_or(usize::MAX)),
            Source::Table(id) => {
                let mut len = 0usize;
                // SAFETY: `collection_len` writes one `usize` through the pointer it is given, a live local.
                self.std
                    .check(unsafe { (self.std.api().collection_len)(self.std.ctx(), *id, &mut len) })?;
                Ok(len)
            }
        }
    }

    /// Whether there are no members.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn is_empty(&self) -> Result<bool> {
        self.len().map(|len| len == 0)
    }

    /// A snapshot of the members, in order. It does not follow later changes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn to_vec(&self) -> Result<Vec<T>> {
        Ok(self
            .keys()?
            .iter()
            .filter_map(|key| T::from_key(&self.std, key))
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
        match &self.source {
            Source::Clients(object, _) => {
                let key = member.key();
                let node = key.parse::<u64>().map_err(|_| kind_error("a window's node number"))?;
                Ok(object.contains(&Std::current()?.core_client(gnoblin_core::NodeId(node))?)?)
            }
            Source::Workspaces(list) => {
                let key = member.key();
                Ok(list
                    .by_id(&key)?
                    .is_some_and(|workspace| workspace.attached().unwrap_or(false)))
            }
            Source::Table(id) => {
                // SAFETY: `collection_contains` takes the context, the collection and a member id.
                let held = self
                    .std
                    .check(unsafe { (self.std.api().collection_contains)(self.std.ctx(), *id, member.raw()) })?;
                Ok(held == 1)
            }
        }
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
        match &self.source {
            Source::Clients(object, origin) => {
                let selection = object.where_(&client_query(query)?)?;
                Ok(Collection {
                    std: self.std.clone(),
                    source: Source::Clients(
                        Rc::new(selection),
                        Origin::Selection(Rc::new(origin.clone()), query.clone()),
                    ),
                    _members: PhantomData,
                })
            }
            Source::Workspaces(_) => Err(kind_error("a selection")),
            Source::Table(id) => Ok(Collection::from_table(
                self.std.clone(),
                table_where(&self.std, *id, query)?,
            )),
        }
    }

    /// Calls `f` for `event` of this collection of the function table, on the shell thread, from the next turn of the dispatch.
    fn listen<F>(&self, id: GnoblinStdCollection, event: u32, replay: bool, f: F) -> Result<Subscription>
    where
        F: FnMut(&Std, &RawEvent<'_>) + 'static,
    {
        let slot = Box::into_raw(Box::new(Slot {
            f,
            std: self.std.clone(),
        }));
        // SAFETY: `slot` is a leaked box the library hands back to `event_tramp::<F>` and `drop_slot::<Slot<F>>`; on failure the
        // library has not adopted it and it is reclaimed below.
        let reg = unsafe {
            (self.std.api().collection_listen)(
                self.std.ctx(),
                id,
                event,
                if replay { abi::STD_LISTEN_REPLAY } else { 0 },
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
    pub fn on_added<F>(&self, f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.membership(false, false, f)
    }

    /// Calls `f` for every member now, and for each member that joins later: what a shell that reloads wants, so it sees the
    /// members that are already there. The members now are told with the next turn's events.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn watch<F>(&self, f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.membership(false, true, f)
    }

    /// Calls `f` with each member that leaves the collection. A client that closed is already gone from the tree: reading it fails
    /// with `NoSuchNode`, and its id is what is left.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn on_removed<F>(&self, f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        self.membership(true, false, f)
    }

    /// The listener of a member joining (`left` false) or leaving, with the members now told too when `replay`.
    fn membership<F>(&self, left: bool, replay: bool, mut f: F) -> Result<Subscription>
    where
        F: FnMut(&T) + 'static,
    {
        let std = self.std.clone();
        match &self.source {
            Source::Clients(object, _) => {
                // The class tells a new listener of the members that are there already; a listener that wants only what happens next
                // lets those pass.
                let skip: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(if replay || left {
                    HashSet::new()
                } else {
                    client_keys(object)?.into_iter().collect()
                }));
                let run = move |client: model::Client| {
                    let Ok(key) = client.id() else { return };
                    if !left && skip.borrow_mut().remove(&key) {
                        return;
                    }
                    if let Some(member) = T::from_key(&std, &key) {
                        f(&member);
                    }
                };
                let listener = if left {
                    object.on_removed(run)?
                } else {
                    object.on_added(run)?
                };
                Ok(keep_listeners(vec![listener], Rc::clone(object)))
            }
            Source::Workspaces(list) => {
                let f = Rc::new(RefCell::new(f));
                let told = Rc::clone(&f);
                let tell_std = std.clone();
                let run = move |workspace: classes::Workspace| {
                    if let Some(member) = workspace.id().ok().and_then(|key| T::from_key(&tell_std, &key)) {
                        let mut run = told.borrow_mut();
                        run(&member);
                    }
                };
                let listener = if left {
                    list.on_removed(run)?
                } else {
                    list.on_added(run)?
                };
                if replay {
                    // The list does not tell a new listener who is there: the members now are told from the next turn.
                    let (now, list) = (Rc::clone(&f), Rc::clone(list));
                    let again = std.clone();
                    std.gnoblin().after(0, move || {
                        for workspace in list.list().unwrap_or_default() {
                            if let Some(member) = workspace.id().ok().and_then(|key| T::from_key(&again, &key)) {
                                let mut run = now.borrow_mut();
                                run(&member);
                            }
                        }
                    })?;
                }
                Ok(keep_listeners(vec![listener], Rc::clone(list)))
            }
            Source::Table(id) => {
                let event = if left {
                    abi::STD_EVENT_REMOVED
                } else {
                    abi::STD_EVENT_ADDED
                };
                self.listen(*id, event, replay, move |std, event| {
                    if let Some(member) = T::from_raw(std, event.member) {
                        f(&member);
                    }
                })
            }
        }
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

/// Holds listeners of the object protocol for the life of the generation, with one subscription that ends them all.
fn keep_listeners(listeners: Vec<Listener>, held: impl std::any::Any) -> Subscription {
    Subscription::from_undo(move || {
        drop((listeners, held));
        Ok(())
    })
}

/// A `Query` as the fields of `client_query`.
fn client_query(query: &Query) -> Result<classes::ClientQuery> {
    let mut selection = classes::ClientQuery::default();
    let mut seen: Vec<&str> = Vec::new();
    for (key, value) in query.terms() {
        if seen.contains(&key.as_str()) {
            return Err(Error::new(
                ErrorKind::Type,
                format!("a selection of clients takes one value of `{key}`"),
            ));
        }
        seen.push(key);
        let wrong = || {
            Error::new(
                ErrorKind::Type,
                format!("`{key}` of a client takes another kind of value"),
            )
        };
        match (key.as_str(), value) {
            ("app_id", QueryValue::Text(text)) => selection.app_id = Some(text.clone()),
            ("title", QueryValue::Text(text)) => selection.title = Some(text.clone()),
            ("workspace", QueryValue::Text(text)) => selection.workspace = Some(text.clone()),
            ("focused", QueryValue::Bool(flag)) => selection.focused = Some(*flag),
            ("fullscreen", QueryValue::Bool(flag)) => selection.fullscreen = Some(*flag),
            ("maximised", QueryValue::Bool(flag)) => selection.maximised = Some(*flag),
            ("minimised", QueryValue::Bool(flag)) => selection.minimised = Some(*flag),
            ("app_id" | "title" | "workspace" | "focused" | "fullscreen" | "maximised" | "minimised", _) => {
                return Err(wrong());
            }
            (other, _) => {
                return Err(Error::new(
                    ErrorKind::NoSuchMember,
                    format!(
                        "a client has no property `{other}` to select on: it has app_id, title, workspace, focused, fullscreen, maximised and minimised"
                    ),
                ));
            }
        }
    }
    Ok(selection)
}

/// A selection of the function table's collection `id`.
fn table_where(std: &Std, id: GnoblinStdCollection, query: &Query) -> Result<GnoblinStdCollection> {
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
    let mut selection = 0;
    // SAFETY: `terms` and the strings they borrow from `owned` are live for the call; the library copies what it keeps.
    std.check(unsafe { (std.api().collection_where)(std.ctx(), id, terms.as_ptr(), terms.len(), &mut selection) })?;
    Ok(selection)
}

/// The function table's collection of clients that `origin` names.
fn table_clients(std: &Std, origin: &Origin) -> Result<GnoblinStdCollection> {
    match origin {
        Origin::Root => {
            let mut id = 0;
            // SAFETY: `collection_root` writes one handle through the pointer it is given, a live local.
            std.check(unsafe { (std.api().collection_root)(std.ctx(), abi::STD_COLLECTION_CLIENTS, &mut id) })?;
            Ok(id)
        }
        Origin::Workspace(name) => {
            let mut list = 0;
            // SAFETY: `collection_root` writes one handle through the pointer it is given, a live local.
            std.check(unsafe { (std.api().collection_root)(std.ctx(), abi::STD_COLLECTION_WORKSPACES, &mut list) })?;
            let mut members = GnoblinStdMembers {
                ptr: core::ptr::null(),
                len: 0,
            };
            // SAFETY: `collection_members` fills `members`, a live local; the array it points at is copied before the next call.
            std.check(unsafe { (std.api().collection_members)(std.ctx(), list, &mut members) })?;
            let handles: Vec<u64> = if members.ptr.is_null() {
                Vec::new()
            } else {
                // SAFETY: the library lends `len` ids at `ptr` until the next call into it on this thread.
                unsafe { std::slice::from_raw_parts(members.ptr, members.len) }.to_vec()
            };
            for handle in handles {
                let mut info = abi::GnoblinStdWorkspaceInfo {
                    size: size_of::<abi::GnoblinStdWorkspaceInfo>() as u32,
                    attached: 0,
                    id: core_abi::GnoblinStr::EMPTY,
                    name: core_abi::GnoblinStr::EMPTY,
                    index: -1,
                    active: 0,
                    reserved: 0,
                    output: 0,
                    gap_inner: 0.0,
                    gap_outer: 0.0,
                };
                // SAFETY: `workspace_info` fills `info` up to the `size` set; the strings it lends are used before the next call.
                std.check(unsafe { (std.api().workspace_info)(std.ctx(), handle, &mut info) })?;
                let same = !info.id.ptr.is_null()
                    // SAFETY: the library lends `len` bytes at `ptr` until the next call into it on this thread.
                    && unsafe { std::slice::from_raw_parts(info.id.ptr, info.id.len) } == name.as_bytes();
                if same {
                    let mut clients = 0;
                    // SAFETY: `workspace_clients` writes one handle through the pointer it is given, a live local.
                    std.check(unsafe { (std.api().workspace_clients)(std.ctx(), handle, &mut clients) })?;
                    return Ok(clients);
                }
            }
            Err(Error::new(ErrorKind::NoSuchNode, format!("no workspace `{name}`")))
        }
        Origin::Selection(parent, query) => table_where(std, table_clients(std, parent)?, query),
    }
}

// ---- clients ------------------------------------------------------------------------------------------------------------------

/// The handlers of the core's `client_collection_listener`, none set.
fn no_handlers() -> model::ClientCollectionListenerHandlers {
    model::ClientCollectionListenerHandlers {
        on_added: None,
        on_removed: None,
        on_title_changed: None,
        on_app_id_changed: None,
        on_decoration_changed: None,
        on_pointer_enter: None,
        on_pointer_motion: None,
        on_pointer_leave: None,
        on_pointer_press: None,
        on_pointer_release: None,
        on_focus_changed: None,
        on_geometry_changed: None,
        on_closed: None,
        on_ready: None,
    }
}

/// Who is in a collection of clients as far as the events of a client are concerned: the members now, and the ones that just left
/// (the core tells a client's own listeners first, so a window that closed may have left the collection already).
#[derive(Default)]
struct Here {
    members: HashSet<String>,
    gone: HashSet<String>,
}

/// Which clients an event is for.
pub(crate) type Filter = Rc<dyn Fn(&str, bool) -> bool>;

/// The id of a core client, as the std classes name it.
fn id_of(client: &model::Client) -> Option<String> {
    client.id().ok()
}

/// Listens to the events of the clients `filter` accepts on the core's collection of every client. `install` puts the handler of one event
/// into the set; `extra` are listeners of the object protocol that have to last as long as the subscription.
fn listen_clients(
    std: &Std,
    filter: Filter,
    extra: Box<dyn std::any::Any>,
    install: impl FnOnce(&mut model::ClientCollectionListenerHandlers, Filter, Std),
) -> Result<Subscription> {
    let objects = std.gnoblin().objects()?;
    let root = model::ModelRoot::get(&objects)?
        .clients()?
        .ok_or_else(|| Error::new(ErrorKind::Failed, "the core has no collection of clients"))?;
    let mut handlers = no_handlers();
    install(&mut handlers, filter, std.clone());
    let listener = handlers.implement(&objects)?;
    root.add_listener(&listener)?;
    Ok(Subscription::from_undo(move || {
        let result = root.remove_listener(&listener);
        drop(extra);
        Ok(result?)
    }))
}

impl Collection<Client> {
    /// The clients of the workspace `id`, which the library keeps as a collection.
    pub(crate) fn of_workspace(std: &Std, id: &str, clients: classes::ClientCollection) -> Collection<Client> {
        Collection {
            std: std.clone(),
            source: Source::Clients(Rc::new(clients), Origin::Workspace(id.to_owned())),
            _members: PhantomData,
        }
    }

    /// Every client.
    pub(crate) fn all(std: &Std) -> Result<Collection<Client>> {
        let objects = std.objects()?;
        Ok(Collection {
            std: std.clone(),
            source: Source::Clients(Rc::new(classes::ClientCollection::all(&objects)?), Origin::Root),
            _members: PhantomData,
        })
    }

    fn clients(&self) -> Result<&Rc<classes::ClientCollection>> {
        match &self.source {
            Source::Clients(object, _) => Ok(object),
            _ => Err(kind_error("a collection of clients")),
        }
    }

    /// Adds a client to the collection (a workspace's collection: it leaves the workspace it was on).
    ///
    /// # Errors
    ///
    /// The collection is not one a shell may change, or the client is gone.
    pub fn add(&self, client: impl AsClient) -> Result<()> {
        Ok(self.clients()?.add(&Std::current()?.core_client(client.client_id())?)?)
    }

    /// Takes a client off the collection: `true` when it was there.
    ///
    /// # Errors
    ///
    /// The collection is not one a shell may change.
    pub fn remove(&self, client: impl AsClient) -> Result<bool> {
        Ok(self
            .clients()?
            .remove(&Std::current()?.core_client(client.client_id())?)?)
    }

    /// Trades the places of two clients in the collection's order.
    ///
    /// # Errors
    ///
    /// A client is not in the collection.
    pub fn swap(&self, a: impl AsClient, b: impl AsClient) -> Result<()> {
        let std = Std::current()?;
        Ok(self
            .clients()?
            .swap(&std.core_client(a.client_id())?, &std.core_client(b.client_id())?)?)
    }

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

    /// Listens to one event of the members, now and later.
    fn on_client(
        &self,
        install: impl FnOnce(&mut model::ClientCollectionListenerHandlers, Filter, Std),
        closing: bool,
    ) -> Result<Subscription> {
        let object = self.clients()?;
        let here = Rc::new(RefCell::new(Here {
            members: client_keys(object)?.into_iter().collect(),
            gone: HashSet::new(),
        }));
        let (added, left) = (Rc::clone(&here), Rc::clone(&here));
        let joined = object.on_added(move |client: model::Client| {
            let Ok(id) = client.id() else { return };
            let mut here = added.borrow_mut();
            here.gone.remove(&id);
            here.members.insert(id);
        })?;
        let parted = object.on_removed(move |client: model::Client| {
            let Ok(id) = client.id() else { return };
            let mut here = left.borrow_mut();
            if here.members.remove(&id) {
                here.gone.insert(id);
            }
        })?;
        let filter: Filter = Rc::new(move |id, last| {
            let mut here = here.borrow_mut();
            if here.members.contains(id) {
                return true;
            }
            // The last event of a window that closed: the collection may have dropped it before the core told its listeners.
            if closing && last {
                return here.gone.remove(id);
            }
            false
        });
        listen_clients(
            &self.std,
            filter,
            Box::new((vec![joined, parted], Rc::clone(self.clients()?))),
            install,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_title_changed = Some(Box::new(move |client, title| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client, &title);
                    }
                    Ok(())
                }));
            },
            false,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_app_id_changed = Some(Box::new(move |client, app_id| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client, &app_id);
                    }
                    Ok(())
                }));
            },
            false,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_focus_changed = Some(Box::new(move |client, focused| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client, focused);
                    }
                    Ok(())
                }));
            },
            false,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_geometry_changed = Some(Box::new(move |client, rect| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client, Rect::new(rect.x, rect.y, rect.width, rect.height));
                    }
                    Ok(())
                }));
            },
            false,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_decoration_changed = Some(Box::new(move |client, decoration| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client, &decoration);
                    }
                    Ok(())
                }));
            },
            false,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_closed = Some(Box::new(move |client| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, true))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client);
                    }
                    Ok(())
                }));
            },
            true,
        )
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
        self.on_client(
            move |handlers, filter, std| {
                handlers.on_ready = Some(Box::new(move |client| {
                    if let Some(id) = id_of(&client).filter(|id| filter(id, false))
                        && let Some(client) = id.parse().ok().and_then(|raw| Client::from_raw_id(&std, raw))
                    {
                        f(&client);
                    }
                    Ok(())
                }));
            },
            false,
        )
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

/// Listens to the events of the one client `id`: what a client's own `on_*` methods are.
pub(crate) fn listen_one(
    std: &Std,
    id: u64,
    install: impl FnOnce(&mut model::ClientCollectionListenerHandlers, Filter, Std),
) -> Result<Subscription> {
    let wanted = id.to_string();
    listen_clients(std, Rc::new(move |id, _| id == wanted), Box::new(()), install)
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
        let Source::Table(id) = &self.source else {
            return Err(kind_error("a mode event"));
        };
        self.listen(*id, abi::STD_EVENT_MODE_CHANGED, false, move |std, raw| {
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
        let Source::Table(id) = &self.source else {
            return Err(kind_error("a geometry event"));
        };
        self.listen(*id, abi::STD_EVENT_GEOMETRY_CHANGED, false, move |std, raw| {
            if let Some(output) = Output::from_raw(std, raw.member) {
                f(&output, raw.rect);
            }
        })
    }
}

impl Collection<Workspace> {
    /// The list of workspaces.
    pub(crate) fn list(std: &Std) -> Result<Collection<Workspace>> {
        let objects = std.objects()?;
        Ok(Collection {
            std: std.clone(),
            source: Source::Workspaces(Rc::new(classes::Workspaces::get(&objects)?)),
            _members: PhantomData,
        })
    }

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
        workspace.remove_from_list()
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

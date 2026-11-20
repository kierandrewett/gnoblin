//! What a library keeps for one generation: its objects, its listeners and the work that waits for the next turn.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi::GnoblinCtx;

use super::shell::ShellImpl;
use crate::value::{Callable, ClassId, MemberId, Obj, Val};

/// The native state of an object, shared so a call can hold it while the table is free again.
pub(crate) type State = Rc<RefCell<Box<dyn Any>>>;

/// One object.
pub(crate) struct Entry {
    pub(crate) class: ClassId,
    pub(crate) state: State,
    /// The shell's methods, for an object of an interface class made by `implement`.
    pub(crate) shell: Option<Rc<ShellImpl>>,
    /// Who holds it: the shell's references and the native holders (`Cx::hold`).
    pub(crate) refs: u32,
    pub(crate) parent: Option<Obj>,
    pub(crate) children: Vec<Obj>,
    /// The class is retained (it stays until `destroy`) or a service (one per generation).
    pub(crate) pinned: bool,
}

struct Slot {
    generation: u32,
    entry: Option<Entry>,
}

/// The slots of a generation's objects. A handle is `generation << 32 | (index + 1)`, so zero is nil and a slot's reuse never
/// makes an old handle name a new object.
#[derive(Default)]
pub(crate) struct Slab {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

/// The slot index and generation a handle names.
fn split(obj: Obj) -> Option<(usize, u32)> {
    let index = (obj & 0xffff_ffff) as u32;
    if index == 0 {
        return None;
    }
    Some((index as usize - 1, (obj >> 32) as u32))
}

impl Slab {
    /// Takes a slot for `entry` and returns its handle.
    pub(crate) fn insert(&mut self, entry: Entry) -> Obj {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.entry = Some(entry);
            return (u64::from(slot.generation) << 32) | u64::from(index + 1);
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot {
            generation: 1,
            entry: Some(entry),
        });
        (1_u64 << 32) | u64::from(index + 1)
    }

    pub(crate) fn get(&self, obj: Obj) -> Option<&Entry> {
        let (index, generation) = split(obj)?;
        let slot = self.slots.get(index)?;
        if slot.generation == generation {
            slot.entry.as_ref()
        } else {
            None
        }
    }

    pub(crate) fn get_mut(&mut self, obj: Obj) -> Option<&mut Entry> {
        let (index, generation) = split(obj)?;
        let slot = self.slots.get_mut(index)?;
        if slot.generation == generation {
            slot.entry.as_mut()
        } else {
            None
        }
    }

    /// Takes the object out and retires its handle. The entry is returned so the caller drops it once no borrow is held: dropping
    /// it can run a shell's `drop_user`.
    pub(crate) fn remove(&mut self, obj: Obj) -> Option<Entry> {
        let (index, generation) = split(obj)?;
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let entry = slot.entry.take()?;
        slot.generation = slot.generation.checked_add(1).unwrap_or(1);
        self.free.push(index as u32);
        Some(entry)
    }

    /// Takes every object out, for a generation that retires.
    pub(crate) fn drain(&mut self) -> Vec<Entry> {
        self.free.clear();
        self.slots.drain(..).filter_map(|slot| slot.entry).collect()
    }

    /// How many objects are alive.
    pub(crate) fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.entry.is_some()).count()
    }
}

/// A listener: a closure that hears one member of one object.
pub(crate) struct Listener {
    pub(crate) reg: u64,
    pub(crate) obj: Obj,
    pub(crate) member: MemberId,
    pub(crate) closure: Rc<dyn Callable>,
}

/// The listeners of a generation, by registration and by what they hear.
#[derive(Default)]
pub(crate) struct Listeners {
    pub(crate) by_reg: HashMap<u64, Rc<Listener>>,
    pub(crate) by_target: HashMap<(Obj, MemberId), Vec<u64>>,
}

impl Listeners {
    pub(crate) fn hears(&self, obj: Obj, member: MemberId) -> bool {
        self.by_target.contains_key(&(obj, member))
    }

    pub(crate) fn add(&mut self, reg: u64, listener: Listener) {
        self.by_target
            .entry((listener.obj, listener.member))
            .or_default()
            .push(reg);
        self.by_reg.insert(reg, Rc::new(listener));
    }

    pub(crate) fn remove(&mut self, reg: u64) -> Option<Rc<Listener>> {
        let listener = self.by_reg.remove(&reg)?;
        let key = (listener.obj, listener.member);
        if let Some(regs) = self.by_target.get_mut(&key) {
            regs.retain(|held| *held != reg);
            if regs.is_empty() {
                self.by_target.remove(&key);
            }
        }
        Some(listener)
    }

    /// Every listener of `obj`, for the object's end.
    pub(crate) fn of_object(&self, obj: Obj) -> Vec<u64> {
        self.by_target
            .iter()
            .filter(|((target, _), _)| *target == obj)
            .flat_map(|(_, regs)| regs.iter().copied())
            .collect()
    }

    /// The listeners of one member, in the order they were added.
    pub(crate) fn heard_by(&self, obj: Obj, member: MemberId) -> Vec<Rc<Listener>> {
        self.by_target
            .get(&(obj, member))
            .into_iter()
            .flatten()
            .filter_map(|reg| self.by_reg.get(reg).cloned())
            .collect()
    }
}

/// Work that waits for the next turn, in the order it was asked for.
pub(crate) enum Pending {
    /// A signal of `obj`, with its payload.
    Signal { obj: Obj, member: MemberId, args: Vec<Val> },
    /// A property of `obj` changed: listeners hear its value as of the turn, once however many writes there were.
    Changed { obj: Obj, member: MemberId },
    /// Work of the library's own: run from the turn with the object it is about.
    Task {
        obj: Obj,
        run: Box<dyn FnOnce(&mut super::cx::Cx<'_>)>,
    },
}

/// Everything a library holds for one generation.
pub(crate) struct CtxState {
    pub(crate) ctx: GnoblinCtx,
    pub(crate) objects: RefCell<Slab>,
    pub(crate) listeners: RefCell<Listeners>,
    pub(crate) queue: RefCell<Vec<Pending>>,
    /// A flush is posted or running.
    pub(crate) scheduled: Cell<bool>,
    /// The singleton object of each service class.
    pub(crate) services: RefCell<HashMap<ClassId, Obj>>,
}

impl CtxState {
    pub(crate) fn new(ctx: GnoblinCtx) -> CtxState {
        CtxState {
            ctx,
            objects: RefCell::new(Slab::default()),
            listeners: RefCell::new(Listeners::default()),
            queue: RefCell::new(Vec::new()),
            scheduled: Cell::new(false),
            services: RefCell::new(HashMap::new()),
        }
    }
}

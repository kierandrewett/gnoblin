//! What `add_listener` does with a listener the shell implements.
//!
//! A listener is an object of an interface (`client_listener`, `output_collection_listener`, ...) whose methods are all optional. Adding
//! one registers a listener of each signal of the target that the listener has a method for, so it stands in the same list as the
//! listeners a shell makes with `listen` and is told in the order they were added. A method the listener did not define is never
//! called, a listener that is added twice is added once, and one that is removed hears nothing, whatever was queued for it already.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Cx, ShellImpl};
use gnoblin_object::{Callable, Fail, MemberId, Obj, Res, Val};

/// The method of a shell's listener that one signal calls.
struct Forward {
    shell: Rc<ShellImpl>,
    method: MemberId,
    /// Whether the listener is still added: a call queued before it was removed is not made.
    live: Rc<Cell<bool>>,
}

impl Callable for Forward {
    fn call(&self, _member: MemberId, args: &[Val]) -> Res<Val> {
        if self.live.get() {
            self.shell.call(self.method, args)
        } else {
            Ok(Val::None)
        }
    }
}

/// One listener added to an object: the registrations it made, and whether it still stands.
pub(super) struct Listening {
    listener: Obj,
    regs: Vec<u64>,
    live: Rc<Cell<bool>>,
}

/// The listeners an object was given.
pub(super) type Registered = Vec<Listening>;

/// What a new listener is told of what is there already: its method for `added` and the calls to make.
pub(super) struct Replay {
    pub(super) closure: Rc<dyn Callable>,
}

impl Replay {
    /// Calls the listener's `on_added` with each of `calls` from the next turn, as a listener added before them would have been.
    pub(super) fn tell(self, cx: &Cx<'_>, calls: Vec<Vec<Val>>) {
        if calls.is_empty() {
            return;
        }
        cx.later(move |cx| {
            for args in calls {
                args.iter().for_each(|arg| cx.hand_out(arg));
                if let Err(fail) = self.closure.call(0, &args) {
                    cx.log(abi::LOG_WARN, &format!("a listener failed: {fail}"));
                }
            }
        });
    }
}

/// Adds `listener` (an object value) to `target`, whose registrations are `regs`. Answers the listener's way to be told of what is
/// already there, when it has an `on_added`.
pub(super) fn add(cx: &Cx<'_>, target: Obj, regs: &mut Registered, listener: &Val) -> Res<Option<Replay>> {
    let listener = listener.object()?;
    if regs.iter().any(|held| held.listener == listener) {
        return Ok(None);
    }
    let shell = cx.shell(listener)?;
    let interface = cx.class_of(listener)?;
    let class = cx.class_of(target)?;
    let def = cx
        .class_def_of(class)
        .ok_or_else(|| Fail::err("the target's class is not in the catalog"))?;
    let live = Rc::new(Cell::new(true));
    let mut mine = Vec::new();
    let mut replay = None;
    for signal in def.members.iter().filter(|member| member.kind == abi::MEMBER_SIGNAL) {
        let Ok(method) = cx.member_of(interface, abi::MEMBER_METHOD, &format!("on_{}", signal.name)) else {
            continue;
        };
        if !shell.has(method) {
            continue;
        }
        let forward: Rc<dyn Callable> = Rc::new(Forward {
            shell: Rc::clone(&shell),
            method,
            live: Rc::clone(&live),
        });
        if signal.name == "added" {
            replay = Some(Replay {
                closure: Rc::clone(&forward),
            });
        }
        match cx.listen(target, signal.name, forward) {
            Ok(reg) => mine.push(reg),
            Err(fail) => {
                mine.into_iter().for_each(|reg| cx.unlisten(reg));
                return Err(fail);
            }
        }
    }
    regs.push(Listening {
        listener,
        regs: mine,
        live,
    });
    Ok(replay)
}

/// Removes a listener added with [`add`]. One that is not added is ignored.
pub(super) fn remove(cx: &Cx<'_>, regs: &mut Registered, listener: &Val) -> Res<()> {
    let listener = listener.object()?;
    if let Some(at) = regs.iter().position(|held| held.listener == listener) {
        let held = regs.remove(at);
        held.live.set(false);
        held.regs.into_iter().for_each(|reg| cx.unlisten(reg));
    }
    Ok(())
}

//! The PipeWire side of the privacy service: connects, binds capture nodes and feeds every
//! registry, node and core event into [`Tracker`]. It is the only code that touches
//! PipeWire, and it runs on the thread [`crate::start`] spawns.
//!
//! Every event ends in [`flush`], which publishes the tracker's activity only when it
//! changed. A failed connection is retried after [`RECONNECT_DELAY`]; while it is down the
//! published state is all-false, so a caller never sees a stale "in use".

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pipewire::context::ContextRc;
use pipewire::keys;
use pipewire::main_loop::MainLoopRc;
use pipewire::node::{Node, NodeListener, NodeState};
use pipewire::proxy::ProxyT;
use pipewire::registry::{GlobalObject, RegistryRc};
use pipewire::spa::utils::dict::DictRef;
use pipewire::types::ObjectType;

use crate::log;
use crate::model::Tracker;
use crate::{Privacy, RECONNECT_DELAY};

/// The core's object id. An error naming it means the whole connection is gone, not one node.
const CORE_ID: u32 = 0;

/// A bound capture node. Dropping the proxy or the listener stops PipeWire events for it, so
/// both are held here for as long as the global exists.
struct Held {
    proxy: u32,
    _node: Node,
    _listener: NodeListener,
}

/// Everything the PipeWire callbacks share on one connection.
struct PipewireSession {
    service: Privacy,
    tracker: RefCell<Tracker>,
    held: RefCell<HashMap<u32, Held>>,
    mainloop: MainLoopRc,
}

impl PipewireSession {
    /// Publish the tracker's change, if any, and stop the loop when the connection is lost
    /// so [`run_once`] can rebuild it.
    fn flush(&self) {
        let (change, lost) = {
            let mut tracker = self.tracker.borrow_mut();
            (tracker.take_change(), tracker.is_lost())
        };
        if let Some(activity) = change {
            self.service.publish(activity);
        }
        if lost {
            self.mainloop.quit();
        }
    }
}

/// Own the PipeWire thread's loop forever: connect, read until the connection is lost, then
/// retry after [`RECONNECT_DELAY`].
pub fn run(service: &Privacy) {
    pipewire::init();
    let mut logged_down = false;
    loop {
        match run_once(service) {
            Ok(()) => logged_down = false,
            Err(error) => {
                if !logged_down {
                    log::info!(
                        area = "services",
                        ?error,
                        "PipeWire is not available, so privacy is unknown"
                    );
                    logged_down = true;
                }
            }
        }
        std::thread::sleep(RECONNECT_DELAY);
    }
}

/// One connection, from connect to loss. Returns an error only when the connection could not
/// be made; a connection that is lost after it was made returns `Ok` and the caller retries.
fn run_once(service: &Privacy) -> Result<(), pipewire::Error> {
    let mainloop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let session = Rc::new(PipewireSession {
        service: service.clone(),
        tracker: RefCell::new(Tracker::default()),
        held: RefCell::new(HashMap::new()),
        mainloop: mainloop.clone(),
    });
    session.tracker.borrow_mut().reset();

    let initial_sync = Rc::new(RefCell::new(None));
    let _core_listener = {
        let done_session = Rc::clone(&session);
        let session = Rc::clone(&session);
        let initial_sync = Rc::clone(&initial_sync);
        core.add_listener_local()
            .done(move |id, seq| {
                if id == CORE_ID && Some(seq.seq()) == *initial_sync.borrow() {
                    done_session.tracker.borrow_mut().synced();
                    done_session.flush();
                }
            })
            .error(move |id, _seq, _res, message| {
                log::debug!(
                    area = "services",
                    id = id,
                    message = message,
                    "PipeWire reported an error"
                );
                // The registry is the only other proxy this service owns, so an error on
                // anything but a bound node means the connection is gone.
                let bound = session.held.borrow().values().any(|held| held.proxy == id);
                if id == CORE_ID || !bound {
                    session.tracker.borrow_mut().lost();
                    session.held.borrow_mut().clear();
                } else {
                    session.tracker.borrow_mut().node_errored(id);
                    session.held.borrow_mut().retain(|_, held| held.proxy != id);
                }
                session.flush();
            })
            .register()
    };

    let _registry_listener = {
        let added = Rc::clone(&session);
        let removed = Rc::clone(&session);
        let bind_registry = registry.clone();
        registry
            .add_listener_local()
            .global(move |global: &GlobalObject<&DictRef>| {
                bind_node(&added, &bind_registry, global);
            })
            .global_remove(move |id| {
                removed.held.borrow_mut().remove(&id);
                removed.tracker.borrow_mut().node_removed(id);
                removed.flush();
            })
            .register()
    };

    let sync = core.sync(0)?;
    *initial_sync.borrow_mut() = Some(sync.seq());

    mainloop.run();
    log::info!(area = "services", "PipeWire connection lost, so privacy is unknown");
    session.tracker.borrow_mut().lost();
    // The node listeners hold an `Rc` to this session and the session holds them, so the
    // cycle only breaks when they are dropped here. Without this, every reconnect leaks.
    session.held.borrow_mut().clear();
    session.flush();
    Ok(())
}

/// Bind one global if it is a microphone or camera capture node, and track its run state.
/// Every other global is ignored.
fn bind_node(session: &Rc<PipewireSession>, registry: &RegistryRc, global: &GlobalObject<&DictRef>) {
    if global.type_ != ObjectType::Node {
        return;
    }
    let Some(props) = global.props.as_ref() else {
        return;
    };
    let class = props.get(*keys::MEDIA_CLASS).unwrap_or_default();
    let microphone = class.starts_with("Audio/Source") || class == "Stream/Input/Audio";
    let camera = class.starts_with("Video/Source");
    if !microphone && !camera {
        return;
    }
    let node: Node = match registry.bind(global) {
        Ok(node) => node,
        Err(error) => {
            log::debug!(area = "services", ?error, "a capture node could not be bound");
            return;
        }
    };
    let proxy = node.upcast_ref().id();
    let global_id = global.id;
    session
        .tracker
        .borrow_mut()
        .node_bound(global_id, proxy, microphone, camera);
    let listener = {
        let session = Rc::clone(session);
        node.add_listener_local()
            .info(move |info| {
                let running = matches!(info.state(), NodeState::Running);
                session.tracker.borrow_mut().node_info(global_id, running);
                session.flush();
            })
            .register()
    };
    session.held.borrow_mut().insert(
        global_id,
        Held {
            proxy,
            _node: node,
            _listener: listener,
        },
    );
    session.flush();
}

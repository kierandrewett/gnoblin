//! The scene service: the tree's root, the metres in a scene unit, and the cameras, and the registration of every class of
//! `scene.toml`.

use std::rc::Rc;

use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Res, Val};

use super::Kind;
use super::node::{find_below, node_of};
use super::registry::{NodeHandle, SceneWorld, world_of};

/// The state of the scene service.
struct SceneService {
    world: Rc<SceneWorld>,
}

impl SceneService {
    /// The object that holds the tree's own root, made the first time it is asked for.
    fn root(&self, cx: &mut gnoblin_object::server::Cx<'_>) -> Res<Val> {
        let node = self.world.root()?;
        if let Some(entry) = self.world.entry(node) {
            return cx.handout(entry.obj);
        }
        let obj = cx.spawn(
            "scene_node",
            NodeHandle {
                world: Rc::clone(&self.world),
                node,
                root: true,
            },
        )?;
        self.world.adopt(node, obj, Kind::Node, true);
        cx.handout(obj)
    }
}

pub(super) fn register(builder: &mut Builder) {
    super::node::register(builder);
    super::view::register(builder);
    super::bindings::register(builder);
    let _ = builder
        .class::<SceneService>("scene")
        .ctor(|cx, _inits| Ok(SceneService { world: world_of(cx)? }))
        .get("root", |service, cx| service.root(cx))
        .get("metres_per_unit", |service, _| {
            Ok(Val::Double(service.world.metres.get()))
        })
        .set("metres_per_unit", |service, _, value| {
            let metres = value.double()?;
            if !(metres.is_finite() && metres > 0.0) {
                return Err(Fail::type_error("Scene.metresPerUnit: must be a finite number above 0"));
            }
            service.world.metres.set(metres);
            Ok(())
        })
        .get("cameras", |service, cx| {
            let mut cameras: Vec<(u64, u64)> = service
                .world
                .entries
                .borrow()
                .iter()
                .filter(|(node, entry)| entry.kind == Kind::Camera3D && service.world.alive(**node))
                .map(|(node, entry)| (*node, entry.obj))
                .collect();
            cameras.sort_unstable();
            cameras
                .into_iter()
                .map(|(_, obj)| cx.handout(obj))
                .collect::<Res<Vec<_>>>()
                .map(Val::List)
        })
        .method("add", |service, cx, args| {
            let obj = args[0].object()?;
            let (node, _) = node_of(cx, obj)?;
            let mounted = service
                .world
                .g
                .node(gnoblin_core::host::NodeId(node))
                .parent()
                .ok()
                .flatten()
                .is_some();
            if !mounted {
                let root = service.world.root()?;
                service.world.insert(node, root, None)?;
            }
            cx.handout(obj)
        })
        .method("find", |service, cx, args| {
            let root = service.world.root()?;
            match find_below(&service.world, root, args[0].text()?) {
                Some(node) => match service.world.entry(node) {
                    Some(entry) => cx.handout(entry.obj),
                    None => Ok(Val::None),
                },
                None => Ok(Val::None),
            }
        });
}

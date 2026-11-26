//! The drags of a room layout's bindings: which binding takes a chord, what each frame of the drag does, and how it ends.
//!
//! A drag starts when exactly a binding's buttons have been down together for the session's settle time over a panel, so a two-button
//! chord is not taken for its first button. The candidates are asked in order of priority; the first that takes it holds the drag, which
//! pins the panel (the ring leaves it alone) and claims the binding's buttons on the controller, so the window under the ray does not
//! also get them. Each frame the drag updates; the release of the chord ends it, and a lost hand, a binding that was removed or turned
//! off, a panel that went, a throw or the end of the session cancels it. A drag that ends leaves the panel pinned; a cancelled one gives
//! it back to the ring unless it was pinned before. Every way a drag ends gives its claim back.
//!
//! A binding's hooks are its handler's where the handler defines them, and the binding's own (`Native`) otherwise. Poses are in the
//! room's placement frame.

use std::rc::Rc;

use gnoblin_core::NodeId;
use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{MemberId, Res, Val};

use super::controller::{ControllerData, Owner, inputs_of};
use super::layout::{BindingData, CORNERS, Native, Tracking};
use super::room::{self, Panel, RoomData};
use super::{SessionData, World};
use crate::animation::staging::Prop;
use crate::math::transform::PoseMatrix;
use crate::math::vector::{Flat, Point3, RayLine};

/// A frame longer than this is told as this, in seconds.
const MAX_DELTA: f64 = 0.1;

/// What the binding's own drag keeps from its start.
#[derive(Clone, Debug)]
pub(crate) enum State {
    /// The binding has no drag of its own, or the ray struck no panel.
    None,
    Move {
        start_local: PoseMatrix,
        start_world: PoseMatrix,
        start_controller: PoseMatrix,
        distance: f64,
        offset: Point3,
    },
    Resize {
        start_local: PoseMatrix,
        start_world: PoseMatrix,
        plane: Flat,
        anchor: (f64, f64),
        first: (f64, f64),
        width: f64,
        height: f64,
        /// The size the hand asks for, in metres.
        wanted: (f64, f64),
    },
    Rotate {
        start_local: PoseMatrix,
        start_world: PoseMatrix,
        start_controller: PoseMatrix,
    },
    Scale {
        start_local: PoseMatrix,
        start_world: PoseMatrix,
        start: Point3,
        aim: Point3,
    },
}

/// A drag that is running.
pub(crate) struct Run {
    pub(crate) controller: Rc<ControllerData>,
    pub(crate) binding: Rc<BindingData>,
    pub(crate) panel: Panel,
    pub(crate) hit: Point3,
    pub(crate) last: f64,
    /// Whether the panel was pinned before the drag, so a cancel can give it back to the ring.
    pub(crate) was_pinned: bool,
    pub(crate) state: State,
}

/// The hooks of a drag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hook {
    Start,
    Update,
    End,
    Cancel,
}

impl Hook {
    fn member(self) -> &'static str {
        match self {
            Hook::Start => "on_start",
            Hook::Update => "on_update",
            Hook::End => "on_end",
            Hook::Cancel => "on_cancel",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Hook::Start => "onStart",
            Hook::Update => "onUpdate",
            Hook::End => "onEnd",
            Hook::Cancel => "onCancel",
        }
    }
}

/// What an event of a drag says.
struct Event<'a> {
    now: f64,
    delta: f64,
    controller: &'a ControllerData,
    binding: &'a BindingData,
    panel: Option<Panel>,
    hit: Option<Point3>,
    room: &'a RoomData,
    unit: f64,
}

fn vec3(point: Point3) -> Val {
    crate::math::vec3_val(point)
}

impl Event<'_> {
    /// The `xr_drag_event` record, each object in it handed to the shell.
    fn record(&self, world: &World, cx: &Cx<'_>) -> Res<Val> {
        let ray = self.controller.ray(&world.g);
        Ok(Val::Record {
            class: 0,
            fields: vec![
                Val::Double(self.now),
                Val::Double(self.delta),
                cx.handout(self.controller.obj.get())?,
                match self.panel {
                    Some(panel) => cx.handout(panel.obj)?,
                    None => Val::None,
                },
                Val::Record {
                    class: 0,
                    fields: vec![vec3(ray.origin), vec3(ray.direction)],
                },
                self.hit.map_or(Val::None, vec3),
                Val::Text(self.controller.hand.clone()),
                Val::Text(self.binding.trigger.borrow().clone()),
                cx.handout(self.room.obj)?,
                Val::Double(self.unit),
            ],
        })
    }
}

/// The member of the handler interface for `name`.
fn member(cx: &Cx<'_>, name: &str) -> Option<MemberId> {
    let class = cx.class_named("xr_drag_handler").ok()?;
    cx.member_of(class, abi::MEMBER_METHOD, name).ok()
}

/// Runs `hook` of the handler when it defines it: `None` when it does not, the answer when it does.
fn handler_hook(world: &World, cx: &Cx<'_>, hook: Hook, event: &Event<'_>) -> Option<Res<Val>> {
    let handler = event.binding.handler.borrow().clone()?;
    let member = member(cx, hook.member())?;
    if !handler.shell.has(member) {
        return None;
    }
    Some(
        event
            .record(world, cx)
            .and_then(|record| handler.shell.call(member, &[record])),
    )
}

/// Reports a hook that threw.
fn report(world: &World, binding: &BindingData, hook: &str, message: &str) {
    world.warn(&format!("{}.{hook} threw: {message}", binding.id.borrow()));
}

/// The inputs a drag of `binding` claims: its handler's `claims`, or its trigger's buttons.
fn claims(world: &World, cx: &Cx<'_>, binding: &BindingData, controller: &ControllerData) -> Res<Vec<usize>> {
    let handler = binding.handler.borrow().clone();
    if let (Some(handler), Some(member)) = (handler, member(cx, "claims"))
        && handler.shell.has(member)
    {
        let answer = handler.shell.call(member, &[])?;
        let names: Vec<String> = answer
            .list()?
            .iter()
            .map(|name| name.text().map(str::to_owned))
            .collect::<Res<_>>()?;
        return inputs_of(&controller.target(), &names);
    }
    let _ = world;
    Ok(binding.parsed.borrow().buttons.clone())
}

/// The binding's own start: the state its drag keeps, or none to decline.
fn native_start(world: &World, event: &Event<'_>) -> Option<State> {
    let native = event.binding.native.borrow().clone();
    let tracked = event.controller.tracked(&world.g);
    let (Some(panel), Some(hit)) = (event.panel, event.hit) else {
        return None;
    };
    let start_world = world.world_pose(panel.view);
    let start_local = world.pose(panel.view);
    let controller = event.controller.pose(&world.g);
    Some(match native {
        Native::Plain => return None,
        Native::Move { .. } => {
            if !tracked {
                return None;
            }
            let ray = event.controller.ray(&world.g);
            State::Move {
                start_local,
                start_world,
                start_controller: controller,
                // Measured to the point the ray struck, so the panel does not jump to the ray's end.
                distance: hit.distance(ray.origin),
                offset: start_world.position.subtract(hit),
            }
        }
        Native::Resize { edges, .. } => {
            let local = start_world.inverse().transform_point(hit);
            // The panel's local y points up, so the top corners are the ones with y above the centre.
            let corner = match edges {
                0 => (
                    if local.x >= 0.0 { 1.0 } else { -1.0 },
                    if local.y >= 0.0 { 1.0 } else { -1.0 },
                ),
                at => CORNERS[(at - 1) as usize],
            };
            let width = world.number(panel.view, "width");
            let height = world.number(panel.view, "height");
            let scale = start_world.scale * event.unit;
            State::Resize {
                start_local,
                start_world,
                plane: start_world.plane(),
                anchor: (-corner.0 * width / 2.0, -corner.1 * height / 2.0),
                first: (local.x, local.y),
                width,
                height,
                wanted: (width * scale, height * scale),
            }
        }
        Native::Rotate => {
            if !tracked {
                return None;
            }
            State::Rotate {
                start_local,
                start_world,
                start_controller: controller,
            }
        }
        Native::Scale { .. } => {
            if !tracked {
                return None;
            }
            let ray = event.controller.ray(&world.g);
            State::Scale {
                start_local,
                start_world,
                start: controller.position,
                aim: ray.direction.normalize(),
            }
        }
    })
}

/// A pose that is only `pose`'s rotation.
fn turn_of(pose: &PoseMatrix) -> PoseMatrix {
    PoseMatrix::new(Point3::default(), pose.rotation(), 1.0)
}

/// `pose` with another rotation.
fn with_rotation(pose: &PoseMatrix, rotation: [f64; 3]) -> PoseMatrix {
    PoseMatrix::new(pose.position, rotation, pose.scale)
}

/// The binding's own update.
fn native_update(world: &World, run: &mut Run, unit: f64) -> Res<()> {
    let native = run.binding.native.borrow().clone();
    let view = run.panel.view;
    match (&mut run.state, native) {
        (
            State::Move {
                start_world,
                start_controller,
                distance,
                offset,
                ..
            },
            Native::Move { tracking, keep },
        ) => {
            let target = match tracking {
                Tracking::Controller => {
                    let delta = run.controller.pose(&world.g).multiply(&start_controller.inverse());
                    let moved = delta.multiply(start_world);
                    if keep {
                        with_rotation(&moved, start_world.rotation())
                    } else {
                        moved
                    }
                }
                Tracking::Ray => {
                    let ray = run.controller.ray(&world.g);
                    let position = ray.origin.add(ray.direction.normalize().scale(*distance)).add(*offset);
                    PoseMatrix::new(position, start_world.rotation(), start_world.scale)
                }
            };
            world.place_world(view, &target)
        }
        (
            State::Resize {
                start_world,
                plane,
                anchor,
                first,
                width,
                height,
                wanted,
                ..
            },
            Native::Resize {
                min_width, min_height, ..
            },
        ) => {
            let ray = run.controller.ray(&world.g);
            let Some((point, _)) = plane.intersect(ray) else {
                return Ok(());
            };
            // The panel grows by how far the ray is from the anchor compared with where it started, so the first update does not jump.
            // A metre is `1 / (scale * unit)` of the start's pixels.
            let per_metre = 1.0 / (start_world.scale * unit);
            let at = start_world.inverse().transform_point(point);
            let kx = (at.x - anchor.0) / (first.0 - anchor.0);
            let ky = (at.y - anchor.1) / (first.1 - anchor.1);
            let grown_width = (*width * kx).max(min_width * per_metre);
            let grown_height = (*height * ky).max(min_height * per_metre);
            *wanted = (grown_width / per_metre, grown_height / per_metre);
            // A panel has one scale, so the preview grows evenly to the larger of the two asked-for sizes.
            let k = (grown_width / *width).max(grown_height / *height);
            let anchor_world = start_world.transform_point(Point3::new(anchor.0, anchor.1, 0.0));
            let centre = anchor_world.add(start_world.position.subtract(anchor_world).scale(k));
            let target = PoseMatrix::new(centre, start_world.rotation(), start_world.scale * k);
            world.place_world(view, &target)
        }
        (
            State::Rotate {
                start_world,
                start_controller,
                ..
            },
            Native::Rotate,
        ) => {
            let hand = turn_of(&run.controller.pose(&world.g).multiply(&start_controller.inverse()));
            let rotation = hand.multiply(&turn_of(start_world)).rotation();
            world.place_world(view, &with_rotation(start_world, rotation))
        }
        (
            State::Scale {
                start_world,
                start,
                aim,
                ..
            },
            Native::Scale { min, max },
        ) => {
            let metres = run.controller.pose(&world.g).position.subtract(*start).dot(*aim) * unit;
            let scale = (start_world.scale + metres).max(min).min(max);
            world.place_world(
                view,
                &PoseMatrix::new(start_world.position, start_world.rotation(), scale),
            )
        }
        _ => Ok(()),
    }
}

/// The binding's own end: a resize asks the window for one new size when the panel's needs it.
fn native_end(world: &World, run: &Run, unit: f64) -> Res<()> {
    let (State::Resize { wanted, .. }, Native::Resize { pixels_per_metre, .. }) =
        (&run.state, run.binding.native.borrow().clone())
    else {
        return Ok(());
    };
    let client = run.panel.client;
    let window = world.g.node(NodeId(client));
    if !window.is_alive() {
        return Ok(());
    }
    let desired = (
        (wanted.0 * pixels_per_metre).round(),
        (wanted.1 * pixels_per_metre).round(),
    );
    if desired.0 < 1.0 || desired.1 < 1.0 {
        return Ok(());
    }
    let width = crate::nodes::number(&world.g, client, "width");
    let height = crate::nodes::number(&world.g, client, "height");
    let aspect_changed = (desired.0 / desired.1 / (width / height) - 1.0).abs() > 0.02;
    let too_coarse = width < desired.0 * 0.85 || height < desired.1 * 0.85;
    if !aspect_changed && !too_coarse {
        return Ok(());
    }
    // One request: both sides go in the turn's one transaction, so the client gets one configure.
    let refused = |error: gnoblin_core::CoreError| gnoblin_object::Fail::new(error.kind().code(), error.to_string());
    window
        .set("width", gnoblin_core::Value::Double(desired.0))
        .map_err(refused)?;
    window
        .set("height", gnoblin_core::Value::Double(desired.1))
        .map_err(refused)?;
    // The picture will have `desired` pixels, and the panel keeps the metres the hand drew.
    let pose = world.world_pose(run.panel.view);
    let scale = 1.0 / (pixels_per_metre * unit);
    world.place_world(run.panel.view, &PoseMatrix::new(pose.position, pose.rotation(), scale))
}

/// The binding's own cancel: the panel goes back to where the drag found it.
fn native_cancel(world: &World, run: &Run) -> Res<()> {
    let start = match &run.state {
        State::None => return Ok(()),
        State::Move { start_local, .. }
        | State::Resize { start_local, .. }
        | State::Rotate { start_local, .. }
        | State::Scale { start_local, .. } => *start_local,
    };
    world.place(run.panel.view, &start)
}

/// Runs `hook` of a running drag: the handler's, or the binding's own. Answers whether it ran without a throw.
fn run_hook(
    world: &World,
    cx: &Cx<'_>,
    hook: Hook,
    run: &mut Run,
    session: &SessionData,
    room: &RoomData,
    now: f64,
) -> bool {
    let unit = room::unit(world, room);
    let event = Event {
        now,
        delta: ((now - run.last).max(0.0) / 1000.0).min(MAX_DELTA),
        controller: &run.controller,
        binding: &run.binding,
        panel: Some(run.panel),
        hit: Some(run.hit),
        room,
        unit,
    };
    let _ = session;
    let outcome = match handler_hook(world, cx, hook, &event) {
        Some(answer) => answer.map(drop),
        None => match hook {
            Hook::Update => native_update(world, run, unit),
            Hook::End => native_end(world, run, unit),
            Hook::Cancel => native_cancel(world, run),
            Hook::Start => Ok(()),
        },
    };
    match outcome {
        Ok(()) => true,
        Err(fail) => {
            report(world, &run.binding, hook.label(), &fail.message);
            false
        }
    }
}

/// Ends a drag with `hook` (`End` or `Cancel`): the claim goes back, the hook runs, and a cancelled drag gives its panel back to the ring
/// unless it was pinned before.
fn finish(world: &World, cx: &Cx<'_>, mut run: Run, hook: Hook, session: &SessionData, room: &RoomData, now: f64) {
    unclaim(world, &run);
    run_hook(world, cx, hook, &mut run, session, room, now);
    if hook == Hook::Cancel && !run.was_pinned {
        room.unpin(run.panel.view);
    }
}

/// Gives back what a drag claimed. A controller that is gone holds no claim, so only a refused write is reported.
fn unclaim(world: &World, run: &Run) {
    if let Err(fail) = run.controller.release(&world.g, &Owner::Binding(run.binding.obj.get())) {
        report(world, &run.binding, "release", &fail.message);
    }
}

/// Cancels every running drag once, with its hook, and gives every claim back.
pub(crate) fn cancel_all(world: &World, cx: &Cx<'_>) {
    let runs = std::mem::take(&mut world.rt.borrow_mut().runs);
    if runs.is_empty() {
        return;
    }
    let session = world.rt.borrow().session.clone();
    let room = session
        .as_ref()
        .and_then(|session| world.room_by_obj(session.room.get()));
    let now = world.now();
    for mut run in runs {
        unclaim(world, &run);
        if let (Some(session), Some(room)) = (&session, &room) {
            run_hook(world, cx, Hook::Cancel, &mut run, session, room, now);
            if !run.was_pinned {
                room.unpin(run.panel.view);
            }
        }
    }
}

/// One frame of the drags of `room`.
pub(crate) fn run(world: &World, cx: &Cx<'_>, session: &SessionData, room: &RoomData) {
    let now = world.now();
    let bindings: Vec<Rc<BindingData>> = room
        .layout
        .borrow()
        .as_ref()
        .map(|(_, layout)| layout.bindings.items.borrow().clone())
        .unwrap_or_default();
    let controllers: Vec<Rc<ControllerData>> = session.controllers.borrow().clone();
    for controller in &controllers {
        let down = controller.down.borrow().clone();
        {
            let mut rt = world.rt.borrow_mut();
            let fresh = rt.since.get(&controller.node).is_none_or(|(held, _)| *held != down);
            if fresh {
                rt.since.insert(controller.node, (down.clone(), now));
            }
        }
        let running = {
            let mut rt = world.rt.borrow_mut();
            rt.runs
                .iter()
                .position(|run| Rc::ptr_eq(&run.controller, controller))
                .map(|at| rt.runs.remove(at))
        };
        if let Some(mut run) = running {
            let held = run.binding.parsed.borrow().held(&down);
            let still = bindings.iter().any(|binding| Rc::ptr_eq(binding, &run.binding)) && run.binding.enabled.get();
            let alive = controller.alive(&world.g) && controller.tracked(&world.g);
            let shown = world.scene.alive(run.panel.view)
                && matches!(world.read(run.panel.view, "source"), Some(Prop::Node(source)) if source != 0)
                && world.g.node(NodeId(run.panel.client)).is_alive();
            if !alive || !still || !shown {
                finish(world, cx, run, Hook::Cancel, session, room, now);
            } else if !held {
                finish(world, cx, run, Hook::End, session, room, now);
            } else if run_hook(world, cx, Hook::Update, &mut run, session, room, now) {
                run.last = now;
                world.rt.borrow_mut().runs.push(run);
            } else {
                finish(world, cx, run, Hook::Cancel, session, room, now);
            }
            continue;
        }
        if down.is_empty() {
            world.rt.borrow_mut().declined.remove(&controller.node);
            continue;
        }
        let since = world.rt.borrow().since.get(&controller.node).map_or(now, |(_, at)| *at);
        let settled = now - since >= session.settle.get();
        if world.rt.borrow().declined.contains(&controller.node) || !controller.tracked(&world.g) || !settled {
            continue;
        }
        let mut candidates: Vec<Rc<BindingData>> = bindings
            .iter()
            .filter(|binding| binding.enabled.get() && binding.parsed.borrow().matches(&controller.hand, &down))
            .cloned()
            .collect();
        candidates.sort_by(|a, b| b.priority.get().total_cmp(&a.priority.get()));
        if candidates.is_empty() {
            continue;
        }
        let ray: RayLine = controller.ray(&world.g);
        let hit = room::pick(world, room, ray);
        let unit = room::unit(world, room);
        let mut started = false;
        for binding in candidates {
            let event = Event {
                now,
                delta: 0.0,
                controller,
                binding: &binding,
                panel: hit.map(|(panel, _)| panel),
                hit: hit.map(|(_, point)| point),
                room,
                unit,
            };
            let native = native_start(world, &event);
            let taken = match handler_hook(world, cx, Hook::Start, &event) {
                // A handler that answers nothing takes the drag; only `false` declines.
                Some(Ok(answer)) => !matches!(answer, Val::Bool(false)),
                Some(Err(fail)) => {
                    report(world, &binding, Hook::Start.label(), &fail.message);
                    continue;
                }
                None => native.is_some(),
            };
            let Some((panel, point)) = hit.filter(|_| taken) else {
                continue;
            };
            let was_pinned = room.is_pinned(panel.view);
            let run = Run {
                controller: Rc::clone(controller),
                binding: Rc::clone(&binding),
                panel,
                hit: point,
                last: now,
                was_pinned,
                state: native.unwrap_or(State::None),
            };
            // Whatever the binding does to the panel, the ring must not put it back a frame later.
            room.pin(panel.view);
            // The drag owns its buttons from here: the program stops passing them to the window under the ray. A claim that cannot be made
            // ends the drag at once, so a binding never runs half-claimed.
            let claimed = claims(world, cx, &binding, controller)
                .and_then(|inputs| controller.claim(&world.g, Owner::Binding(binding.obj.get()), inputs));
            match claimed {
                Ok(()) => {
                    world.rt.borrow_mut().runs.push(run);
                    started = true;
                    break;
                }
                Err(fail) => {
                    report(world, &binding, "claim", &fail.message);
                    finish(world, cx, run, Hook::Cancel, session, room, now);
                }
            }
        }
        if !started {
            world.rt.borrow_mut().declined.insert(controller.node);
        }
    }
    // A controller that left the session while it held a drag.
    let orphans: Vec<Run> = {
        let mut rt = world.rt.borrow_mut();
        let (keep, gone): (Vec<Run>, Vec<Run>) = rt.runs.drain(..).partition(|run| {
            controllers
                .iter()
                .any(|controller| Rc::ptr_eq(controller, &run.controller))
        });
        rt.runs = keep;
        gone
    };
    for run in orphans {
        finish(world, cx, run, Hook::Cancel, session, room, now);
    }
}

/// Whether a drag waits on the settle time, and how long until it is due, in milliseconds.
pub(crate) fn settle_due(world: &World, session: &SessionData) -> Option<f64> {
    let now = world.now();
    let rt = world.rt.borrow();
    session
        .controllers
        .borrow()
        .iter()
        .filter(|controller| {
            !controller.down.borrow().is_empty()
                && !rt.declined.contains(&controller.node)
                && !rt.runs.iter().any(|run| Rc::ptr_eq(&run.controller, controller))
        })
        .filter_map(|controller| {
            rt.since
                .get(&controller.node)
                .map(|(_, at)| at + session.settle.get() - now)
        })
        .filter(|left| *left > 0.0)
        .reduce(f64::min)
}

//! A std shell for the replay of the tiling fixtures through the object protocol: one workspace with gaps and a chord for each of the
//! library's layouts and each action of the dwindle, the strip and the carousel, written over the classes of `gnoblin_std::objects`
//! (`workspace`, the built-in layout classes, `client_state`) and not over the std table.
//!
//! It also binds the pointer drags each layout brings, through the classes of the policy catalog (`swap_binding` and `split_resize_binding`
//! on a tiling layout, `move_binding` and `resize_binding` on a floating one) installed on the core's input context, and the probe bindings
//! of the shell's own that the tiling script probes. The chords are those of the tiling script:
//!
//! | chord | what it does |
//! |---|---|
//! | `Super+1` | master and stack |
//! | `Super+2` to `Super+6`, `Super+minus` | dwindle with various options |
//! | `Super+7`, `Super+8` | strip |
//! | `Super+9`, `Super+0` | arc, carousel |
//! | `Super+equal` | master and stack, master on the right, two masters |
//! | `Super+space` | floating |
//! | `Super+g` | grid |
//! | `Super+m` | the first window's size limits (150 by 120 to 500 by 400) |
//! | `Super+F1` to `Super+F12` | the dwindle's actions |
//! | `Super+Left`, `Super+Right`, `Super+Up`, `Super+Down` | strip focus, strip scroll, carousel rotation |

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::NodeId;
use gnoblin_core::objects::{
    BindingEvent, BindingOptions, GestureBindingHandlers, ModelRoot, PointerDragBindingHandlers,
};
use gnoblin_std::objects::{
    ArcLayout, CarouselLayout, ClientState, DwindleDirection, DwindleForceSplit, DwindleLayout, DwindlePointer,
    DwindleSplit, DwindleSplitBias, FloatingLayout, GridLayout, Layout, MasterStackLayout, MasterStackOrientation,
    MoveBinding, Objects, ResizeBinding, SizeLimit, SplitResizeBinding, StripLayout, SwapBinding, Workspace,
    Workspaces,
};
use gnoblin_std::prelude::*;

const CHORDS: &[&str] = &[
    "Super+1",
    "Super+2",
    "Super+3",
    "Super+4",
    "Super+5",
    "Super+6",
    "Super+7",
    "Super+8",
    "Super+9",
    "Super+0",
    "Super+minus",
    "Super+equal",
    "Super+space",
    "Super+g",
    "Super+m",
    "Super+F1",
    "Super+F2",
    "Super+F3",
    "Super+F4",
    "Super+F5",
    "Super+F6",
    "Super+F7",
    "Super+F8",
    "Super+F9",
    "Super+F10",
    "Super+F11",
    "Super+F12",
    "Super+Left",
    "Super+Right",
    "Super+Up",
    "Super+Down",
];

/// The window the size limits are for: the first one.
const FIRST: &str = "40";

/// The answer of a call of a class, as the shell's own result type holds it.
fn done<T, E: Into<gnoblin_std::Error>>(result: Result<T, E>) -> gnoblin_std::Result<()> {
    result.map(drop).map_err(Into::into)
}

/// The layout the workspace has now, of the kinds whose actions a chord runs.
#[derive(Default)]
struct Current {
    dwindle: Option<DwindleLayout>,
    strip: Option<StripLayout>,
    carousel: Option<CarouselLayout>,
    /// Every layout the shell made stays alive for as long as it is the workspace's.
    _held: Option<Layout>,
}

/// Which drags are bound: a tile swap and a split resize on a tiling layout, a move and a resize on a floating one.
#[derive(Clone, Copy)]
enum DragSet {
    Tiling,
    Floating,
    Nothing,
}

/// The bindings the shell installed: the drags of one kind of layout.
#[derive(Default)]
struct DragBindings {
    moving: Option<MoveBinding>,
    resizing: Option<ResizeBinding>,
    swap: Option<SwapBinding>,
    split: Option<SplitResizeBinding>,
}

impl DragBindings {
    fn uninstall(self) -> gnoblin_std::Result<()> {
        if let Some(binding) = self.moving {
            binding.uninstall()?;
        }
        if let Some(binding) = self.resizing {
            binding.uninstall()?;
        }
        if let Some(binding) = self.swap {
            binding.uninstall()?;
        }
        if let Some(binding) = self.split {
            binding.uninstall()?;
        }
        Ok(())
    }
}

struct ObjectsShell {
    objects: Objects,
    core: gnoblin_core::objects::Objects,
    main: Workspace,
    current: RefCell<Current>,
    held: RefCell<DragBindings>,
}

impl ObjectsShell {
    /// Gives the workspace `layout`; the actions work on the layout it has.
    fn use_layout(&self, layout: Layout, current: Current) -> gnoblin_std::Result<()> {
        self.main.set_layout(&layout)?;
        *self.current.borrow_mut() = Current {
            _held: Some(layout),
            ..current
        };
        Ok(())
    }

    fn dwindle(&self, set: impl FnOnce(&DwindleLayout) -> gnoblin_std::Result<()>) -> gnoblin_std::Result<()> {
        let layout = DwindleLayout::new(&self.objects)?;
        set(&layout)?;
        self.use_layout(
            layout.as_layout()?,
            Current {
                dwindle: Some(layout),
                ..Current::default()
            },
        )
    }

    fn strip(&self, column: f64) -> gnoblin_std::Result<()> {
        let layout = StripLayout::new(&self.objects)?;
        layout.set_column(column)?;
        self.use_layout(
            layout.as_layout()?,
            Current {
                strip: Some(layout),
                ..Current::default()
            },
        )
    }

    /// Binds the pointer drags a layout brings: a tile swap and a split resize, a move and a resize, or none.
    fn bind(&self, drags: DragSet) -> gnoblin_std::Result<()> {
        std::mem::take(&mut *self.held.borrow_mut()).uninstall()?;
        let objects = &self.objects;
        let mut held = DragBindings::default();
        match drags {
            DragSet::Tiling => {
                let swap = SwapBinding::new(objects)?;
                swap.install(None)?;
                let split = SplitResizeBinding::new(objects)?;
                split.install(None)?;
                held.swap = Some(swap);
                held.split = Some(split);
            }
            DragSet::Floating => {
                let moving = MoveBinding::new(objects)?;
                moving.install(None)?;
                let resizing = ResizeBinding::new(objects)?;
                resizing.install(None)?;
                held.moving = Some(moving);
                held.resizing = Some(resizing);
            }
            DragSet::Nothing => {}
        }
        *self.held.borrow_mut() = held;
        Ok(())
    }

    /// Runs a chord, then binds the drags its layout brings.
    fn run(&self, chord: &str) -> gnoblin_std::Result<()> {
        self.run_chord(chord)?;
        match chord {
            "Super+1" | "Super+2" | "Super+3" | "Super+4" | "Super+5" | "Super+6" | "Super+minus" | "Super+equal"
            | "Super+g" => self.bind(DragSet::Tiling),
            "Super+7" | "Super+8" | "Super+9" | "Super+0" => self.bind(DragSet::Nothing),
            "Super+space" => self.bind(DragSet::Floating),
            _ => Ok(()),
        }
    }

    fn run_chord(&self, chord: &str) -> gnoblin_std::Result<()> {
        // The layout's actions work on the layout the workspace has: a chord of a kind it does not have does nothing.
        let current = self.current.borrow();
        let dwindle = current.dwindle.as_ref();
        let strip = current.strip.as_ref();
        match chord {
            "Super+1" => {
                drop(current);
                let layout = MasterStackLayout::new(&self.objects)?;
                self.use_layout(layout.as_layout()?, Current::default())
            }
            "Super+2" => {
                drop(current);
                self.dwindle(|_| Ok(()))
            }
            "Super+3" => {
                drop(current);
                self.dwindle(|layout| {
                    layout.set_preserve_split(true)?;
                    layout.set_ratio(0.6)?;
                    done(layout.set_gap(4.0))
                })
            }
            "Super+4" => {
                drop(current);
                self.dwindle(|layout| done(layout.set_smart_resizing(false)))
            }
            "Super+5" => {
                drop(current);
                self.dwindle(|layout| {
                    layout.set_smart_split(true)?;
                    layout.set_pointer(DwindlePointer::Fixed)?;
                    layout.set_pointer_x(700.0)?;
                    done(layout.set_pointer_y(100.0))
                })
            }
            "Super+6" => {
                drop(current);
                self.dwindle(|layout| {
                    layout.set_use_active_for_splits(false)?;
                    layout.set_force_split(DwindleForceSplit::First)?;
                    layout.set_split_bias(DwindleSplitBias::Opened)?;
                    layout.set_pointer(DwindlePointer::Fixed)?;
                    layout.set_pointer_x(100.0)?;
                    done(layout.set_pointer_y(700.0))
                })
            }
            "Super+minus" => {
                drop(current);
                self.dwindle(|layout| {
                    layout.set_split(DwindleSplit::Vertical)?;
                    done(layout.set_permanent_direction_override(true))
                })
            }
            "Super+7" => {
                drop(current);
                self.strip(0.5)
            }
            "Super+8" => {
                drop(current);
                self.strip(0.4)
            }
            "Super+9" => {
                drop(current);
                let layout = ArcLayout::new(&self.objects)?;
                self.use_layout(layout.as_layout()?, Current::default())
            }
            "Super+0" => {
                drop(current);
                let layout = CarouselLayout::new(&self.objects)?;
                self.use_layout(
                    layout.as_layout()?,
                    Current {
                        carousel: Some(layout),
                        ..Current::default()
                    },
                )
            }
            "Super+equal" => {
                drop(current);
                let layout = MasterStackLayout::new(&self.objects)?;
                layout.set_ratio(0.6)?;
                layout.set_master_count(2)?;
                layout.set_orientation(MasterStackOrientation::Right)?;
                self.use_layout(layout.as_layout()?, Current::default())
            }
            "Super+space" => {
                drop(current);
                let layout = FloatingLayout::new(&self.objects)?;
                self.use_layout(layout.as_layout()?, Current::default())
            }
            "Super+g" => {
                drop(current);
                let layout = GridLayout::new(&self.objects)?;
                self.use_layout(layout.as_layout()?, Current::default())
            }
            "Super+m" => {
                // The first window may not be resized below 150 by 120 or above 500 by 400.
                // The window is the core's `Client`, which the core looks up by its id.
                let first = gnoblin_core::objects::ModelRoot::get(&self.core)?
                    .clients()?
                    .get_by_id(FIRST)
                    .ok()
                    .flatten();
                let Some(first) = first else {
                    return Ok(());
                };
                let state = ClientState::of(&self.objects, &first)?;
                state.set_min_size(&SizeLimit {
                    width: 150.0,
                    height: 120.0,
                })?;
                done(state.set_max_size(&SizeLimit {
                    width: 500.0,
                    height: 400.0,
                }))
            }
            "Super+F1" => dwindle.map_or(Ok(()), |d| done(d.toggle_split())),
            "Super+F2" => dwindle.map_or(Ok(()), |d| done(d.swap_split())),
            "Super+F3" => dwindle.map_or(Ok(()), |d| done(d.rotate_split(90.0))),
            "Super+F4" => dwindle.map_or(Ok(()), |d| done(d.rotate_split(-90.0))),
            "Super+F5" => dwindle.map_or(Ok(()), |d| done(d.preselect(DwindleDirection::Right))),
            "Super+F6" => dwindle.map_or(Ok(()), |d| done(d.preselect(DwindleDirection::Down))),
            "Super+F7" => dwindle.map_or(Ok(()), |d| done(d.preselect(DwindleDirection::Left))),
            "Super+F8" => dwindle.map_or(Ok(()), |d| done(d.move_to_root(true))),
            "Super+F9" => dwindle.map_or(Ok(()), |d| done(d.move_to_root(false))),
            "Super+F10" => dwindle.map_or(Ok(()), |d| done(d.split_ratio(0.1))),
            "Super+F11" => dwindle.map_or(Ok(()), |d| done(d.set_split_ratio(0.3))),
            "Super+F12" => dwindle.map_or(Ok(()), |d| done(d.pseudotile(None, None))),
            "Super+Left" => strip.map_or(Ok(()), |s| done(s.focus_left())),
            "Super+Right" => strip.map_or(Ok(()), |s| done(s.focus_right())),
            "Super+Up" => strip.map_or(Ok(()), |s| done(s.scroll_to(Some(300.0)))),
            "Super+Down" => current.carousel.as_ref().map_or(Ok(()), |c| done(c.rotate(1))),
            other => {
                eprintln!("unknown chord {other}");
                Ok(())
            }
        }
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let main = Workspace::new(&objects, "main")?;
    main.set_gaps(&gnoblin_std::objects::Gaps {
        inner: 6.0,
        outer: 12.0,
    })?;
    let layout = MasterStackLayout::new(&objects)?;
    main.set_layout(&layout.as_layout()?)?;
    Workspaces::get(&objects)?.add(&main)?;
    let shell = Rc::new(ObjectsShell {
        core: g.objects()?,
        objects,
        main,
        current: RefCell::new(Current {
            _held: Some(layout.as_layout()?),
            ..Current::default()
        }),
        held: RefCell::new(DragBindings::default()),
    });
    shell.bind(DragSet::Tiling)?;
    probes(g, &shell.objects)?;
    for chord in CHORDS {
        let shell = Rc::clone(&shell);
        g.bind(chord, move || {
            if let Err(error) = shell.run(chord) {
                g.log(&format!("cannot run {chord}: {error}"));
            }
        })?;
    }
    g.log("objects shell is running");
    Ok(())
}

/// The window the probes write to: the first one.
const FIRST_NODE: u64 = 40;

fn number(g: &Gnoblin, id: u64, key: &str) -> f64 {
    g.node(NodeId(id)).get_f64(key).unwrap_or(0.0)
}

fn put(g: &Gnoblin, id: u64, key: &str, value: f64) {
    let _ = g.node(NodeId(id)).set(key, value);
}

fn bump(g: &Gnoblin, id: u64, key: &str, by: f64) {
    put(g, id, key, number(g, id, key) + by);
}

/// What a gesture probe writes at each step, by the properties it uses: `starts` counts the starts, `x` follows the progress, `y` takes
/// what the end and the cancel add.
#[derive(Clone, Copy)]
struct Counter {
    trigger: &'static str,
    starts: &'static str,
    sign: f64,
    offset: f64,
    end_by: f64,
    cancel_by: f64,
}

/// The probes of the bindings of the shell's own: three gestures, and two drags of `Super+MouseMiddle` that the matcher chooses between
/// by priority and by decline. `A` has the higher priority and declines a window titled `B`; `B` takes what is left. `A` moves the window it
/// pressed along x, `B` along y.
fn probes(g: &Gnoblin, objects: &Objects) -> gnoblin_std::Result<()> {
    let core = g.objects()?;
    let _ = objects;
    let model = ModelRoot::get(&core)?;
    let context = model.input()?.primary().ok();
    let Some(context) = context else {
        return Ok(());
    };
    let gestures = [
        Counter {
            trigger: "Swipe3Left",
            starts: "width",
            sign: 1.0,
            offset: 0.0,
            end_by: 10.0,
            cancel_by: 1000.0,
        },
        Counter {
            trigger: "Swipe3Right",
            starts: "height",
            sign: -1.0,
            offset: 5000.0,
            end_by: 20.0,
            cancel_by: 2000.0,
        },
        Counter {
            trigger: "Swipe4",
            starts: "width",
            sign: 1.0,
            offset: 10000.0,
            end_by: 30.0,
            cancel_by: 3000.0,
        },
    ];
    for probe in gestures {
        let follow = move |g: &Gnoblin, event: &BindingEvent| {
            put(
                g,
                FIRST_NODE,
                "x",
                probe.offset + probe.sign * (event.progress.unwrap_or(0.0) * 1000.0).round(),
            );
        };
        let gc = g.clone();
        let (gu, ge, gk) = (g.clone(), g.clone(), g.clone());
        let handlers = GestureBindingHandlers {
            on_begin: Box::new(move |_| {
                bump(&gc, FIRST_NODE, probe.starts, 1.0);
                Ok(true)
            }),
            on_update: Some(Box::new(move |event| {
                follow(&gu, &event);
                Ok(())
            })),
            on_end: Some(Box::new(move |event| {
                follow(&ge, &event);
                bump(&ge, FIRST_NODE, "y", probe.end_by);
                Ok(())
            })),
            on_cancel: Some(Box::new(move |_| {
                bump(&gk, FIRST_NODE, "y", probe.cancel_by);
                Ok(())
            })),
        };
        let handler = handlers.implement(&core)?;
        context.bind_gesture(probe.trigger, &handler, None)?;
    }
    for label in ['B', 'A'] {
        let (gu, ge) = (g.clone(), g.clone());
        let handlers = PointerDragBindingHandlers {
            on_start: Box::new(move |event| {
                let Some(client) = &event.client else {
                    return Ok(false);
                };
                let title = client.title().unwrap_or_default();
                Ok(label == 'B' || title != "B")
            }),
            on_update: Some(Box::new(move |event| {
                if let Some(client) = &event.client
                    && let Ok(id) = client.id().unwrap_or_default().parse::<u64>()
                {
                    let (key, value) = if label == 'A' {
                        ("x", event.position.x)
                    } else {
                        ("y", event.position.y)
                    };
                    put(&gu, id, key, value);
                }
                Ok(())
            })),
            on_end: Some(Box::new(move |event| {
                if let Some(client) = &event.client
                    && let Ok(id) = client.id().unwrap_or_default().parse::<u64>()
                {
                    bump(&ge, id, "width", 1.0);
                }
                Ok(())
            })),
            on_cancel: None,
        };
        let handler = handlers.implement(&core)?;
        let options = BindingOptions {
            priority: Some(if label == 'A' { 5.0 } else { 0.0 }),
            ..BindingOptions::default()
        };
        context.bind_drag("Super+MouseMiddle+Drag", &handler, Some(&options))?;
    }
    Ok(())
}

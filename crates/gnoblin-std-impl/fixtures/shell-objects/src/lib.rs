//! A std shell for the replay of the tiling fixtures through the object protocol: one workspace with gaps and a chord for each of the
//! library's layouts and each action of the dwindle, the strip and the carousel, written over the classes of `gnoblin_std::objects`
//! (`workspace`, the built-in layout classes, `client_state`) and not over the std table.
//!
//! `shell-tiling` has the same chords over the table, with the pointer drags each layout brings; this shell has no drag bindings (a
//! binding is the policy lane's class), so the scripts that need one are not replayed (`tests/objects.rs` lists them). The chords are
//! those of `shell-tiling`:
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

use gnoblin_std::objects::{
    ArcLayout, CarouselLayout, ClientState, DwindleDirection, DwindleForceSplit, DwindleLayout, DwindlePointer,
    DwindleSplit, DwindleSplitBias, FloatingLayout, GridLayout, Layout, MasterStackLayout, MasterStackOrientation,
    Objects, SizeLimit, StripLayout, Workspace, Workspaces,
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

struct ObjectsShell {
    objects: Objects,
    core: gnoblin_core::objects::Objects,
    main: Workspace,
    current: RefCell<Current>,
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

    fn run(&self, chord: &str) -> gnoblin_std::Result<()> {
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
                    .and_then(|clients| clients.get_by_id(FIRST).ok().flatten());
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
    });
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

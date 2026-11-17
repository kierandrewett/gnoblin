//! A std shell for the tiling tests: one workspace with gaps and a chord for each of the library's layouts and for each action of the
//! dwindle, the strip and the carousel, and the pointer drags each kind of layout brings (a tile swap and a split resize for the tiling
//! layouts, a move and a resize for the floating one).
//!
//! The TypeScript library does the same through `layout.bindings`, which are active while their layout is; this shell binds the drags
//! when it switches layout and lets the other kind go. `crates/gnoblin-std-impl/tests/tiling.rs` holds a TypeScript shell with the same
//! chords, and the two are driven by one script.
//!
//! | chord | what it does |
//! |---|---|
//! | `Super+1` | master and stack |
//! | `Super+2` to `Super+6`, `Super+minus` | dwindle with various options |
//! | `Super+7`, `Super+8` | strip |
//! | `Super+9`, `Super+0` | arc, carousel |
//! | `Super+equal` | master and stack, master on the right, two masters |
//! | `Super+space` | floating |
//! | `Super+m` | the first window's size limits (150 by 120 to 500 by 400) |
//! | `Super+F1` to `Super+F12` | the dwindle's actions |
//! | `Super+Left`, `Super+Right`, `Super+Up`, `Super+Down` | strip focus, strip scroll, carousel rotation |

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core::NodeId;
use gnoblin_std::prelude::*;
use gnoblin_std::{
    ArcLayout, Carousel3dLayout, DwindleOptions, ForceSplit, Orientation, Point, PointerSource, Preselect, SplitBias,
    SplitMode, StripLayout, Subscription,
};
use gnoblin_std::{GestureBinding, GestureInfo};

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

/// Which drags are bound.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drags {
    /// A tile swap on `Super+Button1` and a split resize on `Super+Button3`.
    Tiling,
    /// A move on `Super+Button1` and a resize on `Super+Button3`.
    Floating,
    /// None.
    Nothing,
}

struct TilingShell {
    std: Std,
    main: Workspace,
    drags: RefCell<Vec<Subscription>>,
    bound: Cell<Drags>,
}

fn pointer(x: f64, y: f64) -> PointerSource {
    PointerSource::Fixed(Point { x, y })
}

impl TilingShell {
    fn bind(&self, drags: Drags) -> gnoblin_std::Result<()> {
        if self.bound.get() == drags {
            return Ok(());
        }
        for drag in self.drags.borrow_mut().drain(..) {
            drag.unsubscribe()?;
        }
        let mut held = Vec::new();
        match drags {
            Drags::Tiling => {
                held.push(self.std.bind_drag(ClientSwapBinding::new())?);
                held.push(self.std.bind_drag(SplitResizeBinding::new())?);
            }
            Drags::Floating => {
                held.push(self.std.bind_drag(ClientMoveBinding::new())?);
                held.push(self.std.bind_drag(ClientResizeBinding::new())?);
            }
            Drags::Nothing => {}
        }
        *self.drags.borrow_mut() = held;
        self.bound.set(drags);
        Ok(())
    }

    fn dwindle(&self, options: DwindleOptions) -> gnoblin_std::Result<()> {
        self.main.set_layout(DwindleLayout::with_options(options)?)?;
        self.bind(Drags::Tiling)
    }

    fn strip(&self, column: f64) -> gnoblin_std::Result<()> {
        self.main.set_layout(StripLayout::with_column(column)?)?;
        self.bind(Drags::Nothing)
    }

    fn run(&self, chord: &str) -> gnoblin_std::Result<()> {
        // The layout's actions work on the workspace's own layout: a chord of a kind the workspace does not have does nothing.
        let main = &self.main;
        let dwindle = DwindleLayout::is_active(main)?.then(DwindleLayout::new);
        let strip = StripLayout::is_active(main)?.then(StripLayout::new);
        match chord {
            "Super+1" => {
                main.set_layout(MasterStackLayout::new())?;
                self.bind(Drags::Tiling)
            }
            "Super+2" => self.dwindle(DwindleOptions::default()),
            "Super+3" => self.dwindle(DwindleOptions {
                preserve_split: true,
                ratio: 0.6,
                gap: Some(4.0),
                ..DwindleOptions::default()
            }),
            "Super+4" => self.dwindle(DwindleOptions {
                smart_resizing: false,
                ..DwindleOptions::default()
            }),
            "Super+5" => self.dwindle(DwindleOptions {
                smart_split: true,
                pointer: pointer(700.0, 100.0),
                ..DwindleOptions::default()
            }),
            "Super+6" => self.dwindle(DwindleOptions {
                use_active_for_splits: false,
                force_split: ForceSplit::First,
                split_bias: SplitBias::Opened,
                pointer: pointer(100.0, 700.0),
                ..DwindleOptions::default()
            }),
            "Super+minus" => self.dwindle(DwindleOptions {
                split: SplitMode::Vertical,
                permanent_direction_override: true,
                ..DwindleOptions::default()
            }),
            "Super+7" => self.strip(0.5),
            "Super+8" => self.strip(0.4),
            "Super+9" => {
                main.set_layout(ArcLayout::new())?;
                self.bind(Drags::Nothing)
            }
            "Super+0" => {
                main.set_layout(Carousel3dLayout::new())?;
                self.bind(Drags::Nothing)
            }
            "Super+equal" => {
                main.set_layout(
                    MasterStackLayout::new()
                        .ratio(0.6)
                        .master_count(2)
                        .orientation(Orientation::Right),
                )?;
                self.bind(Drags::Tiling)
            }
            "Super+space" => {
                main.set_layout(FloatingLayout::new())?;
                self.bind(Drags::Floating)
            }
            "Super+m" => {
                // The first window may not be resized below 150 by 120 or above 500 by 400.
                for client in self.std.clients()?.to_vec()? {
                    if client.window().id().0 == FIRST {
                        client.set_min_size(Some((150.0, 120.0)))?;
                        client.set_max_size(Some((500.0, 400.0)))?;
                    }
                }
                Ok(())
            }
            "Super+g" => {
                main.set_layout(GridLayout::new())?;
                self.bind(Drags::Tiling)
            }
            "Super+F1" => dwindle.map_or(Ok(()), |d| d.toggle_split(main).map(drop)),
            "Super+F2" => dwindle.map_or(Ok(()), |d| d.swap_split(main).map(drop)),
            "Super+F3" => dwindle.map_or(Ok(()), |d| d.rotate_split(main, 90.0).map(drop)),
            "Super+F4" => dwindle.map_or(Ok(()), |d| d.rotate_split(main, -90.0).map(drop)),
            "Super+F5" => dwindle.map_or(Ok(()), |d| d.preselect(main, Preselect::Right)),
            "Super+F6" => dwindle.map_or(Ok(()), |d| d.preselect(main, Preselect::Down)),
            "Super+F7" => dwindle.map_or(Ok(()), |d| d.preselect(main, Preselect::Left)),
            "Super+F8" => dwindle.map_or(Ok(()), |d| d.move_to_root(main, true).map(drop)),
            "Super+F9" => dwindle.map_or(Ok(()), |d| d.move_to_root(main, false).map(drop)),
            "Super+F10" => dwindle.map_or(Ok(()), |d| d.split_ratio(main, 0.1).map(drop)),
            "Super+F11" => dwindle.map_or(Ok(()), |d| d.set_split_ratio(main, 0.3).map(drop)),
            "Super+F12" => dwindle.map_or(Ok(()), |d| d.pseudotile(main, None).map(drop)),
            "Super+Left" => strip.map_or(Ok(()), |s| s.focus_left(main)),
            "Super+Right" => strip.map_or(Ok(()), |s| s.focus_right(main)),
            "Super+Up" => strip.map_or(Ok(()), |s| s.scroll_to(main, 300.0)),
            "Super+Down" => Carousel3dLayout::new().rotate(main, 1),
            other => {
                self.std.gnoblin().log(&format!("unknown chord {other}"));
                Ok(())
            }
        }
    }
}

// ---- probes: bindings of the shell's own, for the binding scenarios ----------------------------------------------------------
//
// The library runs the matcher; these bindings only make what it decided visible. Each writes numbers into a window's rectangle (a
// floating workspace leaves them alone), so a script's `step` shows which binding was asked, in what order and how it ended. The
// TypeScript shell of `tests/tiling.rs` has the same bindings.

/// The window the gesture probes write to: the first one.
const FIRST: u64 = 40;

fn number(g: &Gnoblin, id: u64, key: &str) -> f64 {
    g.node(NodeId(id)).get_f64(key).unwrap_or(0.0)
}

fn put(g: &Gnoblin, id: u64, key: &str, value: f64) {
    let _ = g.node(NodeId(id)).set(key, value);
}

fn bump(g: &Gnoblin, id: u64, key: &str, by: f64) {
    put(g, id, key, number(g, id, key) + by);
}

/// What a gesture probe writes on each step, by the properties it uses: `starts` counts the starts, `x` follows the progress.
struct GestureProbe {
    g: Gnoblin,
    trigger: &'static str,
    starts: &'static str,
    /// The sign and offset of the progress it writes.
    sign: f64,
    offset: f64,
    end_by: f64,
    cancel_by: f64,
}

impl GestureProbe {
    fn follow(&self, event: &GestureInfo) {
        put(
            &self.g,
            FIRST,
            "x",
            self.offset + self.sign * (event.progress * 1000.0).round(),
        );
    }
}

impl GestureBinding for GestureProbe {
    fn trigger(&self) -> String {
        self.trigger.to_owned()
    }

    fn on_start(&mut self, _event: &GestureInfo) -> bool {
        bump(&self.g, FIRST, self.starts, 1.0);
        true
    }

    fn on_update(&mut self, event: &GestureInfo) {
        self.follow(event);
    }

    fn on_end(&mut self, event: &GestureInfo) {
        self.follow(event);
        bump(&self.g, FIRST, "y", self.end_by);
    }

    fn on_cancel(&mut self, _event: &GestureInfo) {
        bump(&self.g, FIRST, "y", self.cancel_by);
    }
}

/// A drag probe of `Super+Button2`. `A` has the higher priority and declines a window titled `B`; `B` takes what is left. `A` moves the
/// window it pressed along x, `B` along y.
struct DragProbe {
    g: Gnoblin,
    label: char,
}

impl DragBinding for DragProbe {
    fn trigger(&self) -> String {
        "Super+Button2".to_owned()
    }

    fn priority(&self) -> f64 {
        if self.label == 'A' { 5.0 } else { 0.0 }
    }

    fn on_start(&mut self, event: &DragEvent) -> bool {
        let Some(client) = &event.client else {
            return false;
        };
        self.label == 'B' || client.window().title().unwrap_or_default() != "B"
    }

    fn on_update(&mut self, event: &DragEvent) {
        let Some(client) = &event.client else {
            return;
        };
        let (key, value) = if self.label == 'A' {
            ("x", event.position.x)
        } else {
            ("y", event.position.y)
        };
        put(&self.g, client.window().id().0, key, value);
    }

    fn on_end(&mut self, event: &DragEvent) {
        if let Some(client) = &event.client {
            bump(&self.g, client.window().id().0, "width", 1.0);
        }
    }
}

fn bind_probes(std: &Std, g: &Gnoblin) -> gnoblin_std::Result<()> {
    let gesture = |trigger, starts, sign, offset, end_by, cancel_by| GestureProbe {
        g: g.clone(),
        trigger,
        starts,
        sign,
        offset,
        end_by,
        cancel_by,
    };
    std.bind_gesture(gesture("Swipe3Left", "width", 1.0, 0.0, 10.0, 1000.0))?;
    std.bind_gesture(gesture("Swipe3Right", "height", -1.0, 5000.0, 20.0, 2000.0))?;
    // No direction, four fingers: the gesture a window closes under.
    std.bind_gesture(gesture("Swipe4", "width", 1.0, 10000.0, 30.0, 3000.0))?;
    // Two bindings of one chord: the higher priority is asked first, and one that declines leaves the drag to the other.
    std.bind_drag(DragProbe {
        g: g.clone(),
        label: 'B',
    })?;
    std.bind_drag(DragProbe {
        g: g.clone(),
        label: 'A',
    })?;
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    let main = Workspace::builder("main")?
        .layout(MasterStackLayout::new())
        .gaps(6.0, 12.0)
        .add()?;
    let shell = Rc::new(TilingShell {
        std,
        main,
        drags: RefCell::new(Vec::new()),
        bound: Cell::new(Drags::Nothing),
    });
    shell.bind(Drags::Tiling)?;
    bind_probes(&shell.std, g)?;
    // A second binding of one chord at one priority is refused: the matcher could not choose.
    match shell.std.bind_drag(DragProbe {
        g: g.clone(),
        label: 'A',
    }) {
        Err(error) => g.log(&format!("duplicate binding refused: {error}")),
        Ok(_) => g.log("duplicate binding accepted"),
    }
    for chord in CHORDS {
        let shell = Rc::clone(&shell);
        g.bind(chord, move || {
            if let Err(error) = shell.run(chord) {
                g.log(&format!("cannot run {chord}: {error}"));
            }
        })?;
    }
    g.log("tiling shell is running");
    Ok(())
}

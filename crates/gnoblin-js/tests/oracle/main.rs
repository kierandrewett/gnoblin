//! The oracle: the TypeScript std and core, run over the fake compositor and recorded as fixtures for the native std port.
//!
//! `design/native-std-port.md` (7.1, lane NpOracle) names the TypeScript library as the only specification of the toolkit, the window
//! frame, the menus, style, scene and input behaviour. Each scenario below runs it and holds what it did:
//!
//! - `crates/gnoblin-std-impl/tests/fixtures/reference/*.json`: the transactions it sent for each step, its log, and snapshots of the
//!   tree, for the desktop (toolkit, frame, menus, scene, style, shell parts);
//! - `crates/gnoblin-js/tests/fixtures/paint/*.bin`: the paint lists of the widgets in each state, byte for byte;
//! - `crates/gnoblin-core-impl/tests/fixtures/model/*.json`: the order of listener calls and the input matcher's decisions.
//!
//! `cargo xtask oracle record` writes them, `cargo xtask oracle check` proves they are what the TypeScript does now, and each native lane
//! reads the same files through `gnoblin_testkit::oracle::Fixtures`. The `README.md` of each fixture directory says how a file is laid
//! out. A scenario never reads a clock, the environment or an address, so the same run on another machine writes the same bytes.

#[path = "../common/mod.rs"]
mod common;
mod frame;
mod input;
mod menus;
mod model;
mod parts;
mod scene;
mod scn;
mod style;
mod ui;

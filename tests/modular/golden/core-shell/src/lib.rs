//! Golden shell of example 12, core only (`design/modular-framework.md` 5.8 and 12).
//!
//! It is built once, against ABI 1.0, and stored as a binary. Every later core runs that file unchanged, so the one thing it
//! says about itself matters: at start it logs the ABI it was built for and the ABI of the core it finds installed, which is
//! how a test sees that a core of a later minor or patch is the one serving it. It then does what a small shell does: it gives
//! each window a place in a cascade, so the test sees that it runs and not only that it loaded.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{Gnoblin, Rect, Window};

/// Where the `n`th window goes: a cascade that wraps after eight.
fn cascade(n: usize) -> Rect {
    let step = 40.0 * (n % 8) as f64;
    Rect::new(60.0 + step, 60.0 + step, 800.0, 600.0)
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let (major, minor, patch) = gnoblin::installed_abi();
    g.log(&format!(
        "golden-core-shell: built for ABI {}.{}; installed core ABI {major}.{minor}.{patch}",
        gnoblin::ABI_MAJOR,
        gnoblin::ABI_MINOR
    ));

    let placed = Rc::new(Cell::new(0usize));
    let place = {
        let placed = Rc::clone(&placed);
        move |window: &Window| {
            let n = placed.get();
            placed.set(n + 1);
            if let Err(error) = window.set_geometry(cascade(n)) {
                window.gnoblin().log(&format!(
                    "golden-core-shell: cannot place window {}: {error}",
                    window.id().0
                ));
            }
        }
    };
    for window in g.windows()? {
        place(&window);
    }
    g.on_client_created(place)?;

    g.log("golden-core-shell: running");
    Ok(())
}

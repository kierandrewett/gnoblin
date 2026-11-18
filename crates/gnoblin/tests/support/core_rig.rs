//! The JavaScript shell of `tests/js_shell.rs` over a core with the std library loaded, in the test's own process.
//!
//! The stock layouts of `gnoblin-std` are the library's (`builtin_*`), and the library is a guest of a core generation. The shell's link to
//! the real compositor is handed to the core, which is then the one holder of it, and the engine drives a view of the core's tree
//! (`gnoblin_runtime_js::embed`). A tick of the shell is the engine's frame and then the core's dispatch, in which the library runs what is
//! dirty. The core's registry is one per process, so the rigs of the tests of a binary take turns: [`Turn`] is the place in the queue.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_js::JsShell;
use gnoblin_runtime_js::embed::Embedded;
use gnoblin_shell_host::supervisor::Clock;
use gnoblin_std_dyn::StdLib;
use gnoblin_tree_client::tree_mirror::{LinkError, TreeLink};

static BUSY: Mutex<bool> = Mutex::new(false);
static FREE: Condvar = Condvar::new();

thread_local! {
    static HOLDS: Cell<bool> = const { Cell::new(false) };
}

/// The right to run a core in this process, held from the moment a test asks for it until the rig is gone.
pub struct Turn(());

impl Turn {
    /// Waits for the core to be free. A test that asks again while its first rig is alive would wait for itself.
    pub fn wait() -> Turn {
        assert!(
            !HOLDS.with(Cell::get),
            "a second shell over a core while this test's first is alive: stop the first (`shell.finish()`) before the second"
        );
        let mut busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
        while *busy {
            busy = FREE.wait(busy).unwrap_or_else(PoisonError::into_inner);
        }
        *busy = true;
        HOLDS.with(|holds| holds.set(true));
        Turn(())
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        HOLDS.with(|holds| holds.set(false));
        *BUSY.lock().unwrap_or_else(PoisonError::into_inner) = false;
        FREE.notify_one();
    }
}

/// Hands out the link the role made, once.
struct Handed(RefCell<Option<Box<dyn TreeLink>>>);

struct Provider(Rc<Handed>);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        self.0
            .0
            .borrow_mut()
            .take()
            .ok_or_else(|| LinkError("the core opened the shell's link twice".to_owned()))
    }
}

/// A core generation with the std library, over the link a role makes.
pub struct CoreRig {
    embedded: Embedded,
    ctx: GnoblinCtx,
    handed: Rc<Handed>,
}

impl CoreRig {
    /// A fresh core whose timers read `clock`, with the std library installed. The compositor's link comes later (`take_over`).
    ///
    /// # Errors
    ///
    /// The core cannot open the generation, or the library is not of the core's ABI.
    pub fn new(clock: Arc<dyn Clock>) -> Result<CoreRig, String> {
        let handed = Rc::new(Handed(RefCell::new(None)));
        host::reset_session();
        host::set_tree_link_provider(Box::new(Provider(Rc::clone(&handed))));
        host::set_clock(clock);
        let ctx = host::open_generation("js-role", gnoblin_core_abi::LANG_TS).map_err(|error| format!("{error:?}"))?;
        let embedded = Embedded::new(host::api(), ctx);
        embedded.install_std(
            StdLib::from_entry_points(
                gnoblin_std_impl::gnoblin_std_get_api,
                gnoblin_std_impl::gnoblin_std_last_error,
            )
            .map_err(|error| format!("{error:?}"))?,
        );
        Ok(CoreRig { embedded, ctx, handed })
    }

    /// Gives the core the compositor's link the role made, and returns the view of the core's tree the engine is to drive.
    ///
    /// # Errors
    ///
    /// The core cannot open the link.
    pub fn take_over(&self, link: Box<dyn TreeLink>) -> Result<Box<dyn TreeLink>, String> {
        *self.handed.0.borrow_mut() = Some(link);
        self.embedded.link()
    }

    /// `__gnoblinStd`, for every generation of the shell.
    pub fn std_native(&self) -> gnoblin_js::JsNative {
        self.embedded.std_native()
    }

    /// The core's half of a frame: what the library announces and runs, and what it writes.
    pub fn dispatch(&self, shell: &JsShell) -> i32 {
        self.embedded.dispatch(shell)
    }
}

impl Drop for CoreRig {
    fn drop(&mut self) {
        host::close_generation(self.ctx);
        host::reset_session();
    }
}

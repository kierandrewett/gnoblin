//! The shell over the real core and the real std library, in the test's own process (design/modular-framework.md, 25.3, B1).
//!
//! The scenarios of this crate's tests drive a `JsShell` over the fake compositor. A shell that uses the desktop library
//! (`gnoblin-std`) needs more than the fake: `__gnoblinStd` calls `libgnoblin-std`, which is a guest of the core and works on the
//! core's tree. This module builds that, so the same scenario, with its assertions unchanged, runs through a core generation with std
//! loaded, as it does in the compositor:
//!
//! - the core is `gnoblin-core-impl` linked into the test binary (a development edge `impl-sealed` allows), with the fake compositor as
//!   the link its host gives the shell thread;
//! - the std library is `gnoblin-std-impl`, linked the same way and handed to the runtime as if it had been opened by name;
//! - the engine's link is the runtime's relay (`gnoblin_runtime_js::embed`): the engine's mirror is a view of the core's, and the
//!   compositor sees one holder;
//! - a frame is the engine's frame and then the core's dispatch, in which the library announces what happened and runs what is dirty
//!   (the layout of a workspace, the writes of a transition), and its callbacks enter the shell as they do in the runtime's turn.
//!
//! The core's registry is one per process, so a [`NativeCore`] holds a process-wide lock for as long as it lives.
#![allow(dead_code)]

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_runtime_js::embed::Embedded;
use gnoblin_shell_host::supervisor::Clock;
use gnoblin_std_dyn::StdLib;
use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_tree_client::tree_mirror::{LinkError, TreeLink};

/// What the core opens when the shell attaches: a clone of the fake compositor.
struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

fn process_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A core generation with the std library loaded, over a fake compositor.
pub struct NativeCore {
    embedded: Embedded,
    ctx: GnoblinCtx,
    _lock: MutexGuard<'static, ()>,
}

impl NativeCore {
    /// A core over `comp`, whose timers read `clock`.
    pub fn new(comp: &FakeCompositor, clock: Arc<dyn Clock>) -> NativeCore {
        let lock = process_lock();
        host::reset_session();
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        host::set_clock(clock);
        let ctx = host::open_generation("js-rig", gnoblin_core_abi::LANG_TS).expect("a generation opens");
        let embedded = Embedded::new(host::api(), ctx);
        embedded.install_std(
            StdLib::from_entry_points(
                gnoblin_std_impl::gnoblin_std_get_api,
                gnoblin_std_impl::gnoblin_std_last_error,
            )
            .expect("the std library is of this ABI"),
        );
        NativeCore {
            embedded,
            ctx,
            _lock: lock,
        }
    }

    /// The engine's link: a view of the core's tree. Boxed, to be the `link` of `JsShell::new`.
    pub fn link(&self) -> Box<dyn TreeLink> {
        self.embedded.link().expect("the core opens the link")
    }

    /// Gives every generation of a shell `__gnoblinStd`.
    pub fn configure(&self, config: &mut JsShellConfig) {
        config.js.natives.push(self.embedded.std_native());
    }

    /// The core's half of a frame: what the library announces and runs, and what it writes.
    pub fn dispatch(&self, shell: &JsShell) -> i32 {
        self.embedded.dispatch(shell)
    }

    /// The generation the shell runs as.
    pub fn ctx(&self) -> GnoblinCtx {
        self.ctx
    }
}

impl Drop for NativeCore {
    fn drop(&mut self) {
        host::close_generation(self.ctx);
        host::reset_session();
    }
}

/// A shell over `comp` and a core with std loaded: `config` gets `__gnoblinStd`, the shell's link is the view of the core's tree.
///
/// # Errors
///
/// The shell does not boot.
pub fn shell_over_core(
    comp: &FakeCompositor,
    clock: Arc<dyn Clock>,
    mut config: JsShellConfig,
) -> (JsShell, NativeCore) {
    let core = NativeCore::new(comp, clock);
    core.configure(&mut config);
    let shell = JsShell::new(core.link(), config).expect("the shell boots over the core");
    (shell, core)
}

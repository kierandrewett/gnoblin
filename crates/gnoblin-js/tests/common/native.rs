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

thread_local! {
    /// Whether this thread holds the process lock: a second core on the thread would wait for the first for ever.
    static HOLDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn process_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    assert!(
        !HOLDS.with(std::cell::Cell::get),
        "a second rig over a core while this test's first is alive: the core's registry is one per process, so drop the first (`drop(rig)`) before the second"
    );
    let guard = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    HOLDS.with(|holds| holds.set(true));
    guard
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

    /// `__gnoblinStd`, for a generation that is not a shell's (`JsConfig::natives`).
    pub fn std_native(&self) -> gnoblin_js::JsNative {
        self.embedded.std_native()
    }

    /// Gives every generation of a shell `__gnoblinStd`.
    pub fn configure(&self, config: &mut JsShellConfig) {
        config.js.natives.push(self.embedded.std_native());
    }

    /// Gives a bare generation (a script run with no shell and no host) `__gnoblinStd`, for a case of a pure function of the stock
    /// library that the library computes.
    pub fn configure_generation(&self, config: &mut gnoblin_js::JsConfig) {
        config.natives.push(self.embedded.std_native());
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
        HOLDS.with(|holds| holds.set(false));
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

/// A shell over a core with std loaded, that is one value: the core lives as long as the shell does and goes after it. It
/// derefs to the shell, so a test that held a `JsShell` holds this.
pub struct CoreShell {
    shell: JsShell,
    /// Dropped after the shell.
    pub core: NativeCore,
}

impl CoreShell {
    /// A shell of `config` over `comp` and a core whose timers read the config's clock.
    pub fn new(comp: &FakeCompositor, config: JsShellConfig) -> CoreShell {
        let clock = Arc::clone(&config.clock);
        let (shell, core) = shell_over_core(comp, clock, config);
        CoreShell { shell, core }
    }

    /// One frame of the shell, then the core's half of it.
    pub fn frame_with_core(&mut self) -> Vec<String> {
        let diagnostics = self.shell.frame().expect("a frame").diagnostics;
        self.core.dispatch(&self.shell);
        diagnostics
    }
}

impl std::ops::Deref for CoreShell {
    type Target = JsShell;

    fn deref(&self) -> &JsShell {
        &self.shell
    }
}

impl std::ops::DerefMut for CoreShell {
    fn deref_mut(&mut self) -> &mut JsShell {
        &mut self.shell
    }
}

//! The shell over the real core and the real std library, in the test's own process.
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
#![allow(dead_code, unsafe_code)]

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_js::{JsRole, JsRoleConfig, JsRoleLinks, JsShell, JsShellConfig};
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
    /// False once [`close`](Self::close) ended the generation, so the drop does not end it again.
    open: bool,
    /// Whether the first generation is still staged, waiting for [`role`](Self::role) to take it over once the entry has loaded.
    booting: bool,
    _lock: MutexGuard<'static, ()>,
}

impl NativeCore {
    /// A core over `comp`, whose timers read `clock`. Its generation is active from the start, for a host whose shell has no entry to
    /// load.
    pub fn new(comp: &FakeCompositor, clock: Arc<dyn Clock>) -> NativeCore {
        Self::over(comp, clock, false)
    }

    /// A core whose first generation is staged, as the loader makes it for a first start (`NativeShell::start`: stage, `init`, activate):
    /// what the entry spawns at its top level waits, and [`role`](Self::role) takes the generation over once the entry has loaded.
    pub fn staged(comp: &FakeCompositor, clock: Arc<dyn Clock>) -> NativeCore {
        Self::over(comp, clock, true)
    }

    fn over(comp: &FakeCompositor, clock: Arc<dyn Clock>, booting: bool) -> NativeCore {
        let lock = process_lock();
        host::reset_session();
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        host::set_clock(clock);
        let ctx = if booting {
            host::stage_generation("js-rig", gnoblin_core_abi::LANG_TS)
        } else {
            host::open_generation("js-rig", gnoblin_core_abi::LANG_TS).expect("a generation opens")
        };
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
            open: true,
            booting,
            _lock: lock,
        }
    }

    /// A role over this core, built as the runtime builds its own (`gnoblin-runtime-js`, `instance.rs`: `role_config` and `boot`).
    ///
    /// The role does not reload itself when its watcher sees a change (`external_reload`): the host serves the reload through the
    /// loader, which stages a generation of the core, so that a stage that fails is rolled back and one that took over ends what the
    /// old one started. A test reloads with [`reload`](Self::reload), which is that sequence. A role that reloaded inside its own
    /// `tick` would evaluate the entry against the generation that serves, with no stage open in the core to roll back.
    ///
    /// When the first generation is staged ([`staged`](Self::staged)) it takes over once the entry has loaded, as `NativeShell::start`
    /// activates it after `init`.
    pub fn role(&mut self, mut config: JsRoleConfig, links: JsRoleLinks) -> JsRole {
        config.external_reload = true;
        self.configure(&mut config.shell);
        let role = JsRole::over_link(self.link(), links, config).expect("a role");
        if std::mem::take(&mut self.booting) {
            host::take_over(self.ctx).expect("the first generation takes over");
        }
        role
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
        config.js.natives.push(self.embedded.object_native());
    }

    /// Gives a bare generation (a script run with no shell and no host) `__gnoblinStd`, for a case of a pure function of the stock
    /// library that the library computes.
    pub fn configure_generation(&self, config: &mut gnoblin_js::JsConfig) {
        config.natives.push(self.embedded.std_native());
        config.natives.push(self.embedded.object_native());
    }

    /// The core's half of a frame: what the library announces and runs, and what it writes.
    pub fn dispatch(&self, shell: &JsShell) -> i32 {
        self.embedded.dispatch(shell)
    }

    /// The runtime's pieces for a host that holds libraries of its own.
    pub fn embedded(&self) -> &Embedded {
        &self.embedded
    }

    /// The generation the shell runs as.
    pub fn ctx(&self) -> GnoblinCtx {
        self.ctx
    }

    /// Ends the generation while `shell` is still alive, as a shutdown does: the core's shutdown hooks run with `shell` as the way its
    /// callbacks enter, so a binding's `onCancel` for a drag in flight reaches the script. Returns how many registrations were removed;
    /// zero when the generation was already closed.
    pub fn close(&mut self, shell: &JsShell) -> usize {
        if !std::mem::replace(&mut self.open, false) {
            return 0;
        }
        self.embedded.enter(shell, || host::close_generation(self.ctx))
    }

    /// [`reload`](Self::reload), with `start` run as the new generation after its script has run and before the commit: what a native
    /// shell's start writes to the tree through the core's node API, with nothing of the std library in it. `start` is given the staged
    /// generation's context. It runs when the script fails too: a start that wrote before it threw has written.
    pub fn reload_with(
        &mut self,
        role: &mut gnoblin_js::JsRole,
        start: impl FnOnce(GnoblinCtx),
    ) -> gnoblin_js::ReloadReport {
        let staged = host::stage_generation("js-rig", gnoblin_core_abi::LANG_TS);
        self.embedded.belong_to(staged);
        let report = role.reload_now();
        start(staged);
        if report.ok {
            let old = std::mem::replace(&mut self.ctx, staged);
            self.embedded.serve(staged);
            self.embedded.enter(role.shell_mut(), || host::end_generation(old));
            host::take_over(staged).expect("the staged generation takes over");
            host::close_generation(old);
        } else {
            self.embedded.belong_to(self.ctx);
            host::close_generation(staged);
        }
        report
    }

    /// Reloads `role` as the runtime and the loader do: the new script is evaluated for a staged generation of the core. When it
    /// took, the commit runs in the loader's order: the old generation's shutdown hooks run (with the shell as the way they enter),
    /// the staged generation takes over (the old claims are released and the new ones sent behind them), and the old one is closed.
    /// Else the staged generation is dropped and the old one serves on.
    pub fn reload(&mut self, role: &mut gnoblin_js::JsRole) -> gnoblin_js::ReloadReport {
        self.reload_with(role, |_| {})
    }
}

impl Drop for NativeCore {
    fn drop(&mut self) {
        if self.open {
            host::close_generation(self.ctx);
        }
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

/// Writes the `effects` of `node` as the generation `ctx`: one shadow of `blur`, through the core's node API as a native shell's start
/// does it.
pub fn write_effect(ctx: GnoblinCtx, node: u64, blur: f64) {
    use gnoblin_core_abi as abi;
    let api = host::api();
    let (class, name) = ("window", "effects");
    // SAFETY: `catalog_class` and `catalog_member` are functions of the core's table; the texts are live and `member` is a live struct
    // that starts with its size.
    let property = unsafe {
        let class = (api.catalog_class)(ctx, class.as_ptr(), class.len());
        assert_ne!(class, 0, "the tree has a window class");
        let mut member: abi::GnoblinMember = std::mem::zeroed();
        member.size = size_of::<abi::GnoblinMember>() as u32;
        let code = (api.catalog_member)(
            ctx,
            class,
            abi::MEMBER_PROPERTY,
            name.as_ptr(),
            name.len(),
            &raw mut member,
        );
        assert_eq!(code, abi::E_OK, "the window class has effects");
        member.id
    };
    // SAFETY: an all-zero effect record is a valid one: no text, no parameters, every number zero.
    let mut effect: abi::GnoblinEffect = unsafe { std::mem::zeroed() };
    effect.size = size_of::<abi::GnoblinEffect>() as u32;
    effect.kind = abi::EFFECT_SHADOW;
    effect.blur_radius = blur;
    effect.colour = abi::GnoblinStr::borrowed("#ff000099");
    let value = abi::GnoblinValue {
        size: size_of::<abi::GnoblinValue>() as u32,
        kind: abi::VALUE_EFFECTS,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: (&raw const effect).cast(),
        len: 1,
    };
    // SAFETY: `write_set` is a function of the core's table; `value` and `effect` are live for the call.
    let code = unsafe { (api.write_set)(ctx, node, property, &value, 0.0) };
    assert_eq!(code, abi::E_OK, "the effects write was refused");
}

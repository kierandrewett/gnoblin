//! The TypeScript std of the parity tests, run over a core with the std library loaded.
//!
//! The reference side of these tests is a `JsShell` running `data/js/gnoblin-std`. Its stock layouts are the library's now
//! (`builtin_*`), so the shell needs what the compositor gives it: a core generation with `libgnoblin-std` and the engine's link as a
//! view of the core's tree (`gnoblin-js/tests/common/native.rs`). A frame is the engine's and then the core's dispatch.
#![allow(dead_code)]

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use gnoblin_js::{JsFrame, JsShell, JsShellConfig, JsShellError};
use gnoblin_testkit::compositor::FakeCompositor;

#[path = "../../../gnoblin-js/tests/common/native.rs"]
pub mod native;

/// A shell over a core with the std library loaded. It reads as the `JsShell`, and its `frame` runs the core's half too.
pub struct Reference {
    shell: JsShell,
    core: native::NativeCore,
}

impl Reference {
    /// A shell over `comp` that boots with `config`.
    pub fn new(comp: &FakeCompositor, config: JsShellConfig) -> Reference {
        let clock = Arc::clone(&config.clock);
        let (shell, core) = native::shell_over_core(comp, clock, config);
        Reference { shell, core }
    }

    /// One frame: the engine's, then the core's dispatch.
    ///
    /// # Errors
    ///
    /// The engine's frame fails.
    pub fn frame(&mut self) -> Result<JsFrame, JsShellError> {
        let frame = self.shell.frame()?;
        self.core.dispatch(&self.shell);
        Ok(frame)
    }
}

impl Deref for Reference {
    type Target = JsShell;

    fn deref(&self) -> &JsShell {
        &self.shell
    }
}

impl DerefMut for Reference {
    fn deref_mut(&mut self) -> &mut JsShell {
        &mut self.shell
    }
}

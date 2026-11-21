//! The stock lock screen as native classes of `gnoblin.std`: a clock and a password entry over the screen while the session is locked.
//!
//! Its behaviour is held to the recorded fixtures `reference/parts_lock*.json` (`design/native-std-port-tests.md`). The screen is built over
//! the UI toolkit (`crate::ui`): every canvas of it is a canvas of the toolkit, mounted under the lock node, updated, painted and written to
//! the compositor by the toolkit's turn. The pieces:
//!
//! - `screen`: the screen itself, which follows the lock's state, takes the compositor's answer to a password check and renews the lease;
//! - `entry`: the password entry, which keeps the typed characters as code points and overwrites them when it hands the password on;
//! - `style`: the `lock_style` record, its stock values and reading one a shell gave;
//! - `classes`: the catalog's classes (`lock.toml`) implemented over the above.
//!
//! The password goes to the compositor through the core's `session_lock` (`gnoblin.model`), which wipes it once it is sent. A shell that makes
//! no `lock_screen` leaves the compositor's own lock card in charge, so a core-only shell is safe by default (`docs/session-lock.md`).

mod classes;
mod entry;
mod screen;
mod style;

use std::rc::Rc;

use gnoblin_paint::PaintOp;

use crate::ui::CanvasEvent;
use crate::ui::widget::{Cid, Ui};

pub(crate) use classes::register;
pub(crate) use entry::PasswordPart;
pub(crate) use screen::{LockWorld, after_update, turn, wake_in};

/// What a canvas of the lock screen does with a key by itself: the password entry types it. A key is never stopped here.
pub(crate) fn key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    entry::key(ui, cid, raw);
    false
}

/// What a canvas of the lock screen paints by itself when it is not the toolkit's own: the password entry's mask.
pub(crate) fn paint(ui: &Ui, cid: Cid) -> Option<Result<Vec<PaintOp>, String>> {
    entry::paint(ui, cid)
}

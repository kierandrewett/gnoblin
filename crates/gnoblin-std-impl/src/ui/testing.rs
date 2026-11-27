//! What a test of the toolkit may change: how text is measured. The painter measures with fonts, which differ from machine to machine;
//! a test replaces it with a rule.

use std::cell::RefCell;
use std::rc::Rc;

pub use super::text::Measurer;

thread_local! {
    static NOTES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Keeps a line the toolkit reported, for [`take_diagnostics`].
pub(crate) fn note(line: &str) {
    NOTES.with(|notes| notes.borrow_mut().push(line.to_owned()));
}

/// The lines the toolkit reported (a step that failed, a handler that threw) since the last call, in order.
pub fn take_diagnostics() -> Vec<String> {
    NOTES.with(|notes| std::mem::take(&mut *notes.borrow_mut()))
}

/// Measures every text of this thread with `measurer` from now on.
pub fn set_measurer(measurer: Rc<dyn Measurer>) {
    super::text::set_measurer(measurer);
}

/// A measurement the measurer gives changed (an answer arrived): every text measures again and every canvas is updated.
pub fn measurements_changed() {
    for ui in super::tree::all() {
        ui.epoch.set(ui.epoch.get() + 1);
        ui.schedule();
    }
}

/// Stops the stock focus policy of every toolkit of this thread: the test's shell has a focus policy of its own, as the recorded
/// TypeScript library does.
pub fn stop_stock_focus() {
    for ui in super::tree::all() {
        crate::policy::stop_focus(&ui);
    }
}

//! Example 20: the typed object API of `libgnoblin-std`.
//!
//! The classes of `gnoblin.std` (an entry model, the focus policy, the records and enums they take) are not functions of the library's
//! table: they are served through one object table, and `gnoblin_std::objects` is the typed Rust API written from the library's
//! catalog. This shell uses it the way a toolkit would:
//!
//! - makes an [`EntryModel`] and reads its properties (`text`, `length`, `caret`, `selection`);
//! - calls its methods (`insert`, `select_all`), which change it at once, and listens to its `changed` signal, which the library tells
//!   from its next turn and never inside the call that caused it;
//! - calls a static function of the focus policy with records and an enum, and reads the record it answers with (a field the library
//!   leaves out is an `Option`, one it always sends is the type itself);
//! - keeps the entry in the closure of a chord: `<Super>e` types one more `!` into it.
//!
//! Every line it logs starts with `object:`. Build it with `cargo build -p gnoblin-example-native-object` and run it with
//! `gnoblin ./target/debug/libnative_object.so`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_std::{EntryModel, FocusEvent, FocusEventKind, FocusMode, FocusPolicy, FocusSettings};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;

    // Made with its text; the caret starts at the end, and positions count characters, not bytes.
    let entry = EntryModel::new(&objects, "héllo")?;
    g.log(&format!(
        "object: made an entry of {} characters with the caret at {}",
        entry.length()?,
        entry.caret()?
    ));

    // The library tells `changed` from its next turn, so these lines come after the ones above and below.
    let heard = Rc::new(Cell::new(0_u32));
    let counter = Rc::clone(&heard);
    entry
        .on_changed(move || {
            counter.set(counter.get() + 1);
            g.log(&format!("object: heard changed #{}", counter.get()));
        })?
        .keep();

    entry.insert(", world")?;
    entry.select_all()?;
    let selection = entry.selection()?;
    g.log(&format!(
        "object: text {:?} selected {:?} as {selection:?}",
        entry.text()?,
        entry.selected_text()?
    ));

    // Typing over a selection replaces it, so put the caret at the end first: `extend` is an optional argument and is left out.
    entry.end(None)?;

    // A function of the focus policy: records go in, a record comes out, and a field the caller leaves out has its default.
    let settings = FocusSettings {
        mode: FocusMode::Sloppy,
        ..FocusSettings::default()
    };
    let event = FocusEvent {
        held: Some(false),
        ..FocusEvent::new(FocusEventKind::Enter)
    };
    let action = FocusPolicy::focus_decision(&objects, &settings, &event)?;
    g.log(&format!(
        "object: a pointer entering a window under sloppy focus: focus {:?}, raise {:?}",
        action.focus, action.raise
    ));

    g.bind("<Super>e", move || {
        let before = heard.get();
        let typed = entry.insert("!").and_then(|()| entry.text());
        match typed {
            Ok(text) => g.log(&format!("object: typed, text is {text:?}, heard {before} so far")),
            Err(error) => g.log(&format!("object: cannot edit the entry: {error}")),
        }
    })?;

    g.log("object: running");
    Ok(())
}

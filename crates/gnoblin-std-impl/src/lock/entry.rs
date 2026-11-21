//! The password entry: a box that holds the typed characters as code points and draws one mask character for each.
//!
//! Return hands the password, as one string, to the entry's `on_submit` (or to the lock screen that owns the entry) and then overwrites what
//! the entry held. A key typed while the entry is not enabled is dropped. The characters are never logged or kept in a signal.

use std::rc::Rc;

use gnoblin_object::{Closure, Val};
use gnoblin_paint::PaintOp;

use super::style::LockStyle;
use crate::ui::CanvasEvent;
use crate::ui::paint;
use crate::ui::widget::{Cid, DEFAULT_FONT, Special, Ui};

/// What a password entry adds to a box.
#[derive(Clone)]
pub(crate) struct PasswordPart {
    /// The look the entry was made with.
    pub(crate) style: LockStyle,
    /// The characters typed, as code points.
    typed: Vec<u32>,
    /// Whether the entry takes keys.
    pub(crate) enabled: bool,
    /// The most characters it takes.
    pub(crate) limit: usize,
    /// The mask text's size.
    pub(crate) size: f64,
    /// The mask text's colour.
    pub(crate) color: String,
    /// Receives the password on Return.
    pub(crate) on_submit: Option<Closure>,
    /// The password goes to the lock screen that owns the entry, not to `on_submit`.
    pub(crate) screen: bool,
}

impl PasswordPart {
    /// An entry of `style`.
    pub(crate) fn new(style: &LockStyle, screen: bool) -> PasswordPart {
        PasswordPart {
            style: style.clone(),
            typed: Vec::new(),
            enabled: true,
            limit: usize::try_from(style.entry_limit).unwrap_or(usize::MAX),
            size: style.entry_size,
            color: style.entry_color.clone(),
            on_submit: None,
            screen,
        }
    }

    /// Forgets what was typed: the code points are overwritten, then dropped.
    fn wipe(&mut self) {
        self.typed.fill(0);
        self.typed.clear();
    }
}

/// Makes the canvas `cid` a password entry of `style`, `width` wide.
pub(crate) fn init(ui: &Ui, cid: Cid, style: &LockStyle, width: f64, screen: bool) {
    ui.tweak(cid, |widget| {
        widget.width = Some(width);
        widget.height = Some(style.entry_height);
        widget.b.fill = style.entry_fill.clone();
        widget.b.radius = style.entry_radius;
        widget.sp = Special::Password(Box::new(PasswordPart::new(style, screen)));
    });
}

/// Reads what a password entry keeps.
pub(crate) fn read<R>(ui: &Ui, cid: Cid, pick: impl FnOnce(&PasswordPart) -> R) -> Option<R> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Password(part) => Some(pick(part)),
        _ => None,
    })
    .flatten()
}

/// Changes what a password entry keeps.
pub(crate) fn change(ui: &Ui, cid: Cid, change: impl FnOnce(&mut PasswordPart)) {
    ui.tweak(cid, |widget| {
        if let Special::Password(part) = &mut widget.sp {
            change(part);
        }
    });
}

/// How many characters are typed.
pub(crate) fn length(ui: &Ui, cid: Cid) -> usize {
    read(ui, cid, |part| part.typed.len()).unwrap_or(0)
}

/// Forgets what was typed.
pub(crate) fn clear(ui: &Rc<Ui>, cid: Cid) {
    change(ui, cid, PasswordPart::wipe);
    ui.schedule();
}

/// Overwrites a copy of the password.
fn forget(password: String) {
    let mut bytes = password.into_bytes();
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

/// Hands the typed password on, then forgets it. Nothing happens when nothing is typed or nobody wants it.
fn submit(ui: &Rc<Ui>, cid: Cid) {
    let Some((password, handler, to_screen)) = read(ui, cid, |part| {
        if part.typed.is_empty() || (!part.screen && part.on_submit.is_none()) {
            return None;
        }
        let password: String = part.typed.iter().filter_map(|code| char::from_u32(*code)).collect();
        Some((password, part.on_submit.clone(), part.screen))
    })
    .flatten() else {
        return;
    };
    if to_screen {
        super::screen::submit(ui, &password);
    } else if let Some(handler) = handler
        && let Err(fail) = handler.call(0, &[Val::Text(password.clone())])
    {
        ui.warn(&format!("PasswordEntry.onKey threw: {fail}"));
    }
    forget(password);
    clear(ui, cid);
}

/// What the keyboard does to an entry: typed characters, Return and BackSpace while it is enabled. It never stops a key.
pub(crate) fn key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) {
    let Some(enabled) = read(ui, cid, |part| part.enabled) else {
        return;
    };
    if !enabled || !raw.pressed {
        return;
    }
    match raw.key.as_str() {
        "Return" | "KP_Enter" => submit(ui, cid),
        "BackSpace" => change(ui, cid, |part| {
            part.typed.pop();
        }),
        _ => change(ui, cid, |part| {
            for character in raw.text.chars() {
                let code = u32::from(character);
                if code >= 0x20 && code != 0x7f && part.typed.len() < part.limit {
                    part.typed.push(code);
                }
            }
        }),
    }
    ui.schedule();
}

/// What the entry draws: its box, and one mask character for each typed code point.
pub(crate) fn paint(ui: &Ui, cid: Cid) -> Option<Result<Vec<PaintOp>, String>> {
    let (typed, size, color) = read(ui, cid, |part| (part.typed.len(), part.size, part.color.clone()))?;
    Some(ops(ui, cid, typed, size, &color))
}

fn ops(ui: &Ui, cid: Cid, typed: usize, size: f64, color: &str) -> Result<Vec<PaintOp>, String> {
    let mut ops = ui.box_ops(cid)?;
    let y = ((ui.height_of(cid) - (size * 3.0 / 2.0).floor()) / 2.0).floor();
    ops.push(paint::text(12.0, y, size, color, DEFAULT_FONT, &"*".repeat(typed))?);
    Ok(ops)
}

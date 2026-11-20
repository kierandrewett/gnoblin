//! `entry_model`: the text, caret and selection of an entry, measured in Unicode code points.
//!
//! The behaviour is that of `EntryModel` in the TypeScript library: typing replaces the selection, deleting removes it first, a caret
//! move that does not extend a selection collapses it, and the control characters a key carries are left out.

use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Res, Val};

/// The model's state.
#[derive(Clone, PartialEq, Eq)]
struct EntryModel {
    chars: Vec<char>,
    caret: usize,
    anchor: usize,
}

/// Whether a key's text may be typed: not a control character.
fn typed(c: char) -> bool {
    let code = u32::from(c);
    code >= 0x20 && code != 0x7f && !(0x80..=0x9f).contains(&code)
}

impl EntryModel {
    fn text(&self) -> String {
        self.chars.iter().collect()
    }

    fn selection(&self) -> Option<(usize, usize)> {
        (self.anchor != self.caret).then(|| (self.anchor.min(self.caret), self.anchor.max(self.caret)))
    }

    fn remove_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.chars.drain(start..end);
        self.caret = start;
        self.anchor = start;
        true
    }

    fn insert(&mut self, text: &str) {
        let typed: Vec<char> = text.chars().filter(|c| typed(*c)).collect();
        if typed.is_empty() {
            return;
        }
        self.remove_selection();
        let count = typed.len();
        self.chars.splice(self.caret..self.caret, typed);
        self.caret += count;
        self.anchor = self.caret;
    }

    fn backspace(&mut self) {
        if self.remove_selection() || self.caret == 0 {
            return;
        }
        self.chars.remove(self.caret - 1);
        self.caret -= 1;
        self.anchor = self.caret;
    }

    fn delete(&mut self) {
        if self.remove_selection() || self.caret >= self.chars.len() {
            return;
        }
        self.chars.remove(self.caret);
    }

    fn move_to(&mut self, at: i64, extend: bool) {
        let at = at.clamp(0, i64::try_from(self.chars.len()).unwrap_or(i64::MAX));
        self.caret = usize::try_from(at).unwrap_or(0);
        if !extend {
            self.anchor = self.caret;
        }
    }

    fn step(&mut self, forward: bool, extend: bool) {
        if !extend && let Some((start, end)) = self.selection() {
            self.move_to(i64::try_from(if forward { end } else { start }).unwrap_or(0), false);
        } else {
            let from = i64::try_from(self.caret).unwrap_or(0);
            self.move_to(if forward { from + 1 } else { from - 1 }, extend);
        }
    }
}

/// Runs a change and tells the listeners what it changed.
fn change(model: &mut EntryModel, cx: &mut Cx<'_>, edit: impl FnOnce(&mut EntryModel)) -> Res<Val> {
    let before = model.clone();
    edit(model);
    if *model != before {
        if model.chars != before.chars {
            cx.changed("text");
            cx.changed("length");
        }
        if model.caret != before.caret {
            cx.changed("caret");
        }
        if model.anchor != before.anchor {
            cx.changed("anchor");
        }
        cx.emit("changed", Vec::new());
    }
    Ok(Val::None)
}

/// The optional `extend` argument of a caret move.
fn extend(args: &[Val], at: usize) -> Res<bool> {
    args.get(at).map_or(
        Ok(false),
        |value| if value.is_none() { Ok(false) } else { value.bool() },
    )
}

fn method<'a>(
    class: ClassBuilder<'a, EntryModel>,
    name: &str,
    edit: fn(&mut EntryModel, &[Val]) -> Res<()>,
) -> ClassBuilder<'a, EntryModel> {
    class.method(name, move |model, cx, args| {
        change(model, cx, |model| {
            // The arguments were checked against the catalog, so reading one cannot fail.
            let _ = edit(model, args);
        })
    })
}

pub(super) fn register(builder: &mut Builder) {
    let class = builder
        .class::<EntryModel>("entry_model")
        .ctor(|_cx, inits| {
            let chars: Vec<char> = match inits.take("text") {
                Some(text) => text.text()?.chars().collect(),
                None => Vec::new(),
            };
            let end = chars.len();
            Ok(EntryModel {
                chars,
                caret: end,
                anchor: end,
            })
        })
        .get("text", |model, _| Ok(Val::Text(model.text())))
        .set("text", |model, cx, value| {
            change(model, cx, |model| {
                model.chars = value.text().unwrap_or_default().chars().collect();
                model.caret = model.chars.len();
                model.anchor = model.caret;
            })
            .map(drop)
        })
        .get("length", |model, _| Ok(Val::Uint(model.chars.len() as u64)))
        .get("caret", |model, _| Ok(Val::Uint(model.caret as u64)))
        .get("anchor", |model, _| Ok(Val::Uint(model.anchor as u64)))
        .get("selection", |model, _| {
            Ok(model.selection().map_or(Val::None, |(start, end)| Val::Record {
                class: 0,
                fields: vec![Val::Uint(start as u64), Val::Uint(end as u64)],
            }))
        })
        .get("selected_text", |model, _| {
            Ok(Val::Text(
                model
                    .selection()
                    .map(|(start, end)| model.chars[start..end].iter().collect())
                    .unwrap_or_default(),
            ))
        })
        .method("before", |model, _cx, args| {
            let at = usize::try_from(args[0].int()?.max(0))
                .unwrap_or(0)
                .min(model.chars.len());
            Ok(Val::Text(model.chars[..at].iter().collect()))
        });
    let class = method(class, "insert", |model, args| {
        model.insert(args[0].text()?);
        Ok(())
    });
    let class = method(class, "backspace", |model, _| {
        model.backspace();
        Ok(())
    });
    let class = method(class, "delete", |model, _| {
        model.delete();
        Ok(())
    });
    let class = method(class, "move", |model, args| {
        model.move_to(args[0].int()?, extend(args, 1)?);
        Ok(())
    });
    let class = method(class, "left", |model, args| {
        model.step(false, extend(args, 0)?);
        Ok(())
    });
    let class = method(class, "right", |model, args| {
        model.step(true, extend(args, 0)?);
        Ok(())
    });
    let class = method(class, "home", |model, args| {
        model.move_to(0, extend(args, 0)?);
        Ok(())
    });
    let class = method(class, "end", |model, args| {
        model.move_to(i64::try_from(model.chars.len()).unwrap_or(0), extend(args, 0)?);
        Ok(())
    });
    let class = method(class, "select_all", |model, _| {
        model.anchor = 0;
        model.caret = model.chars.len();
        Ok(())
    });
    let _ = method(class, "clear", |model, _| {
        model.chars.clear();
        model.caret = 0;
        model.anchor = 0;
        Ok(())
    });
}

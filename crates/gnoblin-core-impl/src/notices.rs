//! The notices of a generation: the sentences its libraries and the core have for the shell's diagnostics.
//!
//! A library that logs at [`LOG_NOTICE`](gnoblin_core_abi::LOG_NOTICE) says something the shell's author must read: a callback of
//! theirs threw, a style was not applied, the compositor refused a transaction. The core keeps the sentence for the shell
//! (`take_notices`) and says it once per *site*. A callback that threw is one site wherever it threw from and whatever it threw, so a
//! listener that fails on every event is reported once, not once per event; a sentence with no stack is its own site.
//!
//! A shell that takes notices says them itself (its diagnostics are logged as warnings by the shell's role), so the core logs a notice
//! as a warning only until the shell has taken its first. A shell that never takes them finds every first sentence in the log.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, PoisonError};

/// The most sentences kept for a shell that has not taken them yet: the oldest go first.
const PENDING_KEPT: usize = 256;
/// The most sites remembered for one generation. A generation that outgrows it says its repeats once more, which is better than a
/// list that grows for as long as the shell runs.
const SITES_KEPT: usize = 4096;

#[derive(Default)]
struct Book {
    /// What the shell has not taken, oldest first.
    pending: Vec<String>,
    /// The sites said so far.
    said: HashSet<String>,
    /// Whether the shell has taken notices: it says them itself from then on.
    taken: bool,
}

static BOOKS: LazyLock<Mutex<HashMap<u64, Book>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The site of a sentence: the first frame of its stack when it has one, else the whole sentence.
fn site_of(text: &str) -> &str {
    text.lines()
        .skip(1)
        .map(str::trim)
        .find(|line| line.starts_with("at "))
        .unwrap_or(text)
}

/// Keeps `text` for the shell of generation `generation`, once per site. Answers whether the core is to log it as a warning: it is
/// new, and no shell takes notices to say it.
pub(crate) fn say(generation: u64, text: &str) -> bool {
    let mut books = BOOKS.lock().unwrap_or_else(PoisonError::into_inner);
    let book = books.entry(generation).or_default();
    if book.said.len() >= SITES_KEPT {
        book.said.clear();
    }
    if !book.said.insert(site_of(text).to_owned()) {
        return false;
    }
    if book.pending.len() >= PENDING_KEPT {
        book.pending.remove(0);
    }
    book.pending.push(text.to_owned());
    !book.taken
}

/// What the shell of generation `generation` has not taken, joined by a zero byte. The shell says them from now on.
pub(crate) fn take(generation: u64) -> String {
    let mut books = BOOKS.lock().unwrap_or_else(PoisonError::into_inner);
    let book = books.entry(generation).or_default();
    book.taken = true;
    std::mem::take(&mut book.pending).join("\0")
}

/// The generation ended: what it said goes with it.
pub(crate) fn forget(generation: u64) {
    BOOKS.lock().unwrap_or_else(PoisonError::into_inner).remove(&generation);
}

//! The `console` global of a generation: `console.log`, `info`, `warn`, `error`, `debug`, `assert` and `table`.
//!
//! Every generation has one, from `init.ts` to a listener to a console expression, because [`JsGeneration::new`]
//! installs it before anything runs. The formatting (Node's `%s %d %j`, errors with their stack, JSON-like objects with
//! a `[Circular]` marker) is `js/console.js`; this module is what it calls: the rate limit, the call site and the log.
//!
//! - **Where a line goes.** To the log, component `js`, area `script`, with `log` and `info` at info level, `warn` at
//!   warn, `error` at error and `debug` at debug, so `gnoblinctl logs` shows it. The message starts with the call site
//!   as `file.ts:line:col` (the file relative to the config root, the position in the `.ts` the author wrote). A
//!   generation that a [`JsConsole`](crate::JsConsole) evaluates in also keeps the lines an expression produced, so the
//!   prompt can print them.
//! - **Caps.** One call is at most [`MAX_LINE_BYTES`] of text (the rest is cut and says how much). A generation may log
//!   [`MAX_LINES_PER_SECOND`] calls in a second; the ones past it are dropped without being formatted, and when the
//!   next second begins one notice says how many were, so a loop that logs cannot flood the log.
//! - **Time** is the generation's clock ([`JsConfig::clock`](crate::JsConfig)): a test moves it and no real time
//!   passes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use rquickjs::context::EvalOptions;
use rquickjs::{Ctx, Function, Object};

use crate::generation::{JsConfig, remap_stack, split_frame};
use crate::loader::JsLoadState;
use crate::supervisor::JsClock;

gnoblin_log::logger!(log = "js");

/// `console.js`, evaluated once per generation.
const PRELUDE: &str = include_str!("../js/console.js");

/// The name `console.js` has in a stack trace; its frames are never the call site.
const PRELUDE_FILE: &str = "gnoblin/console";

/// The most text one console call logs, in bytes.
pub const MAX_LINE_BYTES: usize = 4096;

/// How many console calls a generation may log in one second.
pub const MAX_LINES_PER_SECOND: usize = 200;

/// The span the line limit counts in.
const WINDOW: Duration = Duration::from_secs(1);

/// How serious a console line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsConsoleLevel {
    /// `console.debug`.
    Debug,
    /// `console.log`, `console.info` and `console.table`.
    Info,
    /// `console.warn`.
    Warn,
    /// `console.error` and a failed `console.assert`.
    Error,
}

impl JsConsoleLevel {
    /// The level a console method logs at: `log` and `info` are info.
    fn of_method(method: &str) -> Self {
        match method {
            "debug" => Self::Debug,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }

    /// The level's name as the console protocol and the log say it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One console call, as logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsConsoleLine {
    /// How serious it is.
    pub level: JsConsoleLevel,
    /// Where the script called from, as `file:line:col` in the author's coordinates. `None` for the generation's own
    /// notices and for a call with no frame in a file.
    pub site: Option<String>,
    /// What the call printed, at most [`MAX_LINE_BYTES`] bytes long.
    pub text: String,
}

impl JsConsoleLine {
    /// The log message: the site, then the text.
    fn message(&self) -> String {
        match &self.site {
            Some(site) => format!("{site}: {}", self.text),
            None => self.text.clone(),
        }
    }
}

/// The count of the current second and what a console evaluation has collected.
#[derive(Default)]
struct JsConsoleWindow {
    /// When the second began, on the generation's clock; `None` before the first call.
    start: Option<Duration>,
    lines: usize,
    dropped: usize,
    capture: Option<Vec<JsConsoleLine>>,
}

/// A generation's console: the limit, the call-site lookup and the sink.
pub(crate) struct JsScriptConsole {
    clock: Arc<dyn JsClock>,
    /// The canonical config root, so a file under it is shown relative to it.
    root: Option<PathBuf>,
    load: Rc<JsLoadState>,
    window: RefCell<JsConsoleWindow>,
}

impl JsScriptConsole {
    pub(crate) fn new(config: &JsConfig, load: Rc<JsLoadState>) -> Rc<Self> {
        Rc::new(Self {
            clock: Arc::clone(&config.clock),
            root: std::fs::canonicalize(&config.root).ok(),
            load,
            window: RefCell::new(JsConsoleWindow::default()),
        })
    }

    /// Defines `globalThis.console` in the context `ctx` belongs to.
    pub(crate) fn install(self: &Rc<Self>, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let native = Object::new(ctx.clone())?;
        let console = Rc::clone(self);
        native.set("admit", Function::new(ctx.clone(), move || console.admit()))?;
        let console = Rc::clone(self);
        native.set(
            "emit",
            Function::new(ctx.clone(), move |method: String, text: String, stack: String| {
                console.emit(&method, &text, &stack);
            }),
        )?;
        let load = Rc::clone(&self.load);
        native.set(
            "remap",
            Function::new(ctx.clone(), move |stack: String| remap_stack(&load, &stack).0),
        )?;
        let mut options = EvalOptions::default();
        options.filename = Some(PRELUDE_FILE.to_owned());
        let factory: Function<'_> = ctx.eval_with_options(PRELUDE, options)?;
        factory.call::<_, ()>((native,))
    }

    /// Whether one more line may be logged now. A line that may not is counted as dropped.
    fn admit(&self) -> bool {
        self.roll();
        let mut window = self.window.borrow_mut();
        if window.lines >= MAX_LINES_PER_SECOND {
            window.dropped += 1;
            return false;
        }
        window.lines += 1;
        true
    }

    /// Starts a new second when the last one is over, and says how many lines it dropped.
    pub(crate) fn roll(&self) {
        let now = self.clock.now();
        let dropped = {
            let mut window = self.window.borrow_mut();
            if window.start.is_some_and(|start| now.saturating_sub(start) < WINDOW) {
                return;
            }
            window.start = Some(now);
            window.lines = 0;
            std::mem::take(&mut window.dropped)
        };
        if dropped > 0 {
            self.deliver(JsConsoleLine {
                level: JsConsoleLevel::Warn,
                site: None,
                text: format!(
                    "{dropped} console lines dropped: a generation logs at most {MAX_LINES_PER_SECOND} a second"
                ),
            });
        }
    }

    /// Logs one finished call: its text cut to the cap, its site found in `stack`.
    fn emit(&self, method: &str, text: &str, stack: &str) {
        self.deliver(JsConsoleLine {
            level: JsConsoleLevel::of_method(method),
            site: self.call_site(stack),
            text: capped(text),
        });
    }

    /// The first frame of `stack` that is not `console.js`, as `file:line:col` in the author's coordinates.
    fn call_site(&self, stack: &str) -> Option<String> {
        let frame = stack
            .lines()
            .filter_map(split_frame)
            .find(|frame| frame.file != PRELUDE_FILE)?;
        let (line, column) = self
            .load
            .remap(frame.file, frame.line, frame.column)
            .unwrap_or((frame.line, frame.column));
        let file = self
            .root
            .as_deref()
            .and_then(|root| std::path::Path::new(frame.file).strip_prefix(root).ok())
            .map_or_else(|| frame.file.to_owned(), |relative| relative.display().to_string());
        Some(format!("{file}:{line}:{column}"))
    }

    fn deliver(&self, line: JsConsoleLine) {
        let message = line.message();
        match line.level {
            JsConsoleLevel::Debug => log::debug!(area = "script", "{message}"),
            JsConsoleLevel::Info => log::info!(area = "script", "{message}"),
            JsConsoleLevel::Warn => log::warn!(area = "script", "{message}"),
            JsConsoleLevel::Error => log::error!(area = "script", "{message}"),
        }
        if let Some(capture) = self.window.borrow_mut().capture.as_mut() {
            capture.push(line);
        }
    }

    /// Starts keeping the lines that are logged, for a console evaluation.
    pub(crate) fn begin_capture(&self) {
        self.window.borrow_mut().capture = Some(Vec::new());
    }

    /// Stops keeping lines and returns the ones kept since [`begin_capture`](Self::begin_capture).
    pub(crate) fn end_capture(&self) -> Vec<JsConsoleLine> {
        self.window.borrow_mut().capture.take().unwrap_or_default()
    }
}

/// `text` cut to [`MAX_LINE_BYTES`] at a character boundary, with a note of how much was cut.
fn capped(text: &str) -> String {
    if text.len() <= MAX_LINE_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_LINE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... ({} more bytes)", &text[..end], text.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_is_cut_on_a_character_boundary_and_says_how_much() {
        let long = "é".repeat(MAX_LINE_BYTES);
        let cut = capped(&long);
        assert!(cut.starts_with('é'));
        assert!(cut.ends_with(&format!("... ({} more bytes)", long.len() - MAX_LINE_BYTES)));
        assert_eq!(capped("short"), "short");
    }

    #[test]
    fn the_methods_map_to_levels() {
        assert_eq!(JsConsoleLevel::of_method("log"), JsConsoleLevel::Info);
        assert_eq!(JsConsoleLevel::of_method("info"), JsConsoleLevel::Info);
        assert_eq!(JsConsoleLevel::of_method("warn"), JsConsoleLevel::Warn);
        assert_eq!(JsConsoleLevel::of_method("error"), JsConsoleLevel::Error);
        assert_eq!(JsConsoleLevel::of_method("debug"), JsConsoleLevel::Debug);
    }
}

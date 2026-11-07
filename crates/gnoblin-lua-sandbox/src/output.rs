//! The `print` and `warn` functions, routed to the host instead of standard output.
//!
//! Stock `print` writes to stdout and stock `warn` writes to stderr. A compositor has
//! neither, and a script must not be able to fill the host's streams, so both functions
//! hand their text to a [`Sink`] the host supplies. The host decides where the text goes.
//! The sandbox crate does not depend on the logger, so the sink is a plain closure.
//!
//! `print` keeps stock semantics: each argument goes through the global `tostring`, so
//! `__tostring` and `__name` apply, and the results are joined with tabs. `warn` keeps
//! stock semantics too: its arguments must be strings or numbers, and a single argument
//! starting with `@` is a control message. `@on` enables delivery and `@off` disables
//! it. Stock Lua starts with warnings off, and so does this module.

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, MultiValue, Value};

/// How severe a message from a script is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Output from `print`, which is routine and belongs at debug level.
    Debug,
    /// Output from `warn`, which a script raises on purpose and the host should show.
    Warn,
}

/// The host's receiver for script output. Called once per `print` or delivered `warn`.
pub type Sink = Rc<dyn Fn(LogLevel, &str)>;

/// Replace `print` and `warn` with functions that send their text to `sink`.
///
/// `tostring` is captured here, so the stock function is the one `print` calls even if a
/// script later rebinds the global. The warning switch starts off, as in stock Lua.
pub(crate) fn install(lua: &Lua, sink: Sink) -> Result<(), Error> {
    let globals = lua.globals();
    let tostring: Function = globals.get("tostring")?;

    let print_sink = Rc::clone(&sink);
    let print = lua.create_function(move |_, args: MultiValue| {
        let mut parts = Vec::with_capacity(args.len());
        for value in args {
            parts.push(tostring.call::<String>(value)?);
        }
        print_sink(LogLevel::Debug, &parts.join("\t"));
        Ok(())
    })?;
    globals.set("print", print)?;

    // Shared between calls, so `@on` in one call affects every later call in this state.
    let enabled = Rc::new(Cell::new(false));
    let warn = lua.create_function(move |_, args: MultiValue| {
        let mut parts = Vec::with_capacity(args.len());
        for (index, value) in args.iter().enumerate() {
            parts.push(warn_piece(index, value)?);
        }
        // A lone `@...` argument is a control message, never delivered as text.
        if parts.len() == 1 && parts[0].starts_with('@') {
            match parts[0].as_str() {
                "@on" => enabled.set(true),
                "@off" => enabled.set(false),
                _ => {}
            }
            return Ok(());
        }
        if enabled.get() {
            sink(LogLevel::Warn, &parts.concat());
        }
        Ok(())
    })?;
    globals.set("warn", warn)
}

/// One argument of `warn` as text, or the stock error for a value that is neither a
/// string nor a number.
fn warn_piece(index: usize, value: &Value) -> Result<String, Error> {
    match value {
        Value::String(text) => Ok(text.to_str()?.to_string()),
        Value::Integer(number) => Ok(number.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        other => Err(Error::runtime(format!(
            "bad argument #{} to 'warn' (string expected, got {})",
            index + 1,
            other.type_name()
        ))),
    }
}

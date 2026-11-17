//! The language-neutral call layer over `libgnoblin-std`'s function table.
//!
//! A script runtime does not bind the std table function by function. It converts its own values to [`Dyn`] and calls
//! [`StdLib::call`] with the name of a table entry; this crate checks the arguments, fills in the `repr(C)` records, calls the
//! function, reads the records it wrote and hands the result back as a [`Dyn`]. Every function of the table, every record and
//! every callback trampoline is **generated** by `cargo xtask gen-sdks` from `gnoblin-std-abi` (`generated.rs`), so a function
//! added to the table is callable from every language the day the generator runs, and no runtime lists a function or a record.
//!
//! What is written by hand is what does not depend on the table:
//!
//! - [`Dyn`], the value that crosses, and [`Callable`], how the library calls a script's function or object back;
//! - [`StdLib`]: finding `libgnoblin-std.so.1` with `dlopen` (a core-only shell never maps it, so a runtime cannot link it), the
//!   table, the failure text, and attaching the library to each generation once;
//! - the helpers the generated code uses: argument conversion, [`MapReader`] for records, the [`Arena`] that keeps text and
//!   records alive for one call and frees the `user` of a callback the library refused.
//!
//! The rules of the table hold here as they do for any shell: every call runs on the shell thread, the library never runs a
//! callback inside a call, and a text or an array the library lends is copied before the call returns.
#![allow(unsafe_code)]

mod arena;
mod convert;
// Written by `cargo xtask gen-sdks`.
#[allow(dead_code, unused_variables, unused_mut, clippy::all)]
mod generated;
mod library;

use std::rc::Rc;

use gnoblin_core_abi::{E_OK, E_PANIC, E_TYPE, E_UNSUPPORTED};

pub use arena::Arena;
pub use convert::MapReader;
pub use generated::{FUNCTIONS, FunctionInfo};
pub use library::{GetApiFn, LastErrorFn, StdLib};

/// A value that crosses between a script and the library.
///
/// A script's number is [`Dyn::Float`] or [`Dyn::Int`]/[`Dyn::Uint`]; every conversion to a field accepts any of them when the
/// value is whole and fits, so a runtime need not know which integer type a field is. The ids of nodes, workspaces and
/// registrations are [`Dyn::Uint`].
#[derive(Clone)]
pub enum Dyn {
    /// Nothing: an absent record, a missing value, a call with no result.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer: an id, a handle, a flag set.
    Uint(u64),
    /// A floating-point number.
    Float(f64),
    /// UTF-8 text.
    Text(String),
    /// A list.
    List(Vec<Dyn>),
    /// A record or a dictionary, in the order the script gave it.
    Map(Vec<(String, Dyn)>),
    /// A function or an object the library calls back.
    Object(Rc<dyn Callable>),
}

impl std::fmt::Debug for Dyn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dyn::Nil => f.write_str("nil"),
            Dyn::Bool(value) => write!(f, "{value}"),
            Dyn::Int(value) => write!(f, "{value}"),
            Dyn::Uint(value) => write!(f, "{value}"),
            Dyn::Float(value) => write!(f, "{value}"),
            Dyn::Text(value) => write!(f, "{value:?}"),
            Dyn::List(items) => f.debug_list().entries(items).finish(),
            Dyn::Map(entries) => f
                .debug_map()
                .entries(entries.iter().map(|(key, value)| (key, value)))
                .finish(),
            Dyn::Object(_) => f.write_str("<object>"),
        }
    }
}

/// How the library calls a script back: a function the script gave for a listener, or an object whose methods are a layout's
/// or a binding's functions.
///
/// An implementation runs the script's code on the shell thread and reports a failure as its text; it has already logged the
/// failure as its language does (a traceback, a stack), so the library only learns that the callback failed.
pub trait Callable {
    /// Calls the function itself: a listener.
    ///
    /// # Errors
    ///
    /// The script raised.
    fn invoke(&self, args: Vec<Dyn>) -> Result<Dyn, String>;

    /// Calls the method `name` of the object.
    ///
    /// # Errors
    ///
    /// The script raised, or the object has no such method.
    fn invoke_method(&self, name: &str, args: Vec<Dyn>) -> Result<Dyn, String>;

    /// Whether the object has the method `name`. An optional function of a table is given to the library only when it has.
    fn has_method(&self, name: &str) -> bool;
}

/// A failed call: the ABI's error code and the text the library left for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdError {
    /// A negative `E_*` code of the ABI: what the library returned, or the one of the call layer's own refusals
    /// (`E_TYPE` for an argument of the wrong kind, `E_UNSUPPORTED` for a function the installed library lacks).
    pub code: i32,
    /// What went wrong, in words.
    pub message: String,
}

impl StdError {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> StdError {
        StdError {
            code,
            message: message.into(),
        }
    }

    /// An argument or a field of the wrong kind.
    pub(crate) fn type_error(message: impl Into<String>) -> StdError {
        StdError::new(E_TYPE, message)
    }

    /// The table has no such function, or the installed library is too old to have it.
    pub(crate) fn unsupported(function: &str) -> StdError {
        StdError::new(
            E_UNSUPPORTED,
            format!("the installed libgnoblin-std has no function `{function}`"),
        )
    }
}

impl std::fmt::Display for StdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StdError {}

/// The `user` the library hands back to a trampoline: a script's object, boxed once.
type User = Rc<dyn Callable>;

/// Frees a `user` made by [`Arena::user`]. The library calls it exactly once, when the registration or the layout ends.
pub(crate) unsafe extern "C" fn drop_user(user: *mut core::ffi::c_void) {
    if !user.is_null() {
        // SAFETY: `user` came from `Box::into_raw(Box::new(Rc<dyn Callable>))` in `Arena::user`, and the library calls this once.
        drop(unsafe { Box::from_raw(user.cast::<User>()) });
    }
}

/// The object a trampoline's `user` names.
///
/// # Safety
///
/// `user` must be a pointer [`Arena::user`] made that has not been dropped.
pub(crate) unsafe fn user_of<'a>(user: *mut core::ffi::c_void) -> &'a User {
    // SAFETY: the caller guarantees `user` is a live `Box<Rc<dyn Callable>>`.
    unsafe { &*user.cast::<User>() }
}

/// Runs a trampoline body so that nothing unwinds into the library.
pub(crate) fn contain(body: impl FnOnce() -> i32) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).unwrap_or(E_PANIC)
}

/// The status a callback returns for what the script answered: nothing is `E_OK`, a boolean is 1 or 0 (a drag taken or declined),
/// a number is itself, and a failure is `E_PANIC`.
pub(crate) fn status_of(result: &Result<Dyn, String>) -> i32 {
    match result {
        Err(_) => E_PANIC,
        Ok(Dyn::Bool(taken)) => i32::from(*taken),
        Ok(Dyn::Int(number)) => i32::try_from(*number).unwrap_or(i32::MAX),
        Ok(Dyn::Uint(number)) => i32::try_from(*number).unwrap_or(i32::MAX),
        Ok(_) => E_OK,
    }
}

/// Copies the text a record lends.
///
/// # Safety
///
/// `text` must be null with length zero, or point at `len` readable bytes.
pub(crate) unsafe fn copy_str(text: gnoblin_core_abi::GnoblinStr) -> String {
    if text.ptr.is_null() || text.len == 0 {
        return String::new();
    }
    // SAFETY: the caller guarantees `ptr` is readable for `len` bytes.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text.ptr, text.len) }).into_owned()
}

/// The array a record lends.
///
/// # Safety
///
/// `ptr` must be null, or point at `len` readable values that outlive `'a`.
pub(crate) unsafe fn slice_of<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller guarantees `ptr` is readable for `len` values.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

#[cfg(test)]
mod tests;

//! The pattern functions of the configuration VM: `string.find`, `string.match`,
//! `string.gmatch` and `string.gsub`.
//!
//! The stock functions run the C matcher, which the count hook cannot stop from
//! inside one call. These replacements run [`gnoblin_lua_pattern`] instead, and every
//! step draws from the same instruction budget the hook enforces. The behaviour is
//! meant to match Lua 5.5 exactly: the same results, the same error texts (argument
//! errors included, named by hand as `luaL_argerror` would name them), and the same
//! `init` and `max_n` handling. `tests/pattern_parity.rs` checks that against the
//! stock functions.
//!
//! Number-to-text and number-parsing go through the stock `tostring` and `tonumber`,
//! which are captured at install time. That way a number prints in the same format
//! the stock library uses, without reimplementing `%.14g` here.
use std::cell::Cell;
use std::rc::Rc;

use gnoblin_lua_pattern::{PatternError, PatternMatcher, Piece};
use mlua::{Error, Function, Lua, MultiValue, Table, Value};

use crate::{Guard, budget_message};

/// What the installed functions share: the guard whose budget they draw from, the
/// budget size for the error text, and the stock conversion functions.
struct Env {
    guard: Rc<Guard>,
    budget: u64,
    tostring: Function,
    tonumber: Function,
    /// A unique table marking a pattern error in the first result of a Rust function.
    /// Errors cannot be raised from inside a Rust callback without mlua wrapping them,
    /// so the Lua wrapper below raises the message instead, at the caller's position.
    sentinel: Table,
}

/// The Lua half of the four functions. Each Rust function returns `sentinel, message`
/// for a pattern error, and this raises the message with `error(message, 2)`, so the
/// error is a plain string carrying the caller's `chunk:line:` prefix, as the C
/// library's `luaL_error` produces. The position is taken from the caller, which is
/// why the raise happens in Lua and not in the Rust callback. Budget errors never get
/// here: [`surface`] lets them through as real Rust errors.
const WRAPPER: &str = r#"
local sentinel, impl_find, impl_match, impl_gmatch, impl_gsub = ...
local pack, unpack, raise = table.pack, table.unpack, error
local function wrap(impl)
    return function(...)
        local r = pack(impl(...))
        if r[1] == sentinel then raise(r[2], 2) end
        return unpack(r, 1, r.n)
    end
end
local function gmatch(...)
    local r = pack(impl_gmatch(...))
    if r[1] == sentinel then raise(r[2], 2) end
    local iterator = r[1]
    return function(...)
        local q = pack(iterator(...))
        if q[1] == sentinel then raise(q[2], 2) end
        return unpack(q, 1, q.n)
    end
end
return wrap(impl_find), wrap(impl_match), gmatch, wrap(impl_gsub)
"#;

impl Env {
    /// Turn a Rust result into the sentinel form. Budget errors are passed through
    /// unchanged: the guard has latched them, and they must not gain a position prefix
    /// that the hook's message does not have.
    fn surface(&self, lua: &Lua, result: Result<MultiValue, Error>) -> Result<MultiValue, Error> {
        match result {
            Err(Error::RuntimeError(message)) if !self.guard.aborted() => Ok(values(vec![
                Value::Table(self.sentinel.clone()),
                Value::String(lua.create_string(&message)?),
            ])),
            other => other,
        }
    }
}

/// Replace the four pattern functions of the `string` library in `lua`.
///
/// `budget` is the instruction budget the guard was built from. It goes into the
/// exhaustion message, which must read the same as the hook's.
pub(crate) fn install(lua: &Lua, guard: Rc<Guard>, budget: u64) -> Result<(), Error> {
    let globals = lua.globals();
    let string: Table = globals.get("string")?;
    let env = Rc::new(Env {
        guard,
        budget,
        tostring: globals.get("tostring")?,
        tonumber: globals.get("tonumber")?,
        sentinel: lua.create_table()?,
    });

    let find = {
        let env = Rc::clone(&env);
        lua.create_function(move |lua, args: MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let r = find_aux(lua, &env, &args, SearchKind::Find);
            env.surface(lua, r)
        })?
    };
    let matched = {
        let env = Rc::clone(&env);
        lua.create_function(move |lua, args: MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let r = find_aux(lua, &env, &args, SearchKind::Match);
            env.surface(lua, r)
        })?
    };
    let gmatch = {
        let env = Rc::clone(&env);
        lua.create_function(move |lua, args: MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let r = gmatch_new(lua, &env, &args);
            env.surface(lua, r)
        })?
    };
    let gsub = {
        let env = Rc::clone(&env);
        lua.create_function(move |lua, args: MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let r = gsub_aux(lua, &env, &args);
            env.surface(lua, r)
        })?
    };

    // The wrapper is a chunk of its own. It is loaded with a fixed name so error
    // positions inside it never matter: the raise points at the caller, level 2.
    let (wrap_find, wrap_match, wrap_gmatch, wrap_gsub): (Function, Function, Function, Function) = lua
        .load(WRAPPER)
        .set_name("=gnoblin-strlib")
        .call((env.sentinel.clone(), find, matched, gmatch, gsub))?;
    string.set("find", wrap_find)?;
    string.set("match", wrap_match)?;
    string.set("gmatch", wrap_gmatch)?;
    string.set("gsub", wrap_gsub)
}

/// Which of the two search functions is running. They share one body.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchKind {
    Find,
    Match,
}

impl SearchKind {
    /// The name `luaL_argerror` would print for this function.
    fn name(self) -> &'static str {
        match self {
            SearchKind::Find => "find",
            SearchKind::Match => "match",
        }
    }
}

/// `str_find_aux`: the body of `string.find` and `string.match`.
fn find_aux(lua: &Lua, env: &Env, args: &[Value], mode: SearchKind) -> Result<MultiValue, Error> {
    let name = mode.name();
    let subject = check_string(env, args, 0, name)?;
    let pattern = check_string(env, args, 1, name)?;
    let init = posrelat_i(opt_integer(env, args, 2, name, 1)?, subject.len()) - 1;
    if init > subject.len() {
        // A start past the end finds nothing, not even an empty match.
        return Ok(nil(lua));
    }

    // A plain search skips the matcher entirely. For `find` only, as in the C library:
    // `match` has no plain flag, so the fourth argument is ignored there. A pattern
    // with no special characters is also searched plainly, so a malformed one such as
    // ")" is not an error when it cannot match at all.
    let plain_flag = truthy(args.get(3));
    let no_specials = !pattern.iter().any(|c| b"^$*+?.([%-".contains(c));
    if mode == SearchKind::Find && (plain_flag || no_specials) {
        let found = if pattern.is_empty() {
            Some(init)
        } else {
            subject[init..]
                .windows(pattern.len())
                .position(|w| w == &pattern[..])
                .map(|i| init + i)
        };
        return Ok(match found {
            Some(start) => values(vec![
                Value::Integer(start as i64 + 1),
                Value::Integer((start + pattern.len()) as i64),
            ]),
            None => nil(lua),
        });
    }

    let anchor = pattern.first() == Some(&b'^');
    let pat = if anchor { &pattern[1..] } else { &pattern[..] };
    let mut matcher = PatternMatcher::new(&subject, pat, &env.guard.remaining);
    let mut start = init;
    loop {
        if let Some(end) = lift(env, matcher.match_here(start))? {
            let mut out = Vec::new();
            match mode {
                SearchKind::Find => {
                    out.push(Value::Integer(start as i64 + 1));
                    out.push(Value::Integer(end as i64));
                    for piece in lift(env, matcher.captures(None))? {
                        out.push(piece_value(lua, &subject, piece)?);
                    }
                }
                SearchKind::Match => {
                    for piece in lift(env, matcher.captures(Some(start..end)))? {
                        out.push(piece_value(lua, &subject, piece)?);
                    }
                }
            }
            return Ok(values(out));
        }
        // An anchored pattern tries only the start position. Otherwise every start is
        // tried in turn, including the position just past the last byte.
        if anchor || start >= subject.len() {
            break;
        }
        start += 1;
    }
    Ok(nil(lua))
}

/// `gmatch`: check the arguments and return the iterator closure.
///
/// The iterator owns copies of the subject and pattern, so it does not depend on the
/// arguments still being alive, as in the C version.
fn gmatch_new(lua: &Lua, env: &Rc<Env>, args: &[Value]) -> Result<MultiValue, Error> {
    let subject = check_string(env, args, 0, "gmatch")?;
    let pattern = check_string(env, args, 1, "gmatch")?;
    let mut init = posrelat_i(opt_integer(env, args, 2, "gmatch", 1)?, subject.len()) - 1;
    // A start past the end is clamped to one past the end, so the iterator finds nothing.
    if init > subject.len() {
        init = subject.len() + 1;
    }
    let state = Rc::new(GmatchState {
        env: Rc::clone(env),
        subject,
        pattern,
        src: Cell::new(init),
        lastmatch: Cell::new(None),
    });
    let iterator = lua.create_function(move |lua, _: MultiValue| {
        let r = gmatch_step(lua, &state);
        state.env.surface(lua, r)
    })?;
    Ok(values(vec![Value::Function(iterator)]))
}

/// The state one `gmatch` iterator carries between calls.
struct GmatchState {
    env: Rc<Env>,
    subject: Vec<u8>,
    pattern: Vec<u8>,
    /// Where the next scan starts.
    src: Cell<usize>,
    /// The end of the last match. An empty match at that end is skipped, so the
    /// iterator always advances.
    lastmatch: Cell<Option<usize>>,
}

/// `gmatch_aux`: find the next match and return its captures, or nothing when done.
fn gmatch_step(lua: &Lua, state: &GmatchState) -> Result<MultiValue, Error> {
    let mut matcher = PatternMatcher::new(&state.subject, &state.pattern, &state.env.guard.remaining);
    let mut start = state.src.get();
    while start <= state.subject.len() {
        // A match that ends where the previous one ended is skipped (5.4+ rule).
        if let Some(end) = lift(&state.env, matcher.match_here(start))?
            && Some(end) != state.lastmatch.get()
        {
            state.src.set(end);
            state.lastmatch.set(Some(end));
            let mut out = Vec::new();
            for piece in lift(&state.env, matcher.captures(Some(start..end)))? {
                out.push(piece_value(lua, &state.subject, piece)?);
            }
            return Ok(values(out));
        }
        start += 1;
    }
    // Nothing more: return no values at all, as the C iterator does.
    Ok(values(Vec::new()))
}

/// `str_gsub`: substitute matches of the pattern in the subject.
fn gsub_aux(lua: &Lua, env: &Env, args: &[Value]) -> Result<MultiValue, Error> {
    let name = "gsub";
    let subject = check_string(env, args, 0, name)?;
    let pattern = check_string(env, args, 1, name)?;
    let repl = match args.get(2) {
        Some(
            value @ (Value::String(_) | Value::Integer(_) | Value::Number(_) | Value::Function(_) | Value::Table(_)),
        ) => value.clone(),
        other => {
            return Err(arg_error(
                3,
                name,
                &format!("string/function/table expected, got {}", type_of(other)),
            ));
        }
    };
    let max_n = opt_integer(env, args, 3, name, subject.len() as i64 + 1)?;
    let anchor = pattern.first() == Some(&b'^');
    let pat = if anchor { &pattern[1..] } else { &pattern[..] };

    let mut matcher = PatternMatcher::new(&subject, pat, &env.guard.remaining);
    let mut out = Vec::with_capacity(subject.len());
    let mut pos = 0usize;
    let mut lastmatch = None;
    let mut count: i64 = 0;
    let mut changed = false;
    while count < max_n {
        match lift(env, matcher.match_here(pos))? {
            Some(end) if Some(end) != lastmatch => {
                count += 1;
                changed |= add_value(lua, env, &matcher, &mut out, &subject, pos, end, &repl)?;
                pos = end;
                lastmatch = Some(end);
            }
            _ => {
                // No match here, or an empty match at the end of the last one: copy one
                // byte and try again. At the end of the subject there is nothing left.
                if pos < subject.len() {
                    out.push(subject[pos]);
                    pos += 1;
                } else {
                    break;
                }
            }
        }
        if anchor {
            break;
        }
    }

    // With no replacement the stock function returns the subject value itself, so a
    // number subject comes back as a number.
    let result = if changed {
        out.extend_from_slice(&subject[pos..]);
        Value::String(lua.create_string(&out)?)
    } else {
        args[0].clone()
    };
    Ok(values(vec![result, Value::Integer(count)]))
}

/// `add_value`: append the replacement for the match `s..e`. Returns whether the
/// subject changed, which is false when the table or function gives nothing.
#[allow(clippy::too_many_arguments)]
fn add_value(
    lua: &Lua,
    env: &Env,
    matcher: &PatternMatcher<'_>,
    out: &mut Vec<u8>,
    subject: &[u8],
    s: usize,
    e: usize,
    repl: &Value,
) -> Result<bool, Error> {
    match repl {
        Value::Function(function) => {
            let mut args = Vec::new();
            for piece in lift(env, matcher.captures(Some(s..e)))? {
                args.push(piece_value(lua, subject, piece)?);
            }
            let result: Value = function.call(values(args))?;
            finish_value(env, out, subject, s, e, result)
        }
        Value::Table(table) => {
            // The first capture (or the whole match) is the key.
            let key = piece_value(lua, subject, lift(env, matcher.capture(0, &(s..e)))?)?;
            let result: Value = table.get(key)?;
            finish_value(env, out, subject, s, e, result)
        }
        other => {
            let text = match other {
                Value::String(string) => string.as_bytes().to_vec(),
                number => number_bytes(env, number.clone())?,
            };
            add_s(env, matcher, out, subject, s, e, &text)?;
            Ok(true)
        }
    }
}

/// Turn a function or table result into output. `nil` and `false` keep the original
/// match, and anything that is not a string or number is an error.
fn finish_value(
    env: &Env,
    out: &mut Vec<u8>,
    subject: &[u8],
    s: usize,
    e: usize,
    result: Value,
) -> Result<bool, Error> {
    match result {
        Value::Nil | Value::Boolean(false) => {
            out.extend_from_slice(&subject[s..e]);
            Ok(false)
        }
        Value::String(string) => {
            out.extend_from_slice(&string.as_bytes());
            Ok(true)
        }
        number @ (Value::Integer(_) | Value::Number(_)) => {
            let text = number_bytes(env, number)?;
            out.extend_from_slice(&text);
            Ok(true)
        }
        other => Err(Error::runtime(format!(
            "invalid replacement value (a {})",
            type_name(&other)
        ))),
    }
}

/// `add_s`: expand a replacement string, with `%0`..`%9` and `%%`.
fn add_s(
    env: &Env,
    matcher: &PatternMatcher<'_>,
    out: &mut Vec<u8>,
    subject: &[u8],
    s: usize,
    e: usize,
    repl: &[u8],
) -> Result<(), Error> {
    let mut i = 0;
    while i < repl.len() {
        let c = repl[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        let d = repl.get(i).copied().unwrap_or(0);
        i += 1;
        if d == b'%' {
            out.push(b'%');
        } else if d == b'0' {
            out.extend_from_slice(&subject[s..e]);
        } else if d.is_ascii_digit() {
            match lift(env, matcher.capture((d - b'1') as usize, &(s..e)))? {
                Piece::Text(range) => out.extend_from_slice(&subject[range]),
                Piece::Position(n) => out.extend_from_slice(n.to_string().as_bytes()),
            }
        } else {
            return Err(Error::runtime("invalid use of '%' in replacement string"));
        }
    }
    Ok(())
}

/// Check that argument `i` (0-based) is a string or number and return its bytes.
///
/// A number is converted by the stock `tostring`, so it prints exactly as Lua does.
fn check_string(env: &Env, args: &[Value], i: usize, name: &str) -> Result<Vec<u8>, Error> {
    match args.get(i) {
        Some(Value::String(string)) => Ok(string.as_bytes().to_vec()),
        Some(number @ (Value::Integer(_) | Value::Number(_))) => number_bytes(env, number.clone()),
        other => Err(arg_error(
            i + 1,
            name,
            &format!("string expected, got {}", type_of(other)),
        )),
    }
}

/// Check that argument `i` is an integer, with `default` when it is absent or nil.
///
/// Floats must have an exact integer value, and strings are converted by the stock
/// `tonumber`, so the accepted inputs match `luaL_optinteger`.
fn opt_integer(env: &Env, args: &[Value], i: usize, name: &str, default: i64) -> Result<i64, Error> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(default),
        Some(Value::Integer(n)) => Ok(*n),
        Some(Value::Number(f)) => float_to_integer(*f),
        Some(Value::String(_)) => {
            let converted: Value = env.tonumber.call(args[i].clone())?;
            match converted {
                Value::Integer(n) => Ok(n),
                Value::Number(f) => float_to_integer(f),
                _ => Err(arg_error(i + 1, name, "number expected, got string")),
            }
        }
        other => Err(arg_error(
            i + 1,
            name,
            &format!("number expected, got {}", type_of(other)),
        )),
    }
}

/// The integer value of `f`, or the error `luaL_checkinteger` gives for a float with
/// a fractional part or out of range.
fn float_to_integer(f: f64) -> Result<i64, Error> {
    // `i64::MAX` is not exactly representable, so the upper bound is the first float
    // above it. Any float below that bound with no fraction fits.
    if f.fract() == 0.0 && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f) {
        Ok(f as i64)
    } else {
        Err(Error::runtime("number has no integer representation"))
    }
}

/// `posrelatI`: turn a 1-based start that may be negative into an absolute position
/// of at least 1, clipping the negative positions that run past the front.
fn posrelat_i(pos: i64, len: usize) -> usize {
    if pos > 0 {
        pos as usize
    } else if pos == 0 || pos < -(len as i64) {
        // `0` and any position before the front both clip to the first byte.
        1
    } else {
        (len as i64 + pos + 1) as usize
    }
}

/// Convert a number to text with the stock `tostring`.
fn number_bytes(env: &Env, number: Value) -> Result<Vec<u8>, Error> {
    let text: Value = env.tostring.call(number)?;
    match text {
        Value::String(string) => Ok(string.as_bytes().to_vec()),
        _ => Err(Error::runtime("'tostring' must return a string")),
    }
}

/// Lift a matcher result into a Lua error. A budget exhaustion latches the guard with
/// the same text the hook uses, so `pcall` cannot swallow it.
fn lift<T>(env: &Env, result: Result<T, PatternError>) -> Result<T, Error> {
    result.map_err(|error| match error {
        PatternError::Message(message) => Error::runtime(message),
        PatternError::Budget => env.guard.latch(budget_message(env.budget)),
    })
}

/// The value of one capture piece as a Lua value.
fn piece_value(lua: &Lua, subject: &[u8], piece: Piece) -> Result<Value, Error> {
    match piece {
        Piece::Text(range) => Ok(Value::String(lua.create_string(&subject[range])?)),
        Piece::Position(n) => Ok(Value::Integer(n as i64)),
    }
}

/// An argument error as `luaL_argerror` words it.
fn arg_error(position: usize, name: &str, detail: &str) -> Error {
    Error::runtime(format!("bad argument #{position} to '{name}' ({detail})"))
}

/// The type name `luaL_typename` reports, or `no value` for an absent argument.
fn type_of(value: Option<&Value>) -> &'static str {
    match value {
        None => "no value",
        Some(value) => type_name(value),
    }
}

/// The Lua type name of a value.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::Integer(_) | Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::Thread(_) => "thread",
        _ => "userdata",
    }
}

/// Lua truthiness: only `nil` and `false` are false.
fn truthy(value: Option<&Value>) -> bool {
    !matches!(value, None | Some(Value::Nil) | Some(Value::Boolean(false)))
}

/// A single `nil` result, as `luaL_pushfail` leaves it.
fn nil(lua: &Lua) -> MultiValue {
    let _ = lua;
    values(vec![Value::Nil])
}

/// Collect values into the multiple-value form a callback returns.
fn values(items: Vec<Value>) -> MultiValue {
    items.into_iter().collect()
}

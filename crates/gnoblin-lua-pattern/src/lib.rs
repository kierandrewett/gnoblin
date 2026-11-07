//! A Lua 5.4 pattern matcher for `string.find`, `string.gmatch` and `string.gsub`.
//!
//! The Lua sandbox does not run the C matcher. A C pattern match cannot be interrupted
//! by the count hook, so one `string.gsub` over a large subject could run past every
//! budget. This crate is a line-for-line port of the C matcher with one addition: every
//! step draws one unit from a shared fuel cell, so a match stops when its share of the
//! instruction budget runs out.
//!
//! The port keeps the C structure on purpose: the same `match` dispatch, the same
//! recursion limit, the same capture bookkeeping and the same error texts. That is what
//! lets the sandbox's parity tests compare results with stock Lua.
//!
//! Positions are byte offsets, and `src.len()` plays the part of C's `src_end`. The C code
//! reads the byte after a string's end as a NUL; `byte_at` and `PatternMatcher::at` do
//! the same, so the two ports take the same branches.
use std::cell::Cell;
use std::fmt;
use std::ops::Range;

/// The most captures one pattern may open (`LUA_MAXCAPTURES` in `lstrlib.c`).
const MAX_CAPTURES: usize = 32;

/// How deeply `match` may recurse (`MAXCCALLS` in `lstrlib.c`). A deeper pattern fails
/// with "pattern too complex" rather than exhausting the Rust stack.
const MAX_DEPTH: u32 = 200;

/// The length of a capture whose closing `)` has not been seen yet.
const CAP_UNFINISHED: isize = -1;

/// The length of a position capture `()`, which records an index, not a substring.
const CAP_POSITION: isize = -2;

/// Why a match stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// A message `luaL_error` would raise, such as `malformed pattern (missing ']')`.
    Message(String),
    /// The fuel ran out. The caller turns this into the same budget error the count
    /// hook raises, so a caught error still names the budget.
    Budget,
}

/// The result of one matcher operation.
pub type Result<T, E = PatternError> = std::result::Result<T, E>;

/// Fail with a `luaL_error`-style message.
fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(PatternError::Message(message.into()))
}

/// One capture: where it starts in the subject and how long it is, or one of the
/// `CAP_*` markers.
#[derive(Clone, Copy, Debug, Default)]
struct PatternCapture {
    init: usize,
    len: isize,
}

/// One value a successful match yields to the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// A substring, given as a byte range of the subject.
    Text(Range<usize>),
    /// A position capture. It is 1-based, as Lua shows it.
    Position(usize),
}

/// The matcher state for one subject and one pattern.
///
/// A `PatternMatcher` is cheap to build. Callers reuse one for a whole `gsub` or `gmatch`
/// loop, calling [`PatternMatcher::match_here`] at each start position, which resets the
/// per-attempt state the way `reprepstate` does in C.
pub struct PatternMatcher<'a> {
    src: &'a [u8],
    pat: &'a [u8],
    /// Steps the caller has left. Each step costs one unit, and the matcher stops
    /// with [`PatternError::Budget`] when this reaches zero.
    fuel: &'a Cell<u64>,
    /// Recursion budget left (`matchdepth` in C).
    depth: u32,
    /// Captures opened so far (`level` in C).
    level: usize,
    captures: [PatternCapture; MAX_CAPTURES],
}

impl<'a> PatternMatcher<'a> {
    /// A matcher for `pat` over `src`, drawing steps from `fuel`.
    pub fn new(src: &'a [u8], pat: &'a [u8], fuel: &'a Cell<u64>) -> PatternMatcher<'a> {
        PatternMatcher {
            src,
            pat,
            fuel,
            depth: MAX_DEPTH,
            level: 0,
            captures: [PatternCapture::default(); MAX_CAPTURES],
        }
    }

    /// Try to match the whole pattern starting at subject index `s`.
    ///
    /// On success it returns the end index and leaves the captures in place for
    /// [`PatternMatcher::captures`] or [`PatternMatcher::capture`].
    pub fn match_here(&mut self, s: usize) -> Result<Option<usize>> {
        self.depth = MAX_DEPTH;
        self.level = 0;
        self.match_fn(s, 0)
    }

    /// The values of a successful match, as `push_captures` builds them.
    ///
    /// `whole` is the matched range. With no captures it stands for capture 1. A find
    /// passes `None`, which reports only the explicit captures.
    pub fn captures(&self, whole: Option<Range<usize>>) -> Result<Vec<Piece>> {
        let count = if self.level == 0 && whole.is_some() {
            1
        } else {
            self.level
        };
        let whole = whole.unwrap_or(0..0);
        (0..count).map(|i| self.capture(i, &whole)).collect()
    }

    /// PatternCapture `i` (0-based) of the current match, as `get_onecapture` gives it.
    ///
    /// `whole` is the range that capture 0 means when the pattern has no captures.
    pub fn capture(&self, i: usize, whole: &Range<usize>) -> Result<Piece> {
        if i >= self.level {
            if i != 0 {
                return fail(format!("invalid capture index %{}", i + 1));
            }
            return Ok(Piece::Text(whole.clone()));
        }
        let cap = self.captures[i];
        if cap.len == CAP_UNFINISHED {
            return fail("unfinished capture");
        }
        if cap.len == CAP_POSITION {
            return Ok(Piece::Position(cap.init + 1));
        }
        let len = cap.len as usize;
        Ok(Piece::Text(cap.init..cap.init + len))
    }

    /// Spend one step, or stop when the fuel is gone.
    fn charge(&self) -> Result<()> {
        let left = self.fuel.get();
        if left == 0 {
            return Err(PatternError::Budget);
        }
        self.fuel.set(left - 1);
        Ok(())
    }

    /// The pattern byte at `i`, or NUL past its end, as the C string would read.
    fn at(&self, i: usize) -> u8 {
        byte_at(self.pat, i)
    }

    /// The subject byte at `i`, or NUL past its end, as the C string would read.
    fn src_at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// The C `match` function: match the pattern from index `p` at subject index `s`.
    ///
    /// The recursion budget is taken on entry and given back on exit, exactly as the
    /// C code does with `matchdepth`.
    fn match_fn(&mut self, s: usize, p: usize) -> Result<Option<usize>> {
        if self.depth == 0 {
            return fail("pattern too complex");
        }
        self.depth -= 1;
        let result = self.step(s, p);
        self.depth += 1;
        result
    }

    /// The body of `match`. The C code uses `goto init` for tail calls; here that is a
    /// `loop` that rewrites `s` and `p` and continues.
    fn step(&mut self, mut s: usize, mut p: usize) -> Result<Option<usize>> {
        self.charge()?;
        loop {
            if p == self.pat.len() {
                return Ok(Some(s));
            }
            match self.pat[p] {
                b'(' => {
                    return if self.at(p + 1) == b')' {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                // A `$` is an anchor only as the last byte of the pattern.
                b'$' if p + 1 == self.pat.len() => {
                    return Ok((s == self.src.len()).then_some(s));
                }
                b'%' if self.at(p + 1) == b'b' => match self.match_balance(s, p + 2)? {
                    Some(next) => {
                        s = next;
                        p += 4;
                    }
                    None => return Ok(None),
                },
                b'%' if self.at(p + 1) == b'f' => {
                    p += 2;
                    if self.at(p) != b'[' {
                        return fail("missing '[' after '%f' in pattern");
                    }
                    let ep = class_end(self.pat, p)?;
                    // The frontier is a zero-width test: the byte before `s` must be
                    // outside the set and the byte at `s` inside it. NUL stands in at
                    // both ends of the subject.
                    let previous = if s == 0 { 0 } else { self.src[s - 1] };
                    let current = self.src_at(s);
                    if !self.bracket_class(previous, p, ep - 1) && self.bracket_class(current, p, ep - 1) {
                        p = ep;
                    } else {
                        return Ok(None);
                    }
                }
                b'%' if self.at(p + 1).is_ascii_digit() => match self.match_capture(s, self.at(p + 1))? {
                    Some(next) => {
                        s = next;
                        p += 2;
                    }
                    None => return Ok(None),
                },
                _ => {
                    // A single-class item, with an optional repetition suffix.
                    let ep = class_end(self.pat, p)?;
                    let suffix = self.at(ep);
                    if !self.single_match(s, p, ep) {
                        if matches!(suffix, b'*' | b'?' | b'-') {
                            p = ep + 1;
                        } else {
                            return Ok(None);
                        }
                    } else {
                        match suffix {
                            b'?' => {
                                if let Some(end) = self.match_fn(s + 1, ep + 1)? {
                                    return Ok(Some(end));
                                }
                                p = ep + 1;
                            }
                            b'+' => return self.max_expand(s + 1, p, ep),
                            b'*' => return self.max_expand(s, p, ep),
                            b'-' => return self.min_expand(s, p, ep),
                            _ => {
                                s += 1;
                                p = ep;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Open a capture of kind `what` at `s`, then match the rest of the pattern.
    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> Result<Option<usize>> {
        if self.level >= MAX_CAPTURES {
            return fail("too many captures");
        }
        self.captures[self.level] = PatternCapture { init: s, len: what };
        self.level += 1;
        let result = self.match_fn(s, p)?;
        if result.is_none() {
            self.level -= 1;
        }
        Ok(result)
    }

    /// Close the innermost open capture at `s`, then match the rest of the pattern.
    fn end_capture(&mut self, s: usize, p: usize) -> Result<Option<usize>> {
        let l = self.capture_to_close()?;
        let init = self.captures[l].init;
        self.captures[l].len = (s - init) as isize;
        let result = self.match_fn(s, p)?;
        if result.is_none() {
            self.captures[l].len = CAP_UNFINISHED;
        }
        Ok(result)
    }

    /// The index of the innermost capture that is still open.
    fn capture_to_close(&self) -> Result<usize> {
        for level in (0..self.level).rev() {
            if self.captures[level].len == CAP_UNFINISHED {
                return Ok(level);
            }
        }
        fail("invalid pattern capture")
    }

    /// Check a back-reference digit `%1`..`%9` and return its capture index.
    fn check_capture(&self, digit: u8) -> Result<usize> {
        let index = isize::from(digit) - isize::from(b'1');
        if index < 0 || index as usize >= self.level || self.captures[index as usize].len == CAP_UNFINISHED {
            return fail(format!("invalid capture index %{}", index + 1));
        }
        Ok(index as usize)
    }

    /// Match a back-reference `%n`: the subject must repeat capture `n` at `s`.
    fn match_capture(&self, s: usize, digit: u8) -> Result<Option<usize>> {
        let index = self.check_capture(digit)?;
        let cap = self.captures[index];
        // A position capture has no text to repeat, so it never matches (C compares
        // against a huge unsigned length).
        if cap.len < 0 {
            return Ok(None);
        }
        let len = cap.len as usize;
        if self.src.len() - s >= len && self.src[cap.init..cap.init + len] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    /// Match `%bxy` at `s`, where `p` points at the `x` byte.
    fn match_balance(&self, s: usize, p: usize) -> Result<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return fail("malformed pattern (missing arguments to '%b')");
        }
        let open = self.at(p);
        let close = self.at(p + 1);
        if self.src_at(s) != open {
            return Ok(None);
        }
        let mut depth = 1u32;
        let mut s = s;
        loop {
            s += 1;
            if s >= self.src.len() {
                return Ok(None);
            }
            self.charge()?;
            let c = self.src[s];
            if c == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(s + 1));
                }
            } else if c == open {
                depth += 1;
            }
        }
    }

    /// Whether subject byte `s` matches the single-class item `p..ep`.
    fn single_match(&self, s: usize, p: usize, ep: usize) -> bool {
        if s >= self.src.len() {
            return false;
        }
        let c = self.src[s];
        match self.at(p) {
            b'.' => true,
            b'%' => match_class(c, self.at(p + 1)),
            b'[' => self.bracket_class(c, p, ep - 1),
            pc => pc == c,
        }
    }

    /// Whether byte `c` is in the set `[...]` that starts at `p` and ends at `ec`.
    ///
    /// `p` points at the `[` and `ec` at the closing `]`.
    fn bracket_class(&self, c: u8, p: usize, ec: usize) -> bool {
        let mut sig = true;
        let mut p = p;
        if self.at(p + 1) == b'^' {
            sig = false;
            p += 1;
        }
        loop {
            p += 1;
            if p >= ec {
                break;
            }
            if self.at(p) == b'%' {
                p += 1;
                if match_class(c, self.at(p)) {
                    return sig;
                }
            } else if self.at(p + 1) == b'-' && p + 2 < ec {
                p += 2;
                if self.at(p - 2) <= c && c <= self.at(p) {
                    return sig;
                }
            } else if self.at(p) == c {
                return sig;
            }
        }
        !sig
    }

    /// Match as many copies of the item as possible, then back off until the rest of
    /// the pattern matches (`max_expand`).
    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        let mut count = 0usize;
        while self.single_match(s + count, p, ep) {
            self.charge()?;
            count += 1;
        }
        loop {
            if let Some(end) = self.match_fn(s + count, ep + 1)? {
                return Ok(Some(end));
            }
            if count == 0 {
                return Ok(None);
            }
            count -= 1;
        }
    }

    /// Match as few copies of the item as possible, growing one at a time
    /// (`min_expand`).
    fn min_expand(&mut self, s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        let mut s = s;
        loop {
            if let Some(end) = self.match_fn(s, ep + 1)? {
                return Ok(Some(end));
            }
            if self.single_match(s, p, ep) {
                self.charge()?;
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }
}

/// The byte at `i` in `bytes`, or NUL past its end, as the C string would read it.
fn byte_at(bytes: &[u8], i: usize) -> u8 {
    bytes.get(i).copied().unwrap_or(0)
}

/// The index just past the single-class item that starts at `p` (`classEnd` in C).
///
/// A `%` or `[` item that runs off the end of the pattern is refused here. The walk in
/// [`validate`] calls this too, so a pattern refused at load is refused by the same code
/// that would fail it in use.
fn class_end(pat: &[u8], p: usize) -> Result<usize> {
    let c = byte_at(pat, p);
    let mut p = p + 1;
    match c {
        b'%' => {
            if p == pat.len() {
                return fail("malformed pattern (ends with '%')");
            }
            Ok(p + 1)
        }
        b'[' => {
            if byte_at(pat, p) == b'^' {
                p += 1;
            }
            // The first byte after `[` (or `[^`) is taken literally, so `[]]` is a
            // set of `]`. The loop always consumes one byte before it looks for `]`.
            loop {
                if p == pat.len() {
                    return fail("malformed pattern (missing ']')");
                }
                let ch = pat[p];
                p += 1;
                if ch == b'%' && p < pat.len() {
                    p += 1;
                }
                if byte_at(pat, p) == b']' {
                    break;
                }
            }
            Ok(p + 1)
        }
        _ => Ok(p),
    }
}

/// Whether byte `c` belongs to the class named by the letter `cl` (`%a`, `%D`, ...).
///
/// An upper-case letter negates the class. A byte that is not a class letter matches
/// only itself. The classes follow the C locale, so they are ASCII only.
fn match_class(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        // C `isspace`: space, and the control bytes \t \n \v \f \r. Rust's
        // `is_ascii_whitespace` leaves out \v, so spell the range out.
        b's' => c == b' ' || (b'\t'..=b'\r').contains(&c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        // `%z` is the deprecated NUL class, kept for old patterns.
        b'z' => c == 0,
        _ => return cl == c,
    };
    if cl.is_ascii_lowercase() { res } else { !res }
}

/// A pattern the Lua grammar rejects. `offset` is the byte index in the pattern where
/// the problem was found, so a configuration error can point at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternSyntaxError {
    /// What is wrong, in the words `string.find` uses.
    pub reason: String,
    /// The byte index in the pattern.
    pub offset: usize,
}

impl fmt::Display for PatternSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Byte positions are shown one-based, as a person counts them in a config file.
        write!(f, "{} at byte {}", self.reason, self.offset + 1)
    }
}

impl std::error::Error for PatternSyntaxError {}

fn refuse(reason: impl Into<String>, offset: usize) -> PatternSyntaxError {
    PatternSyntaxError {
        reason: reason.into(),
        offset,
    }
}

/// Turns a grammar-walk error into a syntax error at `offset`. The walk never matches,
/// so it spends no fuel and only `Message` errors reach here.
fn syntax_error(error: PatternError, offset: usize) -> PatternSyntaxError {
    let reason = match error {
        PatternError::Message(reason) => reason,
        PatternError::Budget => "pattern too complex".to_owned(),
    };
    refuse(reason, offset)
}

/// Checks a whole pattern against the Lua grammar, without matching anything.
///
/// It refuses every pattern whose grammar error `string.find` raises for some subject.
/// A legacy pattern that only fails on certain titles is therefore refused at load,
/// rather than failing when a window happens to have such a title.
///
/// # Errors
///
/// Returns the first problem found, with its byte offset.
pub fn validate(pattern: &[u8]) -> std::result::Result<(), PatternSyntaxError> {
    // `finished[i]` is whether capture `i` (in order of opening) has its `)`. `open`
    // holds the innermost captures still waiting for theirs, with where each one opened.
    let mut finished: Vec<bool> = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut p = usize::from(pattern.first() == Some(&b'^'));
    while p < pattern.len() {
        match pattern[p] {
            b'(' => {
                if finished.len() >= MAX_CAPTURES {
                    return Err(refuse("too many captures", p));
                }
                if byte_at(pattern, p + 1) == b')' {
                    finished.push(true);
                    p += 2;
                } else {
                    open.push((finished.len(), p));
                    finished.push(false);
                    p += 1;
                }
            }
            b')' => {
                let Some((index, _)) = open.pop() else {
                    return Err(refuse("invalid pattern capture", p));
                };
                finished[index] = true;
                p += 1;
            }
            // A `$` anchors only as the last byte of the pattern; elsewhere it is literal.
            b'$' if p + 1 == pattern.len() => p += 1,
            b'%' if byte_at(pattern, p + 1) == b'b' => {
                // `%bxy` needs both of its arguments.
                if p + 3 >= pattern.len() {
                    return Err(refuse("malformed pattern (missing arguments to '%b')", p));
                }
                p += 4;
            }
            b'%' if byte_at(pattern, p + 1) == b'f' => {
                p += 2;
                if byte_at(pattern, p) != b'[' {
                    return Err(refuse("missing '[' after '%f' in pattern", p));
                }
                let end = class_end(pattern, p).map_err(|e| syntax_error(e, p))?;
                p = end;
            }
            b'%' if byte_at(pattern, p + 1).is_ascii_digit() => {
                let digit = byte_at(pattern, p + 1);
                // `%0` is never a capture, and `%n` needs capture `n` to be closed already.
                let closed = digit != b'0' && finished.get(usize::from(digit - b'1')) == Some(&true);
                if !closed {
                    return Err(refuse(format!("invalid capture index %{}", char::from(digit)), p));
                }
                p += 2;
            }
            _ => {
                let end = class_end(pattern, p).map_err(|e| syntax_error(e, p))?;
                // A repetition suffix belongs to the item it follows.
                p = if matches!(byte_at(pattern, end), b'?' | b'*' | b'+' | b'-') {
                    end + 1
                } else {
                    end
                };
            }
        }
    }
    if let Some(&(_, at)) = open.last() {
        return Err(refuse("unfinished capture", at));
    }
    Ok(())
}

/// Whether `pattern` matches somewhere in `subject` (or only at its start, when anchored),
/// within `budget` steps.
///
/// The pattern should already have passed [`validate`]. A budget that runs out is
/// [`PatternError::Budget`]; a pattern that recurses too deeply is the "pattern too
/// complex" message. The caller decides what either one means.
///
/// # Errors
///
/// [`PatternError::Budget`] when the steps run out, and [`PatternError::Message`] when the
/// pattern recurses too deeply or was not validated and is malformed.
pub fn find(pattern: &[u8], subject: &[u8], budget: u64) -> Result<bool> {
    let fuel = Cell::new(budget);
    // The `^` anchor is stripped before matching, as `str_find_aux` does in C.
    let anchored = pattern.first() == Some(&b'^');
    let body = if anchored { &pattern[1..] } else { pattern };
    let mut matcher = PatternMatcher::new(subject, body, &fuel);
    let last = if anchored { 0 } else { subject.len() };
    for start in 0..=last {
        if matcher.match_here(start)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `find` with a budget large enough that only the pattern can stop it.
    fn finds(pattern: &str, subject: &str) -> bool {
        validate(pattern.as_bytes()).expect("test pattern is valid");
        find(pattern.as_bytes(), subject.as_bytes(), 1 << 24).expect("test match completes")
    }

    fn syntax(pattern: &str) -> PatternSyntaxError {
        validate(pattern.as_bytes()).expect_err("test pattern should be refused")
    }

    #[test]
    fn plain_text_matches_anywhere_and_anchors_pin_the_ends() {
        assert!(finds("Notes", "My Notes - Work"));
        assert!(!finds("Notes", "note"));
        assert!(finds("^Notes", "Notes - Work"));
        assert!(!finds("^Notes", "My Notes"));
        assert!(finds("Notes$", "My Notes"));
        assert!(!finds("Notes$", "Notes - Work"));
        assert!(finds("^Notes$", "Notes"));
    }

    #[test]
    fn a_dollar_that_is_not_last_is_a_literal() {
        assert!(finds("a$b", "a$b"));
        assert!(!finds("a$b", "ab"));
    }

    #[test]
    fn the_empty_pattern_matches_everything() {
        assert!(finds("", ""));
        assert!(finds("", "anything"));
    }

    #[test]
    fn a_dot_matches_one_byte_and_percent_escapes_a_literal() {
        assert!(finds("org.gnome.Nautilus", "orgxgnomeyNautilus"));
        assert!(finds("^org%.gnome%.Nautilus$", "org.gnome.Nautilus"));
        assert!(!finds("^org%.gnome%.Nautilus$", "orgxgnomexNautilus"));
        assert!(finds("100%%", "100%"));
    }

    #[test]
    fn a_trailing_percent_is_refused_at_load() {
        let error = syntax("done%");
        assert_eq!(error.reason, "malformed pattern (ends with '%')");
        assert_eq!(error.offset, 4);
        assert_eq!(error.to_string(), "malformed pattern (ends with '%') at byte 5");
    }

    #[test]
    fn character_classes_follow_the_c_locale() {
        assert!(finds("^%d+$", "2024"));
        assert!(!finds("^%d+$", "20a4"));
        assert!(finds("^%a%w*$", "Abc123"));
        assert!(finds("%s", "a\x0bb"), "\\v is whitespace in C isspace");
        assert!(finds("^%D+$", "abc"));
        assert!(!finds("^%D+$", "a1"));
        assert!(finds("^%u%l+$", "Notes"));
        assert!(finds("^%x+$", "dEaD"));
        assert!(finds("^%p+$", "-._"));
    }

    #[test]
    fn sets_hold_ranges_negation_and_a_leading_close_bracket() {
        assert!(finds("^[a-c]+$", "bca"));
        assert!(!finds("^[a-c]+$", "bcd"));
        assert!(finds("^[^%d]+$", "abc"));
        assert!(!finds("^[^%d]+$", "a1"));
        assert!(finds("^[]]$", "]"));
        assert!(finds("^[%a_]+$", "a_b"));
    }

    #[test]
    fn repetition_suffixes_are_greedy_or_lazy_as_in_lua() {
        assert!(finds("^ab*$", "abbb"));
        assert!(finds("^ab+$", "abbb"));
        assert!(!finds("^ab+$", "a"));
        assert!(finds("^ab?$", "a"));
        assert!(finds("^a-b$", "aaab"));
        assert!(!finds("^a-b$", "aaac"));
        assert!(finds("x*", "no x here"), "a star may match nothing");
    }

    #[test]
    fn captures_back_references_and_position_captures() {
        assert!(finds("^(%w+)=(%w+)$", "key=value"));
        assert!(finds("(abc)%1", "abcabc"));
        assert!(!finds("(abc)%1", "abcabd"));
        assert!(finds("^()$", ""), "a position capture matches the empty subject");
    }

    #[test]
    fn balanced_and_frontier_patterns() {
        assert!(finds("%b()", "f(a(b)c)"));
        assert!(!finds("%b()", "f(a(b c"));
        assert!(finds("%f[%a]%a+", "THE (quick) fox"));
        assert!(finds("%f[%w]end%f[%W]", "the end."));
        assert!(!finds("%f[%w]end%f[%W]", "theend."));
    }

    #[test]
    fn malformed_patterns_are_refused_with_their_offset() {
        assert_eq!(syntax("[a").reason, "malformed pattern (missing ']')");
        assert_eq!(syntax("a)").reason, "invalid pattern capture");
        assert_eq!(syntax("(a").reason, "unfinished capture");
        assert_eq!(syntax("%1").reason, "invalid capture index %1");
        assert_eq!(syntax("(a)%2").reason, "invalid capture index %2");
        assert_eq!(syntax("%bx").reason, "malformed pattern (missing arguments to '%b')");
        assert_eq!(syntax("%fa").reason, "missing '[' after '%f' in pattern");
        assert_eq!(syntax("a(b)c)").offset, 5);
    }

    #[test]
    fn a_pattern_with_too_many_captures_is_refused() {
        let pattern = "()".to_owned() + &"(a)".repeat(MAX_CAPTURES);
        assert_eq!(syntax(&pattern).reason, "too many captures");
    }

    #[test]
    fn a_runaway_pattern_stops_at_the_budget() {
        // Nested stars over a long subject take exponential time without a budget.
        let subject = "a".repeat(60) + "b";
        validate(b"a*a*a*a*a*a*a*c").expect("valid");
        let halt = find(b"a*a*a*a*a*a*a*c", subject.as_bytes(), 10_000).expect_err("the budget runs out");
        assert_eq!(halt, PatternError::Budget);
    }

    #[test]
    fn a_pattern_that_recurses_too_deeply_stops_at_the_depth_limit() {
        // Each `a?` recurses once when the subject has an `a` to take.
        let pattern = "a?".repeat(MAX_DEPTH as usize + 10);
        let subject = "a".repeat(MAX_DEPTH as usize + 10);
        validate(pattern.as_bytes()).expect("valid");
        let halt = find(pattern.as_bytes(), subject.as_bytes(), 1 << 24).expect_err("the depth runs out");
        assert_eq!(halt, PatternError::Message("pattern too complex".to_owned()));
    }

    #[test]
    fn a_long_subject_with_a_plain_pattern_is_quick() {
        let subject = "a.".repeat(20_000);
        assert!(finds("^a%.a%.", &subject));
        assert!(!finds("^a%.a%.b$", &subject));
    }
}

//!
//! The configuration VM does not run the C matcher. A C pattern match cannot be
//! interrupted by the count hook, so one `string.gsub` over a large subject could run
//! past every budget. This module is a line-for-line port of the C matcher with one
//! addition: every step draws one unit from a shared fuel cell, so a match stops when
//! its share of the instruction budget runs out.
//!
//! The port keeps the C structure on purpose: the same `match` dispatch, the same
//! recursion limit, the same capture bookkeeping and the same error texts. That is
//! what lets `tests/pattern_parity.rs` compare results with stock Lua.
//!
//! Positions are byte offsets, and `src.len()` plays the part of C's `src_end`. The
//! C code reads the byte after a string's end as a NUL; [`PatternMatcher::at`] and
//! [`PatternMatcher::src_at`] do the same, so the two ports take the same branches.
use std::cell::Cell;
use std::ops::Range;

/// The most captures one pattern may open (`LUA_MAXCAPTURES` in `lstrlib.c`).
pub(crate) const MAX_CAPTURES: usize = 32;

/// How deeply `match` may recurse (`MAXCCALLS` in `lstrlib.c`). A deeper pattern fails
/// with "pattern too complex" rather than exhausting the Rust stack.
const MAX_DEPTH: u32 = 200;

/// The length of a capture whose closing `)` has not been seen yet.
const CAP_UNFINISHED: isize = -1;

/// The length of a position capture `()`, which records an index, not a substring.
const CAP_POSITION: isize = -2;

/// Why a match stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PatternError {
    /// A message `luaL_error` would raise, such as `malformed pattern (missing ']')`.
    Message(String),
    /// The fuel ran out. The caller turns this into the same budget error the count
    /// hook raises, so a caught error still names the budget.
    Budget,
}

/// The result of one matcher operation.
pub(crate) type Result<T, E = PatternError> = std::result::Result<T, E>;

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
pub(crate) enum Piece {
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
pub(crate) struct PatternMatcher<'a> {
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
    pub(crate) fn new(src: &'a [u8], pat: &'a [u8], fuel: &'a Cell<u64>) -> PatternMatcher<'a> {
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
    pub(crate) fn match_here(&mut self, s: usize) -> Result<Option<usize>> {
        self.depth = MAX_DEPTH;
        self.level = 0;
        self.match_fn(s, 0)
    }

    /// The values of a successful match, as `push_captures` builds them.
    ///
    /// `whole` is the matched range. With no captures it stands for capture 1. A find
    /// passes `None`, which reports only the explicit captures.
    pub(crate) fn captures(&self, whole: Option<Range<usize>>) -> Result<Vec<Piece>> {
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
    pub(crate) fn capture(&self, i: usize, whole: &Range<usize>) -> Result<Piece> {
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
        self.pat.get(i).copied().unwrap_or(0)
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
                    let ep = self.class_end(p)?;
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
                    let ep = self.class_end(p)?;
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

    /// The index just past the single-class item that starts at `p`.
    fn class_end(&self, p: usize) -> Result<usize> {
        let c = self.at(p);
        let mut p = p + 1;
        match c {
            b'%' => {
                if p == self.pat.len() {
                    return fail("malformed pattern (ends with '%')");
                }
                Ok(p + 1)
            }
            b'[' => {
                if self.at(p) == b'^' {
                    p += 1;
                }
                // The first byte after `[` (or `[^`) is taken literally, so `[]]` is a
                // set of `]`. The loop always consumes one byte before it looks for `]`.
                loop {
                    if p == self.pat.len() {
                        return fail("malformed pattern (missing ']')");
                    }
                    let ch = self.pat[p];
                    p += 1;
                    if ch == b'%' && p < self.pat.len() {
                        p += 1;
                    }
                    if self.at(p) == b']' {
                        break;
                    }
                }
                Ok(p + 1)
            }
            _ => Ok(p),
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

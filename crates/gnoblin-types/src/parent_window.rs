//! The `parent_window` identifier a portal request carries, typed.
//!
//! An `org.freedesktop.portal.*` request names the window it should be modal to as a
//! string. The format is fixed by the portal specification, "Window identifiers" in the
//! xdg-desktop-portal documentation
//! (<https://flatpak.github.io/xdg-desktop-portal/docs/window-identifiers.html>):
//!
//! - the empty string: no parent window;
//! - `x11:<XID>`: an X11 window, its id written in hexadecimal;
//! - `wayland:<handle>`: a toplevel the caller exported with `xdg_exporter`, named by the
//!   opaque handle the exporter returned.
//!
//! Every other scheme is not one the specification defines. The request still succeeds
//! without a parent, so [`ParentWindow::parse`] reports it as an error and the caller
//! decides what to log; it never becomes a parent it cannot resolve.
//!
//! The type lives here, not in the portal, because the portal backend parses the string
//! and the compositor resolves it, and both must agree on what a value means.

use std::fmt;

/// The longest `wayland:` handle accepted. The compositor mints 32 hex digits; the
/// specification fixes no length, so the bound is generous but finite, which keeps a
/// hostile caller from making the request carry an unbounded string.
pub const MAX_HANDLE_BYTES: usize = 64;

/// The longest `x11:` hexadecimal XID accepted: a 32-bit id is at most eight digits.
pub const MAX_XID_DIGITS: usize = 8;

/// The window a portal request asks to be modal to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParentWindow {
    /// No parent: the request is not tied to a window.
    None,
    /// An X11 window, by its XID. Never zero: `x11:0` names no window.
    X11(u32),
    /// A Wayland toplevel, by the handle its exporter minted. The handle is opaque to the
    /// caller and is looked up in the compositor's handle table, never interpreted here.
    Wayland(String),
}

/// Why a `parent_window` string was refused. Each variant names the rule it broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidParentWindow {
    /// The scheme is neither `x11:` nor `wayland:`, nor the empty string.
    UnknownScheme(String),
    /// `x11:` was followed by nothing, by a non-hex digit, or by more than eight digits.
    InvalidXid(String),
    /// `x11:0`: no X11 window has the id zero.
    ZeroXid,
    /// `wayland:` was followed by nothing, by a character outside ASCII letters, digits,
    /// `-`, `_` or `.`, or by more than `MAX_HANDLE_BYTES` bytes.
    InvalidHandle(String),
}

impl fmt::Display for InvalidParentWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownScheme(raw) => write!(f, "parent_window {raw:?} names no scheme the portal defines"),
            Self::InvalidXid(raw) => write!(f, "parent_window {raw:?} is not an x11 id in hexadecimal"),
            Self::ZeroXid => f.write_str("parent_window x11:0 names no window"),
            Self::InvalidHandle(raw) => write!(f, "parent_window {raw:?} is not a wayland handle"),
        }
    }
}

impl std::error::Error for InvalidParentWindow {}

impl ParentWindow {
    /// Parse the string a portal request carries.
    ///
    /// The scheme is matched exactly, lower case, as the specification writes it. A hex XID
    /// accepts either case of `a` to `f`, because the specification does not fix one.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidParentWindow`] for a scheme the portal does not define, a malformed
    /// or zero XID, or a malformed or overlong handle.
    pub fn parse(raw: &str) -> Result<Self, InvalidParentWindow> {
        if raw.is_empty() {
            return Ok(Self::None);
        }
        if let Some(digits) = raw.strip_prefix("x11:") {
            return parse_xid(digits).map(Self::X11).ok_or_else(|| {
                if !digits.is_empty() && digits.bytes().all(|b| b == b'0') {
                    InvalidParentWindow::ZeroXid
                } else {
                    InvalidParentWindow::InvalidXid(raw.to_owned())
                }
            });
        }
        if let Some(handle) = raw.strip_prefix("wayland:") {
            return parse_handle(handle)
                .map(|handle| Self::Wayland(handle.to_owned()))
                .ok_or_else(|| InvalidParentWindow::InvalidHandle(raw.to_owned()));
        }
        Err(InvalidParentWindow::UnknownScheme(raw.to_owned()))
    }

    /// Whether the request names a parent at all.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Parse the hexadecimal digits of an `x11:` identifier into a non-zero XID.
fn parse_xid(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.len() > MAX_XID_DIGITS || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match u32::from_str_radix(digits, 16) {
        Ok(0) | Err(_) => None,
        Ok(xid) => Some(xid),
    }
}

/// Check the handle after `wayland:`: non-empty, bounded, and drawn from a safe alphabet.
fn parse_handle(handle: &str) -> Option<&str> {
    let valid = !handle.is_empty()
        && handle.len() <= MAX_HANDLE_BYTES
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    valid.then_some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_string_names_no_parent() {
        assert_eq!(ParentWindow::parse(""), Ok(ParentWindow::None));
        assert!(ParentWindow::parse("").is_ok_and(|parent| parent.is_none()));
    }

    #[test]
    fn an_x11_id_is_read_in_hexadecimal() {
        assert_eq!(ParentWindow::parse("x11:1a"), Ok(ParentWindow::X11(0x1a)));
        assert_eq!(ParentWindow::parse("x11:FFFFFFFF"), Ok(ParentWindow::X11(u32::MAX)));
        assert_eq!(ParentWindow::parse("x11:0040000a"), Ok(ParentWindow::X11(0x0040_000a)));
    }

    #[test]
    fn a_zero_or_malformed_x11_id_is_refused_with_the_rule_it_broke() {
        assert_eq!(ParentWindow::parse("x11:0"), Err(InvalidParentWindow::ZeroXid));
        assert_eq!(ParentWindow::parse("x11:000"), Err(InvalidParentWindow::ZeroXid));
        assert_eq!(
            ParentWindow::parse("x11:"),
            Err(InvalidParentWindow::InvalidXid("x11:".into()))
        );
        assert_eq!(
            ParentWindow::parse("x11:xyz"),
            Err(InvalidParentWindow::InvalidXid("x11:xyz".into()))
        );
        assert_eq!(
            ParentWindow::parse("x11:-1"),
            Err(InvalidParentWindow::InvalidXid("x11:-1".into()))
        );
        // Nine digits do not fit a 32-bit XID, however they are written.
        assert_eq!(
            ParentWindow::parse("x11:100000000"),
            Err(InvalidParentWindow::InvalidXid("x11:100000000".into()))
        );
    }

    #[test]
    fn a_wayland_handle_is_taken_opaquely_within_its_bounds() {
        assert_eq!(
            ParentWindow::parse("wayland:0123abcd"),
            Ok(ParentWindow::Wayland("0123abcd".into()))
        );
        let longest = "a".repeat(MAX_HANDLE_BYTES);
        assert_eq!(
            ParentWindow::parse(&format!("wayland:{longest}")),
            Ok(ParentWindow::Wayland(longest))
        );
    }

    #[test]
    fn a_wayland_handle_that_is_empty_overlong_or_unsafe_is_refused() {
        let raw_empty = "wayland:";
        assert_eq!(
            ParentWindow::parse(raw_empty),
            Err(InvalidParentWindow::InvalidHandle(raw_empty.into()))
        );
        let overlong = format!("wayland:{}", "a".repeat(MAX_HANDLE_BYTES + 1));
        assert_eq!(
            ParentWindow::parse(&overlong),
            Err(InvalidParentWindow::InvalidHandle(overlong.clone()))
        );
        for raw in ["wayland:a/b", "wayland:a b", "wayland:\u{e9}", "wayland:a\nb"] {
            assert_eq!(
                ParentWindow::parse(raw),
                Err(InvalidParentWindow::InvalidHandle(raw.into())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn garbage_and_unknown_schemes_are_refused_not_guessed() {
        for raw in ["garbage", "X11:1a", "wayland", "x11", "window:1", " x11:1a", "x11 :1a"] {
            assert!(ParentWindow::parse(raw).is_err(), "{raw:?} must be refused");
        }
        assert_eq!(
            ParentWindow::parse("garbage"),
            Err(InvalidParentWindow::UnknownScheme("garbage".into()))
        );
    }

    #[test]
    fn every_refusal_names_the_raw_string_in_its_message() {
        let error = ParentWindow::parse("x11:zz").unwrap_err();
        assert!(error.to_string().contains("x11:zz"), "{error}");
    }
}

//! Reply validation (D19 "Replies are validated" and "A reply cannot grant more
//! than the request asked"). A provider or a claim can never grant more than the
//! request asked for; a reply that does is a provider error.
//!
//! Validation lives here rather than in `gnoblin-schema` because the rules need
//! the typed request and grant, and `gnoblin-schema` holds only the wire model.

use std::fmt;

use crate::outcome::Grant;

/// The request side of a validation: what the caller asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Consent only: the grant must be `Allowed`.
    Consent,
    /// `multiple` allows several paths; `filter_count` bounds the filter index.
    FileChooser { multiple: bool, filter_count: usize },
    /// The desktop file ids that were offered.
    AppChooser { offered: Vec<String> },
    /// Copies up to `max_copies`, pages inside `first_page..=last_page`.
    Print {
        max_copies: u32,
        first_page: u32,
        last_page: u32,
    },
    /// `cursor_modes` is a bitmask of allowed modes (bit `n` is mode `n`);
    /// `multiple` allows more than one source.
    ScreenCast { cursor_modes: u32, multiple: bool },
    /// `device_types` is the requested bitmask.
    RemoteDesktop { device_types: u32 },
}

/// Why a reply exceeds the request or is malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deviation {
    /// The grant kind does not match the request.
    WrongKind,
    /// No path was returned.
    NoPaths,
    /// More than one path in a single-select dialog.
    TooManyPaths,
    /// A path is empty or not absolute.
    BadPath,
    /// The filter index is outside the request's filters.
    FilterOutOfRange,
    /// The app is not one the request offered.
    NotOffered,
    /// Copies are zero or above the request's maximum.
    CopiesOutOfRange,
    /// The page range is empty, reversed or outside the request's range.
    PagesOutOfRange,
    /// The cursor mode was not allowed by the request.
    CursorModeNotAllowed,
    /// Source count is zero, or above one without `multiple`.
    SourcesOutOfRange,
    /// No device type was granted.
    NoDevices,
    /// A device type was granted that was not requested.
    DevicesNotRequested,
}

impl fmt::Display for Deviation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::WrongKind => "grant kind does not match the request",
            Self::NoPaths => "no path granted",
            Self::TooManyPaths => "more than one path in a single-select dialog",
            Self::BadPath => "granted path is empty or not absolute",
            Self::FilterOutOfRange => "filter index outside the request's filters",
            Self::NotOffered => "app was not offered by the request",
            Self::CopiesOutOfRange => "copies outside the requested maximum",
            Self::PagesOutOfRange => "page range outside the requested range",
            Self::CursorModeNotAllowed => "cursor mode not allowed by the request",
            Self::SourcesOutOfRange => "source count outside what the request allows",
            Self::NoDevices => "no device type granted",
            Self::DevicesNotRequested => "device type granted but not requested",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for Deviation {}

/// Checks a provider or UI grant against the request it answers.
///
/// # Errors
///
/// The first [`Deviation`] found. A deviation from a provider is a provider
/// error and falls back to the built-in UI per the negotiation rules.
pub fn validate_reply(request: &Request, grant: &Grant) -> Result<(), Deviation> {
    match (request, grant) {
        (Request::Consent, Grant::Allowed) => Ok(()),
        (Request::FileChooser { multiple, filter_count }, Grant::Files { paths, filter }) => {
            if paths.is_empty() {
                return Err(Deviation::NoPaths);
            }
            if !multiple && paths.len() > 1 {
                return Err(Deviation::TooManyPaths);
            }
            if paths.iter().any(|path| path.is_empty() || !path.starts_with('/')) {
                return Err(Deviation::BadPath);
            }
            if *filter > 0 && *filter >= *filter_count {
                return Err(Deviation::FilterOutOfRange);
            }
            Ok(())
        }
        (Request::AppChooser { offered }, Grant::App { desktop_file_id, .. }) => {
            if offered.contains(desktop_file_id) {
                Ok(())
            } else {
                Err(Deviation::NotOffered)
            }
        }
        (
            Request::Print {
                max_copies,
                first_page,
                last_page,
            },
            Grant::Print {
                copies,
                first_page: granted_first,
                last_page: granted_last,
            },
        ) => {
            if *copies == 0 || copies > max_copies {
                return Err(Deviation::CopiesOutOfRange);
            }
            if granted_first > granted_last || granted_first < first_page || granted_last > last_page {
                return Err(Deviation::PagesOutOfRange);
            }
            Ok(())
        }
        (Request::ScreenCast { cursor_modes, multiple }, Grant::Capture { cursor_mode, sources }) => {
            let allowed = *cursor_mode < 32 && cursor_modes & (1 << cursor_mode) != 0;
            if !allowed {
                return Err(Deviation::CursorModeNotAllowed);
            }
            if *sources == 0 || (!multiple && *sources > 1) {
                return Err(Deviation::SourcesOutOfRange);
            }
            Ok(())
        }
        (Request::RemoteDesktop { device_types }, Grant::Devices { device_types: granted }) => {
            if *granted == 0 {
                return Err(Deviation::NoDevices);
            }
            if granted & !device_types != 0 {
                return Err(Deviation::DevicesNotRequested);
            }
            Ok(())
        }
        _ => Err(Deviation::WrongKind),
    }
}

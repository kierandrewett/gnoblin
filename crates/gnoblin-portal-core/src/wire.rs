//! The byte form portal values take across the provider ABI (`design/modular-framework.md`, 28.3).
//!
//! A UI provider is a C vtable, and a [`Dialog`], a [`Verdict`] and a [`ScreenKey`] are Rust types with strings and lists in
//! them. They cross as bytes, in one encoding both sides share, so neither side names the other's layout: every integer is
//! little-endian, a string is a `u32` length and its UTF-8 bytes, a list is a `u32` count and its items, and an option is a
//! `u8` flag and the value when it is set. An unknown tag, a length past the end or a trailing byte is an error, never a
//! guess, and a decoder allocates no more than the input holds.
//!
//! The dialog models keep their state private, so a dialog is sent as what built it: a consent dialog as the app, the prompt
//! and the tick it opened at; a file chooser as its mode, its folder, its filters, its listing, its filter index and its save
//! name; an app chooser as its content type, its choices, its last choice and its filter text. A dialog the compositor hands
//! a provider has no selection and no decision yet, so nothing else is needed to rebuild it.

use gnoblin_types::Tick;

use crate::model::{
    AppChoice, AppChooser, Consent, Dialog, FileChooser, FileChooserMode, FileEntry, FileFilter, Grant, Interface,
    ScreenKey,
};
use crate::outcome::{CancelReason, PortalError, Verdict};

/// Why a byte string is not a value of the type asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalWireError(pub &'static str);

impl std::fmt::Display for PortalWireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "a portal value did not decode: {}", self.0)
    }
}

impl std::error::Error for PortalWireError {}

/// Appends values to a byte string.
#[derive(Default)]
struct WireWriter(Vec<u8>);

impl WireWriter {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn flag(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn count(&mut self, count: usize) {
        self.u32(u32::try_from(count).unwrap_or(u32::MAX));
    }

    fn index(&mut self, index: usize) {
        self.u32(u32::try_from(index).unwrap_or(u32::MAX));
    }

    fn text(&mut self, text: &str) {
        self.count(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn option_text(&mut self, text: Option<&str>) {
        self.flag(text.is_some());
        if let Some(text) = text {
            self.text(text);
        }
    }
}

/// Reads values back from a byte string.
struct WireReader<'a>(&'a [u8]);

impl<'a> WireReader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], PortalWireError> {
        if self.0.len() < len {
            return Err(PortalWireError("the bytes end inside a value"));
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, PortalWireError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, PortalWireError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, PortalWireError> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    fn flag(&mut self) -> Result<bool, PortalWireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(PortalWireError("a flag is neither 0 nor 1")),
        }
    }

    /// The number of items a list holds. Every item takes at least one byte, so a count past what is left is a lie and
    /// is refused before anything is allocated for it.
    fn count(&mut self) -> Result<usize, PortalWireError> {
        let count = self.u32()? as usize;
        if count > self.0.len() {
            return Err(PortalWireError("a count is past what the bytes hold"));
        }
        Ok(count)
    }

    fn index(&mut self) -> Result<usize, PortalWireError> {
        Ok(self.u32()? as usize)
    }

    fn text(&mut self) -> Result<String, PortalWireError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| PortalWireError("a string is not UTF-8"))
    }

    fn option_text(&mut self) -> Result<Option<String>, PortalWireError> {
        if self.flag()? { self.text().map(Some) } else { Ok(None) }
    }

    fn finish(self) -> Result<(), PortalWireError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(PortalWireError("bytes follow the value"))
        }
    }
}

// ---- interfaces and keys ----------------------------------------------------------------------------------------------

/// The number a provider is told an interface by: its place in [`Interface::ALL`].
#[must_use]
pub fn interface_code(interface: Interface) -> u32 {
    Interface::ALL
        .iter()
        .position(|candidate| *candidate == interface)
        .map_or(0, |index| index as u32)
}

/// The interface a number names.
///
/// # Errors
/// [`PortalWireError`] for a number past the table.
pub fn interface_of(code: u32) -> Result<Interface, PortalWireError> {
    Interface::ALL
        .get(code as usize)
        .copied()
        .ok_or(PortalWireError("an interface number is past the table"))
}

/// A key as two numbers: a kind, and the character of a [`ScreenKey::Text`] (zero for every other kind).
#[must_use]
pub fn key_code(key: ScreenKey) -> (u32, u32) {
    match key {
        ScreenKey::Escape => (1, 0),
        ScreenKey::Enter => (2, 0),
        ScreenKey::Tab => (3, 0),
        ScreenKey::Up => (4, 0),
        ScreenKey::Down => (5, 0),
        ScreenKey::Space => (6, 0),
        ScreenKey::Backspace => (7, 0),
        ScreenKey::Text(character) => (8, u32::from(character)),
    }
}

/// The key two numbers name.
///
/// # Errors
/// [`PortalWireError`] for an unknown kind, or a character that is not one.
pub fn key_of(kind: u32, character: u32) -> Result<ScreenKey, PortalWireError> {
    Ok(match kind {
        1 => ScreenKey::Escape,
        2 => ScreenKey::Enter,
        3 => ScreenKey::Tab,
        4 => ScreenKey::Up,
        5 => ScreenKey::Down,
        6 => ScreenKey::Space,
        7 => ScreenKey::Backspace,
        8 => ScreenKey::Text(char::from_u32(character).ok_or(PortalWireError("a key's character is not a character"))?),
        _ => return Err(PortalWireError("a key kind is unknown")),
    })
}

// ---- dialogs ----------------------------------------------------------------------------------------------------------

const DIALOG_CONSENT: u8 = 1;
const DIALOG_FILES: u8 = 2;
const DIALOG_APP: u8 = 3;

/// The dialog as bytes.
#[must_use]
pub fn encode_dialog(dialog: &Dialog) -> Vec<u8> {
    let mut out = WireWriter::default();
    match dialog {
        Dialog::Consent(consent) => {
            out.u8(DIALOG_CONSENT);
            out.text(consent.app_id());
            out.text(consent.prompt());
            out.u64(consent.opened_at().get());
        }
        Dialog::Files(chooser) => {
            out.u8(DIALOG_FILES);
            out.u8(match chooser.mode() {
                FileChooserMode::Open => 0,
                FileChooserMode::Save => 1,
                FileChooserMode::Folder => 2,
            });
            out.flag(chooser.multiple());
            out.text(chooser.folder());
            out.count(chooser.filters().len());
            for filter in chooser.filters() {
                out.text(&filter.name);
                out.count(filter.patterns.len());
                for pattern in &filter.patterns {
                    out.text(pattern);
                }
            }
            out.count(chooser.entries().len());
            for entry in chooser.entries() {
                out.text(&entry.name);
                out.flag(entry.is_dir);
                out.option_text(entry.mime.as_deref());
            }
            out.index(chooser.filter_index());
            out.text(chooser.name());
        }
        Dialog::App(chooser) => {
            out.u8(DIALOG_APP);
            out.text(chooser.content_type());
            out.count(chooser.choices().len());
            for choice in chooser.choices() {
                out.text(&choice.desktop_file_id);
                out.text(&choice.name);
            }
            out.option_text(chooser.last_choice());
            out.text(chooser.filter_text());
        }
    }
    out.0
}

/// The dialog bytes name.
///
/// # Errors
/// [`PortalWireError`] when the bytes are not an encoded dialog.
pub fn decode_dialog(bytes: &[u8]) -> Result<Dialog, PortalWireError> {
    let mut input = WireReader(bytes);
    let dialog = match input.u8()? {
        DIALOG_CONSENT => {
            let app = input.text()?;
            let prompt = input.text()?;
            let opened = input.u64()?;
            Dialog::Consent(Consent::open(app, prompt, Tick::new(opened)))
        }
        DIALOG_FILES => {
            let mode = match input.u8()? {
                0 => FileChooserMode::Open,
                1 => FileChooserMode::Save,
                2 => FileChooserMode::Folder,
                _ => return Err(PortalWireError("a file chooser mode is unknown")),
            };
            let multiple = input.flag()?;
            let folder = input.text()?;
            let mut filters = Vec::new();
            for _ in 0..input.count()? {
                let name = input.text()?;
                let mut patterns = Vec::new();
                for _ in 0..input.count()? {
                    patterns.push(input.text()?);
                }
                filters.push(FileFilter { name, patterns });
            }
            let mut entries = Vec::new();
            for _ in 0..input.count()? {
                let name = input.text()?;
                let is_dir = input.flag()?;
                let mime = input.option_text()?;
                entries.push(FileEntry { name, is_dir, mime });
            }
            let filter = input.index()?;
            let name = input.text()?;
            let mut chooser = FileChooser::new(mode, multiple, folder, filters, name);
            chooser.set_entries(entries);
            chooser.set_filter(filter);
            Dialog::Files(chooser)
        }
        DIALOG_APP => {
            let content_type = input.text()?;
            let mut choices = Vec::new();
            for _ in 0..input.count()? {
                let desktop_file_id = input.text()?;
                let name = input.text()?;
                choices.push(AppChoice { desktop_file_id, name });
            }
            let last = input.option_text()?;
            let filter = input.text()?;
            Dialog::App(AppChooser::new(content_type, choices, last, filter))
        }
        _ => return Err(PortalWireError("a dialog kind is unknown")),
    };
    input.finish()?;
    Ok(dialog)
}

// ---- verdicts ---------------------------------------------------------------------------------------------------------

const GRANT_ALLOWED: u8 = 1;
const GRANT_FILES: u8 = 2;
const GRANT_APP: u8 = 3;
const GRANT_PRINT: u8 = 4;
const GRANT_CAPTURE: u8 = 5;
const GRANT_DEVICES: u8 = 6;

const VERDICT_HANDLED: u8 = 1;
const VERDICT_CANCELLED: u8 = 2;
const VERDICT_DECLINED: u8 = 3;
const VERDICT_ERROR: u8 = 4;

fn write_grant(out: &mut WireWriter, grant: &Grant) {
    match grant {
        Grant::Allowed => out.u8(GRANT_ALLOWED),
        Grant::Files { paths, filter } => {
            out.u8(GRANT_FILES);
            out.count(paths.len());
            for path in paths {
                out.text(path);
            }
            out.index(*filter);
        }
        Grant::App {
            desktop_file_id,
            remember,
        } => {
            out.u8(GRANT_APP);
            out.text(desktop_file_id);
            out.flag(*remember);
        }
        Grant::Print {
            copies,
            first_page,
            last_page,
        } => {
            out.u8(GRANT_PRINT);
            out.u32(*copies);
            out.u32(*first_page);
            out.u32(*last_page);
        }
        Grant::Capture { cursor_mode, sources } => {
            out.u8(GRANT_CAPTURE);
            out.u32(*cursor_mode);
            out.u32(*sources);
        }
        Grant::Devices { device_types } => {
            out.u8(GRANT_DEVICES);
            out.u32(*device_types);
        }
    }
}

fn read_grant(input: &mut WireReader<'_>) -> Result<Grant, PortalWireError> {
    Ok(match input.u8()? {
        GRANT_ALLOWED => Grant::Allowed,
        GRANT_FILES => {
            let mut paths = Vec::new();
            for _ in 0..input.count()? {
                paths.push(input.text()?);
            }
            Grant::Files {
                paths,
                filter: input.index()?,
            }
        }
        GRANT_APP => Grant::App {
            desktop_file_id: input.text()?,
            remember: input.flag()?,
        },
        GRANT_PRINT => Grant::Print {
            copies: input.u32()?,
            first_page: input.u32()?,
            last_page: input.u32()?,
        },
        GRANT_CAPTURE => Grant::Capture {
            cursor_mode: input.u32()?,
            sources: input.u32()?,
        },
        GRANT_DEVICES => Grant::Devices {
            device_types: input.u32()?,
        },
        _ => return Err(PortalWireError("a grant kind is unknown")),
    })
}

const CANCEL_REASONS: [CancelReason; 4] = [
    CancelReason::User,
    CancelReason::Timeout,
    CancelReason::UiFailed,
    CancelReason::RequesterGone,
];

const ERRORS: [PortalError; 7] = [
    PortalError::NoUi,
    PortalError::Locked,
    PortalError::Busy,
    PortalError::ProviderFailed,
    PortalError::NotSupported,
    PortalError::UnknownApp,
    PortalError::UnsupportedUi,
];

/// The number a provider reports an error by: its place in the table of errors.
#[must_use]
pub fn error_code(error: PortalError) -> u32 {
    ERRORS
        .iter()
        .position(|candidate| *candidate == error)
        .map_or(0, |index| index as u32)
}

/// The error a number names.
///
/// # Errors
/// [`PortalWireError`] for a number past the table.
pub fn error_of(code: u32) -> Result<PortalError, PortalWireError> {
    ERRORS
        .get(code as usize)
        .copied()
        .ok_or(PortalWireError("an error number is past the table"))
}

/// The verdict of one request as bytes.
#[must_use]
pub fn encode_verdict(verdict: &Verdict) -> Vec<u8> {
    let mut out = WireWriter::default();
    match verdict {
        Verdict::Handled(grant) => {
            out.u8(VERDICT_HANDLED);
            write_grant(&mut out, grant);
        }
        Verdict::Cancelled(reason) => {
            out.u8(VERDICT_CANCELLED);
            out.u8(CANCEL_REASONS
                .iter()
                .position(|candidate| candidate == reason)
                .map_or(0, |index| index as u8));
        }
        Verdict::Declined => out.u8(VERDICT_DECLINED),
        Verdict::Error(error) => {
            out.u8(VERDICT_ERROR);
            out.u32(error_code(*error));
        }
    }
    out.0
}

/// The verdict bytes name.
///
/// # Errors
/// [`PortalWireError`] when the bytes are not an encoded verdict.
pub fn decode_verdict(bytes: &[u8]) -> Result<Verdict, PortalWireError> {
    let mut input = WireReader(bytes);
    let verdict = match input.u8()? {
        VERDICT_HANDLED => Verdict::Handled(read_grant(&mut input)?),
        VERDICT_CANCELLED => Verdict::Cancelled(
            CANCEL_REASONS
                .get(usize::from(input.u8()?))
                .copied()
                .ok_or(PortalWireError("a cancel reason is unknown"))?,
        ),
        VERDICT_DECLINED => Verdict::Declined,
        VERDICT_ERROR => Verdict::Error(error_of(input.u32()?)?),
        _ => return Err(PortalWireError("a verdict kind is unknown")),
    };
    input.finish()?;
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(dialog: &Dialog) -> Dialog {
        decode_dialog(&encode_dialog(dialog)).expect("a dialog decodes")
    }

    #[test]
    fn a_consent_dialog_keeps_its_app_prompt_and_opening_tick() {
        let dialog = Dialog::Consent(Consent::open("org.example.App", "Share your screen?", Tick::new(1234)));
        assert_eq!(round(&dialog), dialog);
    }

    #[test]
    fn a_file_chooser_keeps_its_filters_listing_filter_and_name() {
        let mut chooser = FileChooser::new(
            FileChooserMode::Save,
            true,
            "/home/user",
            vec![
                FileFilter {
                    name: "Images".to_owned(),
                    patterns: vec!["*.png".to_owned(), "image/jpeg".to_owned()],
                },
                FileFilter {
                    name: "All".to_owned(),
                    patterns: vec!["*".to_owned()],
                },
            ],
            "draft.txt",
        );
        chooser.set_entries(vec![
            FileEntry {
                name: "Pictures".to_owned(),
                is_dir: true,
                mime: None,
            },
            FileEntry {
                name: "a.png".to_owned(),
                is_dir: false,
                mime: Some("image/png".to_owned()),
            },
        ]);
        chooser.set_filter(1);
        let dialog = Dialog::Files(chooser);
        assert_eq!(round(&dialog), dialog);
    }

    #[test]
    fn an_app_chooser_keeps_its_choices_last_choice_and_filter_text() {
        let dialog = Dialog::App(AppChooser::new(
            "text/plain",
            vec![AppChoice {
                desktop_file_id: "org.example.Edit.desktop".to_owned(),
                name: "Edit".to_owned(),
            }],
            Some("org.example.Edit.desktop".to_owned()),
            "ed",
        ));
        assert_eq!(round(&dialog), dialog);
    }

    #[test]
    fn every_verdict_round_trips() {
        for verdict in [
            Verdict::Handled(Grant::Allowed),
            Verdict::Handled(Grant::Files {
                paths: vec!["/a".to_owned(), "/b".to_owned()],
                filter: 1,
            }),
            Verdict::Handled(Grant::App {
                desktop_file_id: "x.desktop".to_owned(),
                remember: true,
            }),
            Verdict::Handled(Grant::Print {
                copies: 2,
                first_page: 1,
                last_page: 9,
            }),
            Verdict::Handled(Grant::Capture {
                cursor_mode: 2,
                sources: 1,
            }),
            Verdict::Handled(Grant::Devices { device_types: 3 }),
            Verdict::Cancelled(CancelReason::RequesterGone),
            Verdict::Declined,
            Verdict::Error(PortalError::UnsupportedUi),
        ] {
            assert_eq!(decode_verdict(&encode_verdict(&verdict)), Ok(verdict));
        }
    }

    #[test]
    fn every_interface_and_key_round_trips() {
        for interface in Interface::ALL {
            assert_eq!(interface_of(interface_code(interface)), Ok(interface));
        }
        for key in [
            ScreenKey::Escape,
            ScreenKey::Enter,
            ScreenKey::Tab,
            ScreenKey::Up,
            ScreenKey::Down,
            ScreenKey::Space,
            ScreenKey::Backspace,
            ScreenKey::Text('\u{e9}'),
        ] {
            let (kind, character) = key_code(key);
            assert_eq!(key_of(kind, character), Ok(key));
        }
    }

    #[test]
    fn bytes_that_are_not_a_value_are_refused_not_guessed() {
        let good = encode_dialog(&Dialog::Consent(Consent::open("a", "b", Tick::ZERO)));
        assert!(decode_dialog(&good[..good.len() - 1]).is_err(), "truncated");
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_dialog(&trailing).is_err(), "trailing byte");
        assert!(decode_dialog(&[9]).is_err(), "unknown kind");
        assert!(decode_dialog(&[]).is_err(), "empty");
        assert!(decode_verdict(&[1, 9]).is_err(), "unknown grant");
        assert!(interface_of(9999).is_err());
        assert!(key_of(8, 0xD800).is_err(), "a surrogate is not a character");
        // A count that claims more items than there are bytes must not allocate for them.
        let hostile = [DIALOG_FILES, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(decode_dialog(&hostile).is_err());
    }
}

//! Where a screenshot lands on disk, and why a capture the compositor refused is a refusal.
//!
//! The compositor sends one PNG and never names a path. This module picks the folder (the
//! user's Pictures folder, as `user-dirs.dirs` names it), writes the bytes into a new file
//! and returns that file's `file://` URI. A write that fails is a failure: no URI is returned
//! for a file that does not exist, and a zero-byte picture is not a screenshot.
//!
//! The file is named `Screenshot from <stamp>.png`, the name GNOME gives its own screenshots,
//! and is created with mode 0600: a picture of the desktop is private to its user until the
//! user moves it. The stamp is UTC, because this crate has no timezone source.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gnoblin_client::code;

use crate::backends::Unavailable;

/// How many names are tried for one instant before a write gives up.
const NAME_ATTEMPTS: u32 = 100;

/// The folder a screenshot is written to.
///
/// `XDG_PICTURES_DIR` wins, then the user's `user-dirs.dirs`, then `$HOME/Pictures`, the
/// default the XDG base directory specification gives. `None` when there is no home at all.
pub fn pictures_dir(environment: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let home = environment("HOME").filter(|home| !home.is_empty()).map(PathBuf::from);
    if let Some(dir) = environment("XDG_PICTURES_DIR").filter(|dir| Path::new(dir).is_absolute()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(home) = &home {
        let config = environment("XDG_CONFIG_HOME")
            .filter(|config| !config.is_empty())
            .map_or_else(|| home.join(".config"), PathBuf::from);
        if let Ok(text) = fs::read_to_string(config.join("user-dirs.dirs"))
            && let Some(dir) = parse_user_dirs(&text, home)
        {
            return Some(dir);
        }
    }
    home.map(|home| home.join("Pictures"))
}

/// The `XDG_PICTURES_DIR` line of a `user-dirs.dirs` file, with `$HOME` expanded. A value
/// that is neither absolute nor under `$HOME` is not a folder this module can name.
fn parse_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("XDG_PICTURES_DIR=")?.trim().trim_matches('"');
        if value == "$HOME" {
            Some(home.to_owned())
        } else if let Some(relative) = value.strip_prefix("$HOME/") {
            Some(home.join(relative))
        } else if value.starts_with('/') {
            Some(PathBuf::from(value))
        } else {
            None
        }
    })
}

/// Write one PNG into `dir` as a new file named for the current time, and return its path.
///
/// The file is created new, so an existing screenshot is never overwritten: a second one
/// taken in the same second gets ` (2)`, ` (3)` and so on. A write that fails part way
/// removes what it wrote, so no truncated picture is left behind.
pub fn write_png(png: &[u8], dir: &Path) -> Result<PathBuf, Unavailable> {
    if png.is_empty() {
        return Err(Unavailable::refused(
            "the compositor sent an empty picture, so no screenshot was written",
        ));
    }
    fs::create_dir_all(dir).map_err(|error| {
        Unavailable::refused(format!(
            "the screenshot folder {} cannot be created: {error}",
            dir.display()
        ))
    })?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX));
    let stamp = utc_stamp(seconds);
    let mut file = None;
    let mut path = dir.join(format!("Screenshot from {stamp}.png"));
    for attempt in 1..=NAME_ATTEMPTS {
        if attempt > 1 {
            path = dir.join(format!("Screenshot from {stamp} ({attempt}).png"));
        }
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(opened) => {
                file = Some(opened);
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(Unavailable::refused(format!(
                    "the screenshot {} cannot be created: {error}",
                    path.display()
                )));
            }
        }
    }
    let Some(mut file) = file else {
        return Err(Unavailable::refused(format!(
            "no free name for a screenshot in {} after {NAME_ATTEMPTS} tries",
            dir.display()
        )));
    };
    if let Err(error) = file.write_all(png) {
        let _ = fs::remove_file(&path);
        return Err(Unavailable::refused(format!(
            "the screenshot {} cannot be written: {error}",
            path.display()
        )));
    }
    Ok(path)
}

/// The `YYYY-MM-DD HH-MM-SS` UTC stamp of a Unix time, in the form GNOME's file names use.
#[must_use]
pub fn utc_stamp(seconds: i64) -> String {
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let within = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}-{:02}-{:02}",
        within / 3_600,
        within % 3_600 / 60,
        within % 60
    )
}

/// The proleptic Gregorian date of a day count from 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, u32::try_from(month).unwrap_or(1), u32::try_from(day).unwrap_or(1))
}

/// The `file://` URI of an absolute path, with every byte outside the unreserved set escaped.
#[must_use]
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                uri.push(char::from(byte));
            }
            _ => {
                let _ = write!(uri, "%{byte:02X}");
            }
        }
    }
    uri
}

/// The sentence a user reads when the compositor refuses a capture. Each schema code has its
/// own sentence; a code this build does not know is passed on with the compositor's text.
#[must_use]
pub fn refusal(error_code: &str, message: &str) -> Unavailable {
    let reason = match error_code {
        code::CAPTURE_LOCKED => "the session is locked, so the screen cannot be captured".to_owned(),
        code::CAPTURE_NO_OUTPUT => "the compositor has no output to capture".to_owned(),
        code::CAPTURE_UNAVAILABLE => "the output has no frame to capture yet".to_owned(),
        code::CAPTURE_TOO_LARGE => "the picture is larger than one frame can carry".to_owned(),
        code::CAPTURE_ENCODE_FAILED => "the captured frame could not be encoded as PNG".to_owned(),
        _ => format!("the compositor refused the capture ({error_code}): {message}"),
    };
    Unavailable::refused(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A folder under the system temp directory, empty and unique to one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblin-portal-picture-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_picture_is_written_as_a_new_file_and_its_bytes_are_kept() {
        let dir = scratch("write");
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let path = write_png(&png, &dir).expect("the picture is written");
        assert_eq!(fs::read(&path).expect("it reads back"), png);
        assert!(path.starts_with(&dir));
        assert_eq!(path.extension().and_then(|extension| extension.to_str()), Some("png"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_is_named_like_gnome_and_is_private_to_its_user() {
        // Failure mode: a picture of the desktop readable by other users, or a name that
        // does not say what it is.
        let dir = scratch("name");
        let path = write_png(&[1, 2, 3], &dir).expect("the picture is written");
        let name = path.file_name().and_then(|name| name.to_str()).expect("a UTF-8 name");
        assert!(name.starts_with("Screenshot from "), "{name}");
        assert!(name.ends_with(".png"), "{name}");
        let mode = fs::metadata(&path).expect("it exists").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_pictures_in_one_second_are_two_files() {
        // Failure mode: the second screenshot overwrites the first.
        let dir = scratch("collide");
        let first = write_png(&[1], &dir).expect("the first is written");
        let second = write_png(&[2], &dir).expect("the second is written");
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).expect("the first reads back"), [1]);
        assert_eq!(fs::read(&second).expect("the second reads back"), [2]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_picture_is_a_refusal_and_creates_nothing() {
        // Failure mode: a zero-byte answer must not become a file the app is told exists.
        let dir = scratch("empty");
        assert!(write_png(&[], &dir).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn a_folder_that_cannot_be_created_is_a_refusal_not_a_uri() {
        // Failure mode: a regular file where the folder should be cannot become a folder.
        let blocker = scratch("blocked");
        fs::write(&blocker, b"a file").expect("the blocker is written");
        assert!(write_png(&[1, 2, 3], &blocker.join("Screenshots")).is_err());
        let _ = fs::remove_file(&blocker);
    }

    #[test]
    fn the_stamp_is_utc_and_names_the_calendar_day() {
        assert_eq!(utc_stamp(0), "1970-01-01 00-00-00");
        assert_eq!(utc_stamp(1_700_000_000), "2023-11-14 22-13-20");
        // 2024 is a leap year: the day after February 28 is February 29.
        assert_eq!(utc_stamp(1_709_164_800), "2024-02-29 00-00-00");
    }

    #[test]
    fn the_uri_escapes_spaces_and_keeps_the_path() {
        assert_eq!(
            file_uri(Path::new("/home/a b/Pictures/x.png")),
            "file:///home/a%20b/Pictures/x.png"
        );
    }

    #[test]
    fn user_dirs_names_the_pictures_folder_with_home_expanded() {
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Shots\"\n";
        assert_eq!(
            parse_user_dirs(text, Path::new("/home/kieran")),
            Some(PathBuf::from("/home/kieran/Shots"))
        );
        assert_eq!(
            parse_user_dirs("XDG_PICTURES_DIR=\"/data/pics\"\n", Path::new("/h")),
            Some(PathBuf::from("/data/pics"))
        );
        // Failure mode: a relative value names no folder, so the default is used instead.
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=\"pics\"\n", Path::new("/h")), None);
        assert_eq!(parse_user_dirs("XDG_DESKTOP_DIR=\"$HOME/D\"\n", Path::new("/h")), None);
    }

    #[test]
    fn the_environment_overrides_the_user_dirs_and_home_is_the_last_resort() {
        let configured = |name: &str| match name {
            "XDG_PICTURES_DIR" => Some("/srv/pics".to_owned()),
            "HOME" => Some("/nonexistent-gnoblin-test-home".to_owned()),
            _ => None,
        };
        assert_eq!(pictures_dir(configured), Some(PathBuf::from("/srv/pics")));
        let home_only = |name: &str| (name == "HOME").then(|| "/nonexistent-gnoblin-test-home".to_owned());
        assert_eq!(
            pictures_dir(home_only),
            Some(PathBuf::from("/nonexistent-gnoblin-test-home/Pictures"))
        );
        assert_eq!(pictures_dir(|_| None), None);
        // Failure mode: a relative XDG_PICTURES_DIR is not a folder, so the default is used.
        let relative = |name: &str| match name {
            "XDG_PICTURES_DIR" => Some("pics".to_owned()),
            "HOME" => Some("/h".to_owned()),
            _ => None,
        };
        assert_eq!(pictures_dir(relative), Some(PathBuf::from("/h/Pictures")));
    }

    #[test]
    fn every_refusal_names_its_reason() {
        assert!(refusal(code::CAPTURE_LOCKED, "").reason.contains("locked"));
        assert!(refusal(code::CAPTURE_NO_OUTPUT, "").reason.contains("output"));
        assert!(refusal(code::CAPTURE_UNAVAILABLE, "").reason.contains("frame"));
        assert!(refusal(code::CAPTURE_TOO_LARGE, "").reason.contains("larger"));
        assert!(refusal(code::CAPTURE_ENCODE_FAILED, "").reason.contains("PNG"));
        assert!(refusal("from-the-future", "detail").reason.contains("from-the-future"));
    }
}

//! The one version type the IPC and the schema share.
//!
//! Design/rust-compositor.md ("Versioning") makes the IPC version exactly the
//! Gnoblin version and gives every schema member the version that added it, so
//! both sides need one small ordered type instead of a general range type.
//! Pre-release ordering is deliberately absent: a candidate build
//! (`0.2.0-alpha.N`) advertises the triple of the release it becomes, so a
//! candidate and its release interoperate and only three numbers travel.

use std::fmt;
use std::str::FromStr;

/// A full `MAJOR.MINOR.PATCH` version with no pre-release or build metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Build a version from its three parts.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version { major, minor, patch }
    }

    /// Parse exactly three dot-separated decimal numbers.
    ///
    /// Strictness is the point: "0.2" and "0.2.0-alpha.1" must fail here rather
    /// than reach a wire format that only carries three numbers.
    pub fn parse(text: &str) -> Result<Version, InvalidVersion> {
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 3 {
            return Err(InvalidVersion {
                text: text.to_owned(),
                reason: "expected exactly three dot-separated numbers",
            });
        }
        let mut numbers = [0u32; 3];
        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(InvalidVersion {
                    text: text.to_owned(),
                    reason: "each part must be a decimal number",
                });
            }
            match part.parse::<u32>() {
                Ok(number) => numbers[index] = number,
                Err(_) => {
                    return Err(InvalidVersion {
                        text: text.to_owned(),
                        reason: "a part does not fit in 32 bits",
                    });
                }
            }
        }
        Ok(Version::new(numbers[0], numbers[1], numbers[2]))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = InvalidVersion;

    fn from_str(text: &str) -> Result<Version, InvalidVersion> {
        Version::parse(text)
    }
}

/// A string that is not a full `MAJOR.MINOR.PATCH` version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidVersion {
    text: String,
    reason: &'static str,
}

impl InvalidVersion {
    /// The text that was rejected, so a caller can quote it in its own message.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for InvalidVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` is not a MAJOR.MINOR.PATCH version: {}",
            self.text, self.reason
        )
    }
}

impl std::error::Error for InvalidVersion {}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn parses_and_orders_full_versions() {
        let version = Version::parse("0.2.10").expect("version");
        assert_eq!(version, Version::new(0, 2, 10));
        assert_eq!(version.to_string(), "0.2.10");
        assert!(Version::new(0, 2, 9) < version);
        assert!(Version::new(0, 3, 0) > version);
        assert!(Version::new(1, 0, 0) > Version::new(0, 99, 99));
    }

    #[test]
    fn rejects_anything_that_is_not_three_numbers() {
        for text in [
            "",
            "0.2",
            "0.2.0.1",
            "0.2.x",
            "0..2",
            "0.2.0-alpha.1",
            "1.0.0 ",
            "-1.0.0",
        ] {
            assert!(Version::parse(text).is_err(), "`{text}` should not parse");
        }
    }

    #[test]
    fn message_names_the_rejected_text() {
        let error = Version::parse("0.2").expect_err("invalid");
        assert!(error.to_string().contains("`0.2`"), "{error}");
        assert_eq!(error.text(), "0.2");
    }
}

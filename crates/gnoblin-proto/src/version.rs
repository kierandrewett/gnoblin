//! The version rule: which compositor and client versions may talk.
//!
//! Design/rust-compositor.md ("Versioning") uses Cargo's caret rule. A client
//! built against version A is accepted by a compositor at version B when B is at
//! least A and the leftmost non-zero part of A matches B. That gives the two
//! properties the plan needs:
//!
//! - patch and minor releases stay compatible while the leftmost non-zero part is
//!   unchanged, which is what an additive `since` member is for;
//! - a release that removes or reshapes a released member bumps the leftmost
//!   non-zero part, so a client built before it is refused instead of failing
//!   later on a member that moved.
//!
//! For `0.x` versions the leftmost non-zero part is the minor, and for `0.0.x` it
//! is the patch, exactly as Cargo resolves `^0.2` and `^0.0.3`.

use std::fmt;

use gnoblin_schema::Version;

/// Which part of the client's version the caret rule anchors on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Major,
    Minor,
    Patch,
}

fn anchor(version: Version) -> Anchor {
    if version.major != 0 {
        Anchor::Major
    } else if version.minor != 0 {
        Anchor::Minor
    } else {
        Anchor::Patch
    }
}

/// Whether `server` accepts a client built against `client`.
pub fn accepts(server: Version, client: Version) -> bool {
    let compatible = match anchor(client) {
        Anchor::Major => server.major == client.major,
        Anchor::Minor => server.major == client.major && server.minor == client.minor,
        // `^0.0.3` resolves to exactly `0.0.3`.
        Anchor::Patch => server.major == 0 && server.minor == 0 && server.patch == client.patch,
    };
    compatible && server >= client
}

/// Check a pairing, returning the refusal that names both versions.
pub fn check(server: Version, client: Version) -> Result<(), Mismatch> {
    if accepts(server, client) {
        Ok(())
    } else {
        Err(Mismatch { server, client })
    }
}

/// A refused pairing: the compositor's version cannot serve the client's build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// The compositor's IPC version.
    pub server: Version,
    /// The version the client was built against.
    pub client: Version,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compositor IPC {} refuses client IPC {}: the leftmost non-zero part must match and the compositor must be at least as new",
            self.server, self.client
        )
    }
}

impl std::error::Error for Mismatch {}

#[cfg(test)]
mod tests {
    use super::{Mismatch, accepts, check};
    use gnoblin_schema::Version;

    fn version(text: &str) -> Version {
        Version::parse(text).expect("version")
    }

    #[test]
    fn accepts_the_same_version() {
        assert!(accepts(version("0.2.0"), version("0.2.0")));
    }

    #[test]
    fn accepts_a_newer_patch_on_the_same_leftmost_non_zero_part() {
        assert!(accepts(version("0.2.7"), version("0.2.4")));
        assert!(accepts(version("1.9.3"), version("1.4.2")));
    }

    #[test]
    fn refuses_an_older_compositor() {
        assert!(!accepts(version("0.2.4"), version("0.2.7")));
        assert!(!accepts(version("1.4.2"), version("1.9.3")));
    }

    #[test]
    fn refuses_a_changed_leftmost_non_zero_part() {
        // 0.x: the minor is the breaking part.
        assert!(!accepts(version("0.3.0"), version("0.2.4")));
        // 1.x: the major is.
        assert!(!accepts(version("2.0.0"), version("1.4.2")));
        // 0.0.x: the patch is, so nothing else is compatible.
        assert!(accepts(version("0.0.3"), version("0.0.3")));
        assert!(!accepts(version("0.0.4"), version("0.0.3")));
        assert!(!accepts(version("0.1.0"), version("0.0.3")));
    }

    #[test]
    fn the_refusal_names_both_versions() {
        let mismatch = check(version("0.3.0"), version("0.2.4")).expect_err("refused");
        let message = mismatch.to_string();
        assert!(message.contains("compositor IPC 0.3.0"), "{message}");
        assert!(message.contains("client IPC 0.2.4"), "{message}");
        assert_eq!(
            mismatch,
            Mismatch {
                server: version("0.3.0"),
                client: version("0.2.4"),
            }
        );
    }
}

//! Best-effort redaction of secrets in raw child output.
//!
//! This is a set of heuristics over one line of text. It recognises `password=hunter2`,
//! `"token": "..."`, `Authorization: Bearer ...`, PEM private key blocks and long opaque tokens.
//! It cannot recognise every secret: a password written as prose, a secret split across lines,
//! or a token that matches none of the rules passes through unchanged. Redaction is therefore
//! never a guarantee that a line is free of secrets. The real protection is not to capture the
//! output of a process that may print secrets at all.

/// What a redacted secret is replaced with.
pub const REDACTED: &str = "[redacted]";

/// The words that mark the value after them as a secret, matched as substrings of the key in
/// lower case, so `db_password` and `api-token` both count.
const SECRET_NAMES: [&str; 9] = [
    "password",
    "passwd",
    "secret",
    "token",
    "passphrase",
    "answer",
    "key",
    "credential",
    "authorization",
];

/// The shortest run of token characters redacted as an opaque token.
const TOKEN_LENGTH: usize = 40;

/// The quotes a key or value may be wrapped in.
const QUOTES: &[char] = &['"', '\''];

/// Redacts the lines of one stream. A PEM private key block spans lines, so the redactor
/// remembers whether one is open: keep one redactor per stream and feed it the lines in order.
#[derive(Debug, Default)]
pub struct Redactor {
    in_private_key: bool,
}

impl Redactor {
    /// A redactor with no block open.
    pub fn new() -> Self {
        Self::default()
    }

    /// The line with what looks like a secret replaced by [`REDACTED`]. Best effort; see the
    /// module documentation.
    pub fn redact(&mut self, line: &str) -> String {
        if let Some(redacted) = self.private_key_line(line) {
            return redacted;
        }
        scrub(line)
    }

    /// The redacted form of a line that belongs to a PEM private key block, or `None` when the
    /// line is not part of one.
    fn private_key_line(&mut self, line: &str) -> Option<String> {
        let ends = line.contains("-----END");
        if self.in_private_key {
            self.in_private_key = !ends;
            return Some(REDACTED.to_owned());
        }
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            self.in_private_key = !ends;
            return Some(REDACTED.to_owned());
        }
        None
    }
}

/// What the scanner expects next, after the words it has already read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// An ordinary word.
    Any,
    /// A secret name was just read; a separator or the value may follow.
    Name,
    /// The next word is a secret value.
    Value,
}

/// Redacts a single line word by word, keeping the whitespace between the words as it was.
fn scrub(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut expect = Expect::Any;
    let mut rest = line;
    while !rest.is_empty() {
        let gap = rest.len() - rest.trim_start().len();
        out.push_str(&rest[..gap]);
        rest = &rest[gap..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(end);
        rest = tail;
        expect = scrub_word(word, expect, &mut out);
    }
    out
}

/// Appends one word to `out`, redacted if it is a secret, and returns what the next word must be.
fn scrub_word(word: &str, expect: Expect, out: &mut String) -> Expect {
    match expect {
        Expect::Value => {
            if is_auth_scheme(word) {
                out.push_str(word);
                return Expect::Value;
            }
            out.push_str(REDACTED);
            return Expect::Any;
        }
        Expect::Name if word.starts_with(['=', ':']) => {
            let split = word.find(|c: char| c != '=' && c != ':').unwrap_or(word.len());
            out.push_str(&word[..split]);
            if split == word.len() {
                return Expect::Value;
            }
            out.push_str(REDACTED);
            return Expect::Any;
        }
        Expect::Any | Expect::Name => {}
    }
    if let Some(index) = word.find(['=', ':']) {
        if is_secret_name(word[..index].trim_matches(QUOTES)) {
            out.push_str(&word[..=index]);
            if index + 1 == word.len() {
                return Expect::Value;
            }
            out.push_str(REDACTED);
            return Expect::Any;
        }
    } else if is_secret_name(bare(word)) {
        out.push_str(word);
        return Expect::Name;
    } else if bare(word).eq_ignore_ascii_case("bearer") {
        out.push_str(word);
        return Expect::Value;
    }
    push_unless_opaque(word, out);
    Expect::Any
}

/// Appends `word`, with its opaque-token core redacted when it is one.
fn push_unless_opaque(word: &str, out: &mut String) {
    let lead = word.len() - word.trim_start_matches(|c: char| !is_token_char(c)).len();
    let core = word[lead..].trim_end_matches(|c: char| !is_token_char(c));
    if is_opaque_token(core) {
        out.push_str(&word[..lead]);
        out.push_str(REDACTED);
        out.push_str(&word[lead + core.len()..]);
    } else {
        out.push_str(word);
    }
}

/// The word without the punctuation around it.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Whether a key contains one of the [`SECRET_NAMES`].
fn is_secret_name(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|name| key.contains(name))
}

/// Whether a word is an authorisation scheme that precedes the credential itself.
fn is_auth_scheme(word: &str) -> bool {
    let word = bare(word);
    word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic")
}

/// Whether a character can be part of an opaque token, including base64 padding and separators.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')
}

/// Whether a run of token characters is long enough, and mixes letters and digits, to be a key
/// or a credential rather than a word.
fn is_opaque_token(core: &str) -> bool {
    core.len() >= TOKEN_LENGTH
        && core.chars().all(is_token_char)
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(line: &str) -> String {
        Redactor::new().redact(line)
    }

    #[test]
    fn key_value_pairs_keep_the_name_and_lose_the_value() {
        assert_eq!(redact("password=hunter2 user=alice"), "password=[redacted] user=alice");
        assert_eq!(redact("db_password=hunter2"), "db_password=[redacted]");
    }

    #[test]
    fn json_and_spaced_pairs_are_redacted() {
        assert_eq!(redact(r#""token": "abc""#), r#""token": [redacted]"#);
        assert_eq!(redact("Password: hunter2"), "Password: [redacted]");
        assert_eq!(redact("password = hunter2"), "password = [redacted]");
    }

    #[test]
    fn authorization_keeps_the_scheme_and_loses_the_credential() {
        assert_eq!(
            redact("Authorization: Bearer abc.def"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(
            redact("Authorization: Basic dXNlcjpwYXNz"),
            "Authorization: Basic [redacted]"
        );
    }

    #[test]
    fn a_long_opaque_token_is_redacted() {
        assert_eq!(
            redact("value 0123456789abcdefghijABCDEFGHIJ0123456789ab done"),
            "value [redacted] done"
        );
    }

    #[test]
    fn ordinary_lines_pass_through() {
        assert_eq!(redact("compositor started in 40 ms"), "compositor started in 40 ms");
        assert_eq!(redact("Basic setup complete"), "Basic setup complete");
    }

    #[test]
    fn a_private_key_block_is_redacted_across_lines() {
        let begin = ["-----BEGIN", "PRIVATE KEY-----"].join(" ");
        let end = ["-----END", "PRIVATE KEY-----"].join(" ");
        let mut redactor = Redactor::new();
        assert_eq!(redactor.redact(&begin), REDACTED);
        assert_eq!(redactor.redact("MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"), REDACTED);
        assert_eq!(redactor.redact(&end), REDACTED);
        assert_eq!(
            redactor.redact("panicked at src/main.rs:3"),
            "panicked at src/main.rs:3"
        );
    }
}

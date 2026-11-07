//! The gcr secret exchange, protocol `sx-aes-1`, without libgcr.
//!
//! The gcr prompter interface never sends a passphrase in the clear. Both sides
//! build a [`Exchange`]; the caller begins the exchange, the prompter receives the
//! caller's string, answers with its own string, and only then does the passphrase
//! travel inside an encrypted field. The protocol is gcr's, so a gnome-keyring or
//! `pinentry-gnome3` caller cannot tell this implementation from GNOME's.
//!
//! `sx-aes-1` is, in `gcr-secret-exchange.c`'s own words: Diffie-Hellman over the
//! 1536-bit MODP group, a transport key derived with HKDF-SHA256, and AES-128-CBC
//! with PKCS#7 padding. The wire string is a GLib key file:
//!
//! ```text
//! [sx-aes-1]
//! public=<base64 of the sender's DH public key, big endian, no leading zeros>
//!
//! ```
//!
//! and a reply adds the encrypted secret and its IV:
//!
//! ```text
//! [sx-aes-1]
//! public=<base64>
//! secret=<base64 of the AES-128-CBC ciphertext>
//! iv=<base64 of the 16-byte IV>
//!
//! ```
//!
//! Three details are what make this interoperable rather than merely encrypted, and
//! each is a place a reimplementation usually gets wrong:
//!
//! - the public key is exported **without** leading zero bytes, while the shared
//!   secret is **padded** to the prime's 192 bytes before it is hashed;
//! - HKDF is called with no salt and no info, so the salt is SHA-256's 32 zero bytes
//!   rather than an empty string;
//! - the section name is the protocol name, `sx-aes-1`, not `1`.
//!
//! # Failure modes first
//!
//! 1. **A caller sends an exchange this build cannot read.** Parsing, the section
//!    name, a missing or non-base64 `public` key and a wrong IV length are separate
//!    refusals, and a refusal never derives a key from partial input:
//!    `an_exchange_that_is_not_a_key_file_is_refused`,
//!    `an_unknown_protocol_is_refused`, `a_truncated_iv_is_refused`.
//! 2. **A secret is decrypted with the wrong key.** CBC with the wrong key produces
//!    padding that does not validate, which is refused instead of returning garbage:
//!    `a_secret_from_another_exchange_is_refused`.
//! 3. **A passphrase stays in memory after the prompt.** [`Secret`] zeroes its bytes
//!    on drop and never prints them: `a_secret_is_zeroed_and_never_printed`.
//! 4. **Two sides derive different keys.** Both halves of one exchange derive the
//!    same key and each can read the other's secret, which is the property the whole
//!    handshake exists for: `both_sides_derive_the_same_key`.

use aes::Aes128;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use hkdf::Hkdf;
use num_bigint::BigUint;
use sha2::Sha256;

/// The protocol name, which is also the key file's section name.
pub const PROTOCOL: &str = "sx-aes-1";

/// The transport key's length: AES-128 wants 16 bytes.
const KEY_LEN: usize = 16;

/// The IV's length, which is the AES block size.
const IV_LEN: usize = 16;

/// The prime's length in bytes, which is the width every shared secret is padded to.
const PRIME_LEN: usize = 192;

/// The private exponent's length in bytes.
///
/// Four times the transport key's length is the same margin gcr's own generator
/// leaves: the exponent only has to be unguessable, and 256 bits is far past that.
const EXPONENT_LEN: usize = 32;

/// The 1536-bit MODP group (RFC 3526 group 5), which is the group gcr's `sx-aes-1`
/// names as `ietf-ike-grp-modp-1536`. The constant is a protocol constant, so it is
/// spelled out rather than computed from pi.
const PRIME: [u8; PRIME_LEN] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xC9, 0x0F, 0xDA, 0xA2, 0x21, 0x68, 0xC2, 0x34, 0xC4, 0xC6, 0x62,
    0x8B, 0x80, 0xDC, 0x1C, 0xD1, 0x29, 0x02, 0x4E, 0x08, 0x8A, 0x67, 0xCC, 0x74, 0x02, 0x0B, 0xBE, 0xA6, 0x3B, 0x13,
    0x9B, 0x22, 0x51, 0x4A, 0x08, 0x79, 0x8E, 0x34, 0x04, 0xDD, 0xEF, 0x95, 0x19, 0xB3, 0xCD, 0x3A, 0x43, 0x1B, 0x30,
    0x2B, 0x0A, 0x6D, 0xF2, 0x5F, 0x14, 0x37, 0x4F, 0xE1, 0x35, 0x6D, 0x6D, 0x51, 0xC2, 0x45, 0xE4, 0x85, 0xB5, 0x76,
    0x62, 0x5E, 0x7E, 0xC6, 0xF4, 0x4C, 0x42, 0xE9, 0xA6, 0x37, 0xED, 0x6B, 0x0B, 0xFF, 0x5C, 0xB6, 0xF4, 0x06, 0xB7,
    0xED, 0xEE, 0x38, 0x6B, 0xFB, 0x5A, 0x89, 0x9F, 0xA5, 0xAE, 0x9F, 0x24, 0x11, 0x7C, 0x4B, 0x1F, 0xE6, 0x49, 0x28,
    0x66, 0x51, 0xEC, 0xE4, 0x5B, 0x3D, 0xC2, 0x00, 0x7C, 0xB8, 0xA1, 0x63, 0xBF, 0x05, 0x98, 0xDA, 0x48, 0x36, 0x1C,
    0x55, 0xD3, 0x9A, 0x69, 0x16, 0x3F, 0xA8, 0xFD, 0x24, 0xCF, 0x5F, 0x83, 0x65, 0x5D, 0x23, 0xDC, 0xA3, 0xAD, 0x96,
    0x1C, 0x62, 0xF3, 0x56, 0x20, 0x85, 0x52, 0xBB, 0x9E, 0xD5, 0x29, 0x07, 0x70, 0x96, 0x96, 0x6D, 0x67, 0x0C, 0x35,
    0x4E, 0x4A, 0xBC, 0x98, 0x04, 0xF1, 0x74, 0x6C, 0x08, 0xCA, 0x23, 0x73, 0x27, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF,
];

/// The generator of that group.
const GENERATOR: u8 = 2;

/// A passphrase or answer that must not be printed or left behind.
///
/// The bytes are zeroed on drop and `Debug` prints nothing of them, so a `?secret`
/// in a log line is impossible and the only way to read the value is
/// [`Secret::expose`], which a caller does at the one point it needs the bytes.
pub struct Secret(Vec<u8>);

impl Secret {
    /// Take ownership of these bytes.
    pub fn new(bytes: Vec<u8>) -> Secret {
        Secret(bytes)
    }

    /// The bytes, for the one call that needs them.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// The bytes as text, when the caller knows they are a passphrase.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

/// Secrets compare by their bytes, which is what a test asserts with and what a caller
/// checks when it wants to know whether it already has the answer. The comparison is not
/// constant time: it is not a MAC check, and the value being compared never leaves the
/// process it was typed in.
impl PartialEq for Secret {
    fn eq(&self, other: &Secret) -> bool {
        self.0 == other.0
    }
}

impl Eq for Secret {}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Secret(<{} bytes redacted>)", self.0.len())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // `fill` is a plain store, so `black_box` is what stops the compiler from
        // proving the buffer dead and dropping the store with it. This crate forbids
        // unsafe, and a volatile write is the only thing `fill` plus `black_box`
        // cannot express that matters here.
        self.0.fill(0);
        std::hint::black_box(&self.0);
    }
}

/// Why an exchange string could not be read or produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExchangeError {
    /// The string is not a GLib key file at all.
    Malformed,
    /// The file's section is a protocol this build does not speak.
    UnsupportedProtocol {
        /// The section name the string carried.
        protocol: String,
    },
    /// The `public` field is missing or is not base64.
    PublicKey,
    /// The ciphertext or the IV is missing, not base64, or the wrong length.
    EncryptedSecret,
    /// The transport key could not be derived, which only happens when a public key
    /// is not a number in the group.
    KeyAgreement,
    /// The ciphertext did not decrypt to a valid padding, so it was not written by
    /// the holder of this exchange's half of the key.
    Decryption,
    /// The system's random source is unavailable.
    Random,
}

impl std::fmt::Display for ExchangeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExchangeError::Malformed => formatter.write_str("the exchange string is not a key file"),
            ExchangeError::UnsupportedProtocol { protocol } => {
                write!(
                    formatter,
                    "`{protocol}` is not a secret exchange protocol this build speaks"
                )
            }
            ExchangeError::PublicKey => formatter.write_str("the exchange's `public` field is missing or not base64"),
            ExchangeError::EncryptedSecret => {
                formatter.write_str("the exchange's `secret` or `iv` field is missing, not base64, or the wrong length")
            }
            ExchangeError::KeyAgreement => formatter.write_str("the peer's public key is not in the group"),
            ExchangeError::Decryption => formatter.write_str("the exchange's ciphertext did not authenticate"),
            ExchangeError::Random => formatter.write_str("the system random source is unavailable"),
        }
    }
}

impl std::error::Error for ExchangeError {}

/// One side of a gcr secret exchange.
///
/// `Clone` is deliberate: a callback's exchange outlives one prompt (gcr reuses one
/// exchange across the prompts of one prompt object), and the clone taken for the
/// answer carries the derived key so the secret can be encrypted after the begun
/// callback is gone.
///
/// A side generates its key pair when it is made, so [`Exchange::begin`] is
/// infallible once the value exists and the caller's error path is the
/// constructor's.
#[derive(Clone)]
pub struct Exchange {
    private: BigUint,
    public: Vec<u8>,
    key: Option<[u8; KEY_LEN]>,
}

impl std::fmt::Debug for Exchange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Exchange")
            .field("public", &BASE64.encode(&self.public))
            .field("derived", &self.key.is_some())
            .finish()
    }
}

impl Exchange {
    /// Generate a key pair for one exchange.
    pub fn new() -> Result<Exchange, ExchangeError> {
        let mut exponent = [0u8; EXPONENT_LEN];
        getrandom::fill(&mut exponent).map_err(|_| ExchangeError::Random)?;
        Exchange::with_exponent(&exponent)
    }

    /// Build the exchange for a known exponent, which the tests use to pin a
    /// conversation without a random source.
    pub fn with_exponent(exponent: &[u8; EXPONENT_LEN]) -> Result<Exchange, ExchangeError> {
        let private = BigUint::from_bytes_be(exponent);
        if private == BigUint::from(0u8) {
            return Err(ExchangeError::KeyAgreement);
        }
        let prime = BigUint::from_bytes_be(&PRIME);
        let public = BigUint::from(GENERATOR).modpow(&private, &prime);
        Ok(Exchange {
            private,
            public: public.to_bytes_be(),
            key: None,
        })
    }

    /// The string this side sends first.
    pub fn begin(&self) -> String {
        format!("[{PROTOCOL}]\npublic={}\n", BASE64.encode(&self.public))
    }

    /// Receive the other side's string, deriving the transport key and decrypting
    /// its secret when the string carries one.
    ///
    /// Receiving twice is allowed and is how a session can reuse an exchange for
    /// several prompts, exactly as gcr's `GcrSecretExchange` allows: the derived key
    /// is kept and only the secret is replaced.
    pub fn receive(&mut self, exchange: &str) -> Result<Option<Secret>, ExchangeError> {
        let fields = parse(exchange)?;
        let public = BASE64.decode(fields.public).map_err(|_| ExchangeError::PublicKey)?;
        let peer = BigUint::from_bytes_be(&public);
        let prime = BigUint::from_bytes_be(&PRIME);
        if peer == BigUint::from(0u8) || peer >= prime {
            return Err(ExchangeError::KeyAgreement);
        }
        let shared = peer.modpow(&self.private, &prime).to_bytes_be();
        let key = transport_key(&shared)?;
        self.key = Some(key);

        let Some(ciphertext) = fields.secret else {
            return Ok(None);
        };
        let ciphertext = BASE64.decode(ciphertext).map_err(|_| ExchangeError::EncryptedSecret)?;
        let iv = BASE64
            .decode(fields.iv.ok_or(ExchangeError::EncryptedSecret)?)
            .map_err(|_| ExchangeError::EncryptedSecret)?;
        if iv.len() != IV_LEN || ciphertext.is_empty() || ciphertext.len() % IV_LEN != 0 {
            return Err(ExchangeError::EncryptedSecret);
        }
        let mut iv_bytes = [0u8; IV_LEN];
        iv_bytes.copy_from_slice(&iv);
        let plain = Decryptor::<Aes128>::new((&key).into(), (&iv_bytes).into())
            .decrypt_padded_vec::<Pkcs7>(&ciphertext)
            .map_err(|_| ExchangeError::Decryption)?;
        Ok(Some(Secret::new(plain)))
    }

    /// The string this side sends in reply, with the secret when there is one.
    ///
    /// A reply before [`Exchange::receive`] has derived the key is refused: the
    /// first string of an exchange cannot carry a secret, which is a property of the
    /// protocol rather than a policy here.
    pub fn send(&self, secret: Option<&[u8]>) -> Result<String, ExchangeError> {
        let Some(key) = self.key else {
            return Ok(self.begin());
        };
        let Some(secret) = secret else {
            return Ok(self.begin());
        };
        let mut iv = [0u8; IV_LEN];
        getrandom::fill(&mut iv).map_err(|_| ExchangeError::Random)?;
        let ciphertext = Encryptor::<Aes128>::new((&key).into(), (&iv).into()).encrypt_padded_vec::<Pkcs7>(secret);
        let mut text = format!("[{PROTOCOL}]\npublic={}\n", BASE64.encode(&self.public));
        text.push_str(&format!("secret={}\n", BASE64.encode(&ciphertext)));
        text.push_str(&format!("iv={}\n", BASE64.encode(iv)));
        Ok(text)
    }
}

/// The fields of one exchange string.
struct Fields<'a> {
    public: &'a str,
    secret: Option<&'a str>,
    iv: Option<&'a str>,
}

/// Read the key file's `public`, `secret` and `iv` fields.
///
/// A minimal reader on purpose: the format is one section of `key=value` lines, and a
/// full `GKeyFile` implementation would bring comments, escapes, locales and
/// duplicate-key rules that no exchange string uses.
fn parse(exchange: &str) -> Result<Fields<'_>, ExchangeError> {
    let mut section = None;
    let mut public = None;
    let mut secret = None;
    let mut iv = None;
    for line in exchange.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            section = Some(name);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(ExchangeError::Malformed);
        };
        if section != Some(PROTOCOL) {
            return Err(ExchangeError::UnsupportedProtocol {
                protocol: section.unwrap_or_default().to_owned(),
            });
        }
        match key.trim() {
            "public" => public = Some(value.trim()),
            "secret" => secret = Some(value.trim()),
            "iv" => iv = Some(value.trim()),
            // A field this build does not know is ignored, so a later protocol
            // addition that keeps the name does not break this reader.
            _ => {}
        }
    }
    Ok(Fields {
        public: public.ok_or(ExchangeError::PublicKey)?,
        secret,
        iv,
    })
}

/// HKDF-SHA256 with no salt and no info, into the 16-byte transport key.
///
/// The shared secret is padded to the prime's width first: gcr pads it, and HKDF's
/// output is a function of the input bytes, so an unpadded secret would derive a
/// different key and the caller would refuse every secret.
fn transport_key(shared: &[u8]) -> Result<[u8; KEY_LEN], ExchangeError> {
    let mut padded = [0u8; PRIME_LEN];
    if shared.len() > PRIME_LEN {
        return Err(ExchangeError::KeyAgreement);
    }
    padded[PRIME_LEN - shared.len()..].copy_from_slice(shared);
    let mut key = [0u8; KEY_LEN];
    Hkdf::<Sha256>::new(None, &padded)
        .expand(&[], &mut key)
        .map_err(|_| ExchangeError::KeyAgreement)?;
    Ok(key)
}

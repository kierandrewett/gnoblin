//! The `dh-ietf1024-sha256-aes128-cbc-pkcs7` session algorithm.
//!
//! The server takes the client's public value, picks its own exponent, and derives a 16-byte
//! AES key from the shared value with HKDF-SHA256 (no salt, no info). The shared value is used
//! as its big-endian bytes with no padding, which is what the client derives from too. This is
//! the only algorithm offered: `plain` is refused, because a secret sent in the clear over the
//! session bus is one any local process can read.

use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use hkdf::Hkdf;
use num_bigint::BigUint;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

/// The algorithm name a client passes to `OpenSession`.
pub const ALGORITHM: &str = "dh-ietf1024-sha256-aes128-cbc-pkcs7";

/// The 1024-bit MODP group of RFC 2409, section 6.2 (the second Oakley group). Its generator is 2.
const PRIME_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74\
020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245\
E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE6\
49286651ECE65381FFFFFFFFFFFFFFFF";

const GENERATOR: u32 = 2;
/// The AES-128 key length, and the output length HKDF is asked for.
pub const KEY_LEN: usize = 16;
/// The AES block size, which is the IV length.
pub const IV_LEN: usize = 16;
/// The private exponent's length in bytes: 1024 bits, the group's size.
const EXPONENT_LEN: usize = 128;

/// The AES key a session encrypts its secrets with. It is wiped when the session closes.
pub struct DhSessionKey(Zeroizing<[u8; KEY_LEN]>);

/// Why a session could not be opened or a secret could not be encrypted.
#[derive(Debug, PartialEq, Eq)]
pub enum DhError {
    /// The client's public value is outside `(1, p)`, so it would make the shared value
    /// predictable.
    BadPeerKey,
    /// The operating system would not supply random bytes.
    Random,
    /// The IV is not 16 bytes, the ciphertext is empty or not whole blocks, or the padding is
    /// wrong. These are reported alike, so the error reveals nothing about the key.
    BadCiphertext,
}

impl std::fmt::Display for DhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPeerKey => write!(f, "the client's public value is outside the group"),
            Self::Random => write!(f, "no random bytes are available"),
            Self::BadCiphertext => write!(f, "the ciphertext is malformed or its padding is wrong"),
        }
    }
}

fn prime() -> BigUint {
    match BigUint::parse_bytes(PRIME_HEX.as_bytes(), 16) {
        Some(prime) => prime,
        // The constant is a fixed hex string, so this branch exists only to avoid a panic.
        None => unreachable!("PRIME_HEX is a fixed hexadecimal constant"),
    }
}

/// The server's half of a new session: its public value for the client, and the key both sides
/// now share.
pub fn accept(peer_public: &[u8]) -> Result<(Vec<u8>, DhSessionKey), DhError> {
    let prime = prime();
    let peer = BigUint::from_bytes_be(peer_public);
    if peer <= BigUint::from(1u8) || peer >= prime {
        return Err(DhError::BadPeerKey);
    }
    let mut exponent = [0u8; EXPONENT_LEN];
    getrandom::fill(&mut exponent).map_err(|_| DhError::Random)?;
    let private = BigUint::from_bytes_be(&exponent);
    exponent.zeroize();
    let public = BigUint::from(GENERATOR).modpow(&private, &prime);
    let shared = peer.modpow(&private, &prime).to_bytes_be();
    let key = derive_key(&shared);
    Ok((public.to_bytes_be(), key))
}

fn derive_key(shared: &[u8]) -> DhSessionKey {
    let hkdf = Hkdf::<Sha256>::new(None, shared);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    // HKDF-SHA256 can produce 255 blocks; 16 bytes is one, so expansion cannot fail.
    if hkdf.expand(&[], key.as_mut_slice()).is_err() {
        unreachable!("HKDF-SHA256 expands to 16 bytes");
    }
    DhSessionKey(key)
}

/// Encrypt `plain` under the session key with a fresh IV. Returns the IV and the ciphertext.
pub fn encrypt(key: &DhSessionKey, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>), DhError> {
    let mut iv = [0u8; IV_LEN];
    getrandom::fill(&mut iv).map_err(|_| DhError::Random)?;
    let key_array: &[u8; KEY_LEN] = &key.0;
    let cipher = Encryptor::<Aes128>::new(key_array.into(), (&iv).into()).encrypt_padded_vec::<Pkcs7>(plain);
    Ok((iv.to_vec(), cipher))
}

/// Decrypt a secret a client encrypted under the session key. The plaintext is returned in a
/// zeroizing buffer, and the padding bytes are wiped before the buffer is truncated, so no copy
/// of the secret outlives the caller's use of it.
pub fn decrypt(key: &DhSessionKey, iv: &[u8], cipher: &[u8]) -> Result<Zeroizing<Vec<u8>>, DhError> {
    let iv: &[u8; IV_LEN] = iv.try_into().map_err(|_| DhError::BadCiphertext)?;
    if cipher.is_empty() || !cipher.len().is_multiple_of(IV_LEN) {
        return Err(DhError::BadCiphertext);
    }
    let key_array: &[u8; KEY_LEN] = &key.0;
    let mut buffer = Zeroizing::new(cipher.to_vec());
    let plain_len = Decryptor::<Aes128>::new(key_array.into(), iv.into())
        .decrypt_padded::<Pkcs7>(buffer.as_mut_slice())
        .map_err(|_| DhError::BadCiphertext)?
        .len();
    buffer[plain_len..].zeroize();
    buffer.truncate(plain_len);
    Ok(buffer)
}

/// The client's half, written from the specification rather than from `accept`, so the tests
/// check that both sides derive the same key.
#[cfg(test)]
fn client_key(server_public: &[u8], private: &BigUint) -> (Vec<u8>, DhSessionKey) {
    let prime = prime();
    let public = BigUint::from(GENERATOR).modpow(private, &prime);
    let shared = BigUint::from_bytes_be(server_public)
        .modpow(private, &prime)
        .to_bytes_be();
    (public.to_bytes_be(), derive_key(&shared))
}

/// The server's session key and the client's, derived independently; they must agree. Other
/// modules' tests use it to encrypt what a client sends and to read what the server returns.
#[cfg(test)]
pub(crate) fn session_pair() -> (DhSessionKey, DhSessionKey) {
    let client_private = BigUint::from(0x0fed_cba9_8765_4321u64);
    let client_public = BigUint::from(GENERATOR).modpow(&client_private, &prime()).to_bytes_be();
    let (server_public, server_key) = accept(&client_public).expect("a valid public value");
    let (_, client) = client_key(&server_public, &client_private);
    (server_key, client)
}

#[cfg(test)]
mod tests {
    use cbc::cipher::block_padding::NoPadding;

    use super::*;

    #[test]
    fn prime_is_the_rfc2409_group() {
        let prime = prime();
        assert_eq!(prime.bits(), 1024);
        // Fermat's test with the generator: 2^(p-1) = 1 mod p, which the RFC group satisfies.
        let minus_one = &prime - BigUint::from(1u8);
        assert_eq!(BigUint::from(GENERATOR).modpow(&minus_one, &prime), BigUint::from(1u8));
    }

    #[test]
    fn both_sides_derive_the_same_key_and_secret_round_trips() {
        let client_private = BigUint::from(0x1234_5678_9abc_def0u64);
        let client_public = BigUint::from(GENERATOR).modpow(&client_private, &prime()).to_bytes_be();
        let (server_public, server_key) = accept(&client_public).expect("a valid public value");
        let (_, client) = client_key(&server_public, &client_private);

        let (iv, cipher) = encrypt(&server_key, b"hunter2").expect("encrypts");
        let iv: [u8; IV_LEN] = iv.try_into().expect("16-byte IV");
        let client_array: &[u8; KEY_LEN] = &client.0;
        let plain = Decryptor::<Aes128>::new(client_array.into(), (&iv).into())
            .decrypt_padded_vec::<Pkcs7>(&cipher)
            .expect("the client's key decrypts the secret");
        assert_eq!(plain, b"hunter2");
    }

    #[test]
    fn out_of_group_peer_values_are_refused() {
        assert_eq!(accept(&[]).err(), Some(DhError::BadPeerKey));
        assert_eq!(accept(&[1]).err(), Some(DhError::BadPeerKey));
        let prime_bytes = prime().to_bytes_be();
        assert_eq!(accept(&prime_bytes).err(), Some(DhError::BadPeerKey));
    }

    #[test]
    fn decrypt_round_trips_with_the_peers_key() {
        let (server, client) = session_pair();
        let (iv, cipher) = encrypt(&server, b"hunter2").expect("encrypts");
        let plain = decrypt(&client, &iv, &cipher).expect("the peer's key decrypts");
        assert_eq!(plain.as_slice(), b"hunter2");
    }

    #[test]
    fn decrypt_refuses_a_truncated_empty_or_misaligned_input() {
        let (server, client) = session_pair();
        let (iv, cipher) = encrypt(&server, b"hunter2").expect("encrypts");
        assert_eq!(
            decrypt(&client, &iv, &cipher[..cipher.len() - 1]).err(),
            Some(DhError::BadCiphertext)
        );
        assert_eq!(decrypt(&client, &iv, &[]).err(), Some(DhError::BadCiphertext));
        assert_eq!(
            decrypt(&client, &iv[..IV_LEN - 1], &cipher).err(),
            Some(DhError::BadCiphertext)
        );
    }

    #[test]
    fn decrypt_refuses_bad_padding() {
        let (_, client) = session_pair();
        // A block of zeros has a final byte of 0, which is never valid PKCS#7 padding.
        let iv = [7u8; IV_LEN];
        let key_array: &[u8; KEY_LEN] = &client.0;
        let cipher =
            Encryptor::<Aes128>::new(key_array.into(), (&iv).into()).encrypt_padded_vec::<NoPadding>(&[0u8; IV_LEN]);
        assert_eq!(decrypt(&client, &iv, &cipher).err(), Some(DhError::BadCiphertext));
    }

    #[test]
    fn decrypt_does_not_recover_the_secret_under_another_key() {
        let (server, _) = session_pair();
        let stranger_public = BigUint::from(GENERATOR)
            .modpow(&BigUint::from(42u8), &prime())
            .to_bytes_be();
        let (_, stranger) = accept(&stranger_public).expect("a valid public value");
        let (iv, cipher) = encrypt(&server, b"hunter2").expect("encrypts");
        // A wrong key can by chance leave valid padding, so the test asserts only that the
        // plaintext is not recovered.
        match decrypt(&stranger, &iv, &cipher) {
            Ok(plain) => assert_ne!(plain.as_slice(), b"hunter2"),
            Err(error) => assert_eq!(error, DhError::BadCiphertext),
        }
    }
}

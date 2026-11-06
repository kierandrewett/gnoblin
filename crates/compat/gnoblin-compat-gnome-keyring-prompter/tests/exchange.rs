//! The gcr secret exchange against itself: two independent halves, one key.
//!
//! These are the tests that would catch a protocol mistake no amount of round-tripping
//! inside one implementation catches, except where noted: they pin the wire format's
//! fields, the failure of a wrong key, and the promise that a passphrase is not printed.

use std::fmt::Write as _;

use gnoblin_compat_gnome_keyring_prompter::{Exchange, ExchangeError, Secret};

const PASSPHRASE: &[u8] = b"correct horse battery staple";

fn pinned(exponent: u8) -> Exchange {
    Exchange::with_exponent(&[exponent; 32]).expect("a non-zero exponent is a key pair")
}

#[test]
fn both_sides_derive_the_same_key() {
    // The property the handshake exists for: the caller's exchange and the prompter's
    // answer are two halves of one key, and each side can read the other's secret.
    let mut caller = pinned(1);
    let mut prompter = pinned(2);

    let caller_string = caller.begin();
    prompter
        .receive(&caller_string)
        .expect("the prompter reads the caller's half");
    let answer = prompter.send(Some(PASSPHRASE)).expect("the prompter can answer");

    let secret = caller
        .receive(&answer)
        .expect("the caller reads the answer")
        .expect("the answer carries a secret");
    assert_eq!(secret.expose(), PASSPHRASE);
}

#[test]
fn the_first_string_carries_no_secret() {
    // The protocol's own rule: an exchange's first string has only a public key, so
    // nothing can be encrypted before the key is derived.
    let exchange = pinned(3);
    let first = exchange.begin();
    assert!(first.starts_with("[sx-aes-1]\npublic="), "{first}");
    assert!(!first.contains("secret="));
    assert!(!first.contains("iv="));
}

#[test]
fn an_exchange_that_is_not_a_key_file_is_refused() {
    let mut exchange = pinned(4);
    assert_eq!(exchange.receive("not a key file"), Err(ExchangeError::Malformed));
    assert_eq!(exchange.receive("[sx-aes-1]\nnovalue\n"), Err(ExchangeError::Malformed));
    assert_eq!(
        exchange.receive("[sx-aes-1]\nsecret=AAAA\n"),
        Err(ExchangeError::PublicKey),
        "an exchange without a public key cannot derive anything"
    );
    assert_eq!(
        exchange.receive("[sx-aes-1]\npublic=not base64!\n"),
        Err(ExchangeError::PublicKey)
    );
}

#[test]
fn an_unknown_protocol_is_refused() {
    // A future protocol must be refused rather than guessed at: a key derived from the
    // wrong scheme would silently fail to decrypt, which looks like a wrong passphrase.
    let mut exchange = pinned(5);
    assert_eq!(
        exchange.receive("[sx-aes-2]\npublic=Ag==\n"),
        Err(ExchangeError::UnsupportedProtocol {
            protocol: "sx-aes-2".to_owned()
        })
    );
}

#[test]
fn a_truncated_iv_is_refused() {
    // The failure mode: a caller sends a well-formed exchange whose IV is not 16 bytes.
    // CBC must not be run with a partial IV.
    let mut caller = pinned(6);
    let mut prompter = pinned(7);
    prompter.receive(&caller.begin()).expect("derive");
    let answer = prompter.send(Some(PASSPHRASE)).expect("answer");
    let truncated = answer.replace(
        &answer
            .split("iv=")
            .nth(1)
            .expect("the answer has an iv")
            .trim_end()
            .to_owned(),
        "AAAA",
    );
    assert_eq!(caller.receive(&truncated), Err(ExchangeError::EncryptedSecret));
}

#[test]
fn a_secret_from_another_exchange_is_refused() {
    // The failure mode: a ciphertext encrypted under a different key. CBC with the wrong
    // key produces bytes that do not satisfy the padding, and refusing them is what stops
    // a caller from reading garbage as a passphrase.
    let mut caller = pinned(8);
    let other = pinned(9);
    let mut prompter = pinned(10);
    prompter.receive(&other.begin()).expect("other derives");
    let answer = prompter.send(Some(PASSPHRASE)).expect("answer");
    let first = caller.begin();
    caller.receive(&first).expect("first half");
    assert_eq!(caller.receive(&answer), Err(ExchangeError::Decryption));
}

#[test]
fn a_secret_is_zeroed_and_never_printed() {
    // The failure mode: a passphrase ends up in a log line through a `{:?}`. The type has
    // no way to print it, and the buffer is cleared when it is dropped.
    let secret = Secret::new(PASSPHRASE.to_vec());
    let mut printed = String::new();
    write!(&mut printed, "{secret:?}").expect("formatting a secret cannot fail");
    assert!(
        !printed.contains("correct"),
        "a secret's Debug output must not contain it: {printed}"
    );
    assert_eq!(secret.expose(), PASSPHRASE);
    assert_eq!(secret.as_str(), Some("correct horse battery staple"));
    drop(secret);
    assert_eq!(
        Secret::new(vec![0xff]).as_str(),
        None,
        "binary answers have no text form"
    );
}

#[test]
fn an_empty_passphrase_is_still_a_passphrase() {
    // An empty passphrase is what a user may legitimately enter, so the exchange must
    // carry it as a secret rather than as "no secret".
    let mut caller = pinned(11);
    let mut prompter = pinned(12);
    prompter.receive(&caller.begin()).expect("derive");
    let answer = prompter.send(Some(b"")).expect("an empty secret encrypts");
    let secret = caller
        .receive(&answer)
        .expect("reads the answer")
        .expect("the answer carries a secret");
    assert!(secret.expose().is_empty());
}

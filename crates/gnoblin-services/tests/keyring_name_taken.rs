//! A second Secret Service must refuse the name, name its owner, and bind nothing.
//!
//! Apps reach whichever program holds `org.freedesktop.secrets`. When another program (for
//! example gnome-keyring, started by PAM or by activation) already owns it, Gnoblin must not
//! serve, must not take the name back, and must not open the unlock hand-off socket. Every test
//! runs on its own private bus (see `gnoblin-service-testkit`).

use std::path::PathBuf;

use gnoblin_service_keyring::{PasswordOffer, PromptId, ServeError, serve_on};
use gnoblin_service_testkit::Bus;
use zbus::Connection;
use zbus::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

const NAME: &str = "org.freedesktop.secrets";

/// Nothing in these tests asks for a password, so every offer is refused. A refused offer fails
/// closed, which is the behaviour the store should show with no prompt handler.
struct NoOffer;

impl PasswordOffer for NoOffer {
    fn offer(&mut self, _title: &str, _message: &str) -> Result<PromptId, String> {
        Err("no prompt handler in this test".to_owned())
    }
}

async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

/// Paths for one test's store and socket. They sit under the temp dir and carry this process's
/// id, so a leftover from an earlier run cannot make a test pass. Each test owns its directory:
/// `handoff::listen` sets the socket's parent directory to 0700, which must be a directory this
/// test created, never a shared one such as `/tmp` (root-owned, so the chmod fails).
fn unused_paths(label: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("gnoblin-name-taken-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (dir.join("keyring.store"), dir.join("handoff.sock"))
}

/// A foreign program that already owns the name on `address`.
async fn rival(address: &str) -> Connection {
    let connection = connect(address).await;
    let reply = connection
        .request_name_with_flags(NAME, RequestNameFlags::DO_NOT_QUEUE)
        .await
        .expect("the rival requests the name");
    assert_eq!(reply, RequestNameReply::PrimaryOwner, "the rival takes the free name");
    connection
}

#[test]
fn refuses_when_another_program_owns_the_name() {
    let bus = Bus::start();
    let rival = zbus::block_on(rival(bus.address()));
    let rival_name = rival.unique_name().expect("the rival is connected").to_string();

    let gnoblin = zbus::block_on(connect(bus.address()));
    let (store, socket) = unused_paths("foreign");
    let result = zbus::block_on(serve_on(gnoblin, &store, &socket, Box::new(NoOffer)));

    match result {
        Err(ServeError::NameTaken { owner }) => {
            assert_eq!(owner, rival_name, "the refusal names the program that holds the name");
        }
        Err(other) => panic!("expected NameTaken, got {other}"),
        Ok(_) => panic!("a second Secret Service must not be served over a held name"),
    }
    assert!(
        !socket.exists(),
        "the unlock hand-off socket is not bound when the name is refused"
    );
    assert!(!store.exists(), "the store is not created when the name is refused");
}

#[test]
fn a_second_gnoblin_instance_is_refused_too() {
    let bus = Bus::start();
    let (first_store, first_socket) = unused_paths("first");
    let first = zbus::block_on(connect(bus.address()));
    let (connection, _secrets, _answers) =
        zbus::block_on(serve_on(first, &first_store, &first_socket, Box::new(NoOffer)))
            .expect("the first instance takes the free name");
    let first_name = connection
        .unique_name()
        .expect("the first instance is connected")
        .to_string();

    let (store, socket) = unused_paths("second");
    let second = zbus::block_on(connect(bus.address()));
    match zbus::block_on(serve_on(second, &store, &socket, Box::new(NoOffer))) {
        Err(ServeError::NameTaken { owner }) => assert_eq!(owner, first_name),
        Err(other) => panic!("expected NameTaken, got {other}"),
        Ok(_) => panic!("two Gnoblin keyrings must not serve the same name"),
    }
    assert!(!socket.exists(), "the second instance binds no hand-off socket");
    let _ = std::fs::remove_file(&first_socket);
}

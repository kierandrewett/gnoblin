//! Tests of this module.

use super::eval::{as_expression, console_location};
use super::*;

#[test]
fn only_the_shells_own_uid_is_served() {
    assert!(peer_allowed(Some(1000), 1000));
    assert!(!peer_allowed(Some(0), 1000));
    assert!(!peer_allowed(Some(1001), 1000));
    assert!(
        !peer_allowed(None, 1000),
        "a peer whose credentials cannot be read is refused"
    );
}

#[test]
fn a_connection_from_this_process_has_this_uid() {
    let (near, _far) = UnixStream::pair().expect("a socket pair");
    assert!(
        stream_allowed(&near),
        "the kernel's record of a local peer is our own uid"
    );
}

#[test]
fn the_console_socket_sits_beside_the_owner_socket() {
    let env = |name: &str| (name == "GNOBLIN_IPC_SOCKET").then(|| "/run/user/1000/gnoblin/ipc.sock".to_owned());
    assert_eq!(
        console_socket_path(&env).expect("a path"),
        PathBuf::from("/run/user/1000/gnoblin/js.sock")
    );
    let runtime = |name: &str| (name == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_owned());
    assert_eq!(
        console_socket_path(&runtime).expect("a path"),
        PathBuf::from("/run/user/1000/gnoblin/js.sock")
    );
    assert!(console_socket_path(&|_: &str| None).is_err());
}

#[test]
fn the_console_is_on_unless_the_variable_is_zero() {
    let with = |value: &'static str| move |name: &str| (name == CONSOLE_VARIABLE).then(|| value.to_owned());
    assert!(console_enabled(&|_: &str| None));
    assert!(console_enabled(&with("1")));
    assert!(!console_enabled(&with("0")));
}

#[test]
fn a_stack_frame_in_the_console_gives_the_location() {
    assert_eq!(
        console_location("    at <eval> (<console>:3:14)\n    at f (gnoblin-core/host:9:1)"),
        Some((3, 14))
    );
    assert_eq!(console_location("    at <console>:1:5\n"), Some((1, 5)));
    assert_eq!(console_location("    at f (gnoblin-core/host:9:1)"), None);
}

#[test]
fn an_object_literal_at_the_prompt_is_an_object_and_a_block_is_a_block() {
    assert_eq!(as_expression("{a: 1}"), "({a: 1}\n)");
    assert_eq!(as_expression("{ let a = 1; a }"), "{ let a = 1; a }");
    assert_eq!(as_expression("1 + 1"), "1 + 1");
}

#[test]
fn a_stale_socket_file_is_replaced_and_a_live_one_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    // A socket nobody listens on any more: bind, then drop the listener without removing the file.
    drop(UnixListener::bind(&path).expect("a first socket"));
    assert!(path.exists());
    let first = JsConsole::bind(&path).expect("a stale socket is replaced");
    assert_eq!(
        std::fs::metadata(&path).expect("the socket").permissions().mode() & 0o777,
        0o600,
        "the socket is private to its owner"
    );
    assert!(matches!(JsConsole::bind(&path), Err(JsConsoleError::Busy { .. })));
    drop(first);
    assert!(!path.exists(), "the socket file goes with the console");
}

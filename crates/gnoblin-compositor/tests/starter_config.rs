//! The shipped starter configuration, loaded the way a user gets it.
//!
//! `gnoblinctl init` writes `data/default-config` into the user's config directory, and the
//! session then runs its `init.lua`, which loads every `config/*.lua`. This test copies the
//! whole tree into a temporary directory and runs it through a real `Host` on a real
//! compositor socket. The starter must evaluate without a script error and validate
//! without a single refused or unknown key: a starter that refuses a key tells every new
//! user the session is broken before they have changed anything.

mod support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::version;
use gnoblin_config::SearchPaths;
use gnoblin_lua::{Host, Limits};
use support::Session;

/// The tree the session embeds and `gnoblinctl init` writes.
const STARTER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/default-config");

/// The read deadline a role's client carries while its host runs a configuration.
const HOST_PUMP: Duration = Duration::from_millis(100);

/// The files the starter tree holds, which the run must evaluate: the entry point and the
/// five `config/*.lua` files it loads. A new file that is not loaded would be silently unused,
/// so the count is checked too.
const STARTER_FILES: usize = 6;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a test can create a directory");
    for entry in fs::read_dir(from).expect("the starter tree is readable") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("a test can copy a file");
        }
    }
}

#[test]
fn the_shipped_starter_config_loads_with_no_script_error_and_no_refused_key() {
    let session = Session::start();
    let directory = tempfile::tempdir().expect("a temporary directory");
    copy_tree(Path::new(STARTER), directory.path());
    let init = directory.path().join("init.lua");

    let mut client = Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake");
    client.set_read_timeout(Some(HOST_PUMP)).expect("a read deadline");
    let mut host = Host::new(client, Limits::default()).expect("the host starts");
    let paths = SearchPaths {
        home: None,
        config_home: None,
        override_path: Some(init),
    };
    let report = host.run_config(&paths).expect("the run starts");

    let script_errors: Vec<String> = report.errors.iter().map(|error| error.message().to_owned()).collect();
    assert!(
        script_errors.is_empty(),
        "the starter reported script errors:\n{}",
        script_errors.join("\n")
    );
    if let Err(problems) = &report.config {
        panic!("the starter config is refused at validation:\n{problems}");
    }
    assert_eq!(
        report.loaded.len(),
        STARTER_FILES,
        "the run evaluated {:?}; every starter file must load",
        report.loaded
    );
}

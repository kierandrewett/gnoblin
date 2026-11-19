//! The dry run of a shell: `gnoblin --role check PATH`.
//!
//! A shell is a module the core loads, so what a module would do when the session starts it is found out by starting it: this role
//! loads `PATH` the way the shell role does, in a core generation of its own, over a compositor with nothing in it. The tree has the
//! root and one seat, which every compositor has, and no window, output or device; it answers every attach and accepts every
//! transaction. Nothing is spawned, no claim is made, no connection is opened. The shell's libraries are the installed ones (a
//! script that imports the std package runs it over the std library, as in a session), so a refusal the library makes is made here too.
//!
//! The answer is on standard output: a first line, `ok` or `error`, and for an error the words the loader gave, as many lines as
//! they take. The exit code is 0 for `ok` and 1 for `error`. A host that wants the failure of a script as data sets
//! `GNOBLIN_START_FAILURE=json` in the environment of this process: the words are then the JSON object the JavaScript runtime
//! describes (`gnoblin-runtime-js`, `instance.rs`).

use std::cell::RefCell;
use std::sync::Arc;

use gnoblin_compositor::inproc_shell::ShellReports;
use gnoblin_log::ProcessRole;
use gnoblin_log::init::{Console, LogOptions, LogSettings, init};
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx,
};

use crate::host;
use crate::native_shell::OneLink;
use crate::role::{EXIT_FAILED, ShellModule};

/// The id of the inert tree's root node.
const ROOT: u64 = 1;

/// The id of the inert tree's one seat, so `gnoblin.input.primary` is there for a config that binds keys.
const SEAT: u64 = 2;

/// A compositor with nothing in it: the root node and a seat, which answers every attach and accepts every transaction.
#[derive(Default)]
struct InertTree {
    queue: Vec<TreeIncoming>,
}

impl TreeLink for InertTree {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        self.queue.push(TreeIncoming::Attached(Attached {
            epoch: 1,
            root: ROOT,
            serial: 0,
        }));
        self.queue.push(TreeIncoming::Snapshot {
            serial: 0,
            chunk: 0,
            last: true,
            nodes: vec![
                SnapshotNode {
                    id: ROOT,
                    parent: 0,
                    index: 0,
                    class: "node".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: Vec::new(),
                },
                SnapshotNode {
                    id: SEAT,
                    parent: ROOT,
                    index: 0,
                    class: "seat".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: vec![("name".to_owned(), MirrorValue::Text("seat0".to_owned()))],
                },
            ],
        });
        Ok(())
    }

    fn transact(&mut self, _tx: &TreeTx) -> Result<(), LinkError> {
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.queue))
    }
}

/// What the shell reports to nobody.
struct Nobody;

impl ShellReports for Nobody {}

/// Loads the module over the inert tree, prints the verdict and returns the exit code.
pub(crate) fn serve(module: &ShellModule) -> i32 {
    let settings = LogSettings {
        console: Console::Off,
        ..LogSettings::default()
    };
    let _log = init(ProcessRole::Compositor, &LogOptions::from_environment(&settings));
    host::reset_session();
    host::set_tree_link_provider(Box::new(OneLink(RefCell::new(Some(Box::<InertTree>::default())))));
    host::set_reports(Arc::new(Nobody));
    let mut shell = host::native_shell(&module.path);
    if let Some(language) = &module.language {
        shell = shell.with_language(language);
    }
    let code = match shell.start() {
        Ok(_) => {
            shell.stop();
            println!("ok");
            0
        }
        Err(error) => {
            println!("error");
            println!("{error}");
            EXIT_FAILED
        }
    };
    host::reset_session();
    code
}

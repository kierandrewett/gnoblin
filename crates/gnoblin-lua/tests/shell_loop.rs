//! The shell loop over the real socket wire: a change batch from the compositor makes no
//! transaction, and a frame in which the VM wrote something sends exactly one.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use gnoblin_lua::shell_vm::ShellVm;
use gnoblin_lua::tree_core::{ClassSpec, PropSpec, TreeValueKind};
use gnoblin_lua::tree_link::SocketTreeLink;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, ChangesPayload, SnapshotPayload};
use gnoblin_proto::tree::{change, snapshot_node};
use gnoblin_proto::{Envelope, read_frame, write_frame};

const WAIT: Duration = Duration::from_secs(10);
/// How long a frame that should send nothing is watched before it counts as silent.
const QUIET: Duration = Duration::from_millis(300);

fn lua_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

fn classes() -> Vec<ClassSpec> {
    let node = ClassSpec {
        name: "Node".into(),
        base: None,
        props: vec![
            PropSpec {
                name: "children".into(),
                kind: TreeValueKind::Nodes,
                readonly: true,
                default: None,
            },
            PropSpec {
                name: "parent".into(),
                kind: TreeValueKind::Node,
                readonly: true,
                default: None,
            },
            PropSpec {
                name: "title".into(),
                kind: TreeValueKind::Text,
                readonly: false,
                default: None,
            },
        ],
        signals: vec!["ready".into()],
        ..ClassSpec::default()
    };
    let window = ClassSpec {
        name: "Window".into(),
        base: Some("Node".into()),
        ..ClassSpec::default()
    };
    vec![node, window]
}

/// The next frame the VM wrote, or a panic when none arrives in time.
fn receive(server: &mut UnixStream) -> Envelope {
    server.set_read_timeout(Some(WAIT)).expect("the deadline");
    read_frame(server).expect("a frame").expect("the VM has not closed")
}

/// Asserts the VM wrote nothing in the quiet window.
fn assert_silent(server: &mut UnixStream) {
    server.set_read_timeout(Some(QUIET)).expect("the deadline");
    let outcome = read_frame(server);
    assert!(outcome.is_err(), "the VM must not write a frame here, got {outcome:?}");
}

/// Plays the compositor's side of the attach: reads the request, answers with the attached
/// reply and a two-node snapshot, then hands the connection back to the test.
fn serve_attach(mut server: UnixStream) -> UnixStream {
    server.set_read_timeout(Some(WAIT)).expect("the deadline");
    let request = read_frame(&mut server).expect("a frame").expect("the VM sent attach");
    let args: AttachArgs = request.payload_as().expect("the attach args decode");
    assert_eq!(args.version, 1);
    write_frame(
        &mut server,
        &Envelope::reply_message(
            "tree",
            "attach",
            request.request_id,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 0,
            },
        ),
    )
    .expect("the attached reply");
    write_frame(
        &mut server,
        &Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 0,
                chunk: 0,
                last: true,
                nodes: vec![
                    snapshot_node::Properties {
                        id: 1,
                        parent: 0,
                        index: 0,
                        class: "Node".to_owned(),
                        title: String::new(),
                        app_id: String::new(),
                        props: vec![],
                    },
                    snapshot_node::Properties {
                        id: 2,
                        parent: 1,
                        index: 0,
                        class: "Window".to_owned(),
                        title: String::new(),
                        app_id: String::new(),
                        props: vec![],
                    },
                ],
            },
        ),
    )
    .expect("the snapshot event");
    server
}

/// A change batch at `serial` setting the title of node 2 to `title`, as the compositor sends it.
fn title_changes(serial: u64, title: &str) -> Envelope {
    Envelope::event_message(
        "tree",
        "changes",
        &ChangesPayload {
            serial,
            changes: vec![change::Properties {
                kind: "set".to_owned(),
                node: 2,
                key: "title".to_owned(),
                value_kind: "text".to_owned(),
                value_text: title.to_owned(),
                ..Default::default()
            }],
        },
    )
}

#[test]
fn compositor_changes_send_nothing_and_a_frame_with_writes_sends_exactly_one_transaction() {
    let (client, server) = UnixStream::pair().expect("a socketpair");
    let compositor = thread::spawn(move || serve_attach(server));
    let link = SocketTreeLink::new(client).expect("the link wraps the client end");
    let mut vm = ShellVm::new(Box::new(link), &lua_dir(), classes()).expect("boot the shell VM");
    let mut server = compositor.join().expect("the fake compositor finished");

    // The compositor changes a title; the VM only applies it and writes nothing.
    write_frame(&mut server, &title_changes(1, "from the compositor")).expect("the change batch");
    vm.frame(0.0, vec![]).expect("the frame that applies the change");
    assert_silent(&mut server);

    // The VM writes a node in a frame: one transaction, then nothing more.
    vm.run("create", r#"local n = Node(); n.title = "new"; root:add(n)"#)
        .expect("run the write");
    vm.frame(1.0, vec![]).expect("the frame that sends the write");
    let transaction = receive(&mut server);
    assert_eq!(
        (transaction.namespace.as_str(), transaction.member.as_str()),
        ("tree", "transact"),
        "the write must go out as a transact request"
    );
    assert_silent(&mut server);
}

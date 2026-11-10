//! Nodes present in the attach snapshot get `ready` once each, parents first, on the first
//! frame after the shell library is loaded; a node created later still fires `ready` once.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use gnoblin_lua::shell_vm::ShellVm;
use gnoblin_lua::tree_core::{ClassSpec, PropSpec, TreeValueKind};
use gnoblin_lua::tree_link::SocketTreeLink;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, read_frame, write_frame};

const WAIT: Duration = Duration::from_secs(10);

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
    let seat = ClassSpec {
        name: "Seat".into(),
        base: Some("Node".into()),
        props: vec![PropSpec {
            name: "screen".into(),
            kind: TreeValueKind::Node,
            readonly: false,
            default: None,
        }],
        ..ClassSpec::default()
    };
    let screen = ClassSpec {
        name: "Screen".into(),
        base: Some("Node".into()),
        ..ClassSpec::default()
    };
    vec![node, seat, screen]
}

fn node(id: u64, parent: u64, class: &str, title: &str) -> snapshot_node::Properties {
    snapshot_node::Properties {
        id,
        parent,
        index: 0,
        class: class.to_owned(),
        title: title.to_owned(),
        app_id: String::new(),
        props: vec![],
    }
}

/// Plays the compositor's side of the attach: root 1 holds seat 2, which holds screen 3.
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
                    node(1, 0, "Node", "root"),
                    node(2, 1, "Seat", "seat"),
                    node(3, 2, "Screen", "screen"),
                ],
            },
        ),
    )
    .expect("the snapshot event");
    server
}

#[test]
fn snapshot_nodes_get_ready_once_parents_first_and_the_screen_handler_runs() {
    let (client, server) = UnixStream::pair().expect("a socketpair");
    let compositor = thread::spawn(move || serve_attach(server));
    let link = SocketTreeLink::new(client).expect("the link wraps the client end");
    let mut vm = ShellVm::new(Box::new(link), &lua_dir(), classes()).expect("boot the shell VM");
    let _server = compositor.join().expect("the fake compositor finished");

    vm.run(
        "library",
        r#"
        READY = {}
        function Node:on_ready() READY[#READY + 1] = self.title or 'node' end
        "#,
    )
    .expect("load the handlers");

    vm.frame(0.0, vec![])
        .expect("the first frame emits ready for the snapshot");
    let order = vm
        .run("order", r#"return table.concat(READY, ",")"#)
        .expect("read the order")
        .expect("a string");
    assert_eq!(order, "root,seat,screen", "ready once each, parents first");

    vm.frame(1.0, vec![]).expect("a later frame emits nothing again");
    let again = vm
        .run("again", r#"return tostring(#READY)"#)
        .expect("count")
        .expect("a string");
    assert_eq!(again, "3", "ready must not fire again on later frames");

    vm.run("created", r#"READY = {}; root:add(Node())"#)
        .expect("a node created after boot");
    vm.frame(2.0, vec![]).expect("the frame after creation");
    let created = vm
        .run("created-ready", r#"return tostring(#READY)"#)
        .expect("count")
        .expect("a string");
    assert_eq!(
        created, "1",
        "a node added after boot fires ready exactly once, through Node:add"
    );
}

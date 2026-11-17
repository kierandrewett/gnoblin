//! The verbs and signals of the compositor's `gnoblin` namespace, called from a native shell in a real session
//! (`design/modular-framework.md`, section 5.9 and 32).
//!
//! The session is the shipped binary on a headless backend with the fixture shell `shell-verbs` loaded in its process. The shell
//! calls `idle.inhibit` and gets the compositor's answer, is refused for a verb on an object that does not exist, is refused before
//! anything goes out for a wrong argument list, and subscribes to the session's `bell`. What each assertion reads is what the
//! compositor holds (the `idle` object's hold count, read through a fresh owner connection) and what the shell printed, so the call
//! really crossed the owner socket and the signal really came back over it.

use std::path::Path;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_model::{Filter, TypeKey, Value};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::system_bell::v1::client::xdg_system_bell_v1::XdgSystemBellV1;

mod support;
use support::native_shell::*;

/// How many holds keep the session awake, as the compositor's `idle` object publishes it: read through a fresh owner connection.
fn inhibitors(ipc: &Path) -> u64 {
    let transport = UnixTransport::connect(ipc).expect("the session is listening");
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    let mut client =
        Client::connect(transport, gnoblin_types::Version::new(major, minor, patch)).expect("the handshake");
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    let view = snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .expect("the compositor publishes an idle object");
    match view.property("inhibitors") {
        Some(Value::Uint(count)) => *count,
        other => panic!("the idle object's inhibitors is {other:?}"),
    }
}

struct Ringing;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Ringing {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Ringing: ignore XdgSystemBellV1);

/// A Wayland client that rings the system bell (`xdg_system_bell_v1`): what an application does when it wants the user's attention.
struct Ringer {
    _connection: Connection,
    queue: EventQueue<Ringing>,
    bell: XdgSystemBellV1,
}

impl Ringer {
    fn connect(socket: &Path) -> Self {
        let stream = std::os::unix::net::UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<Ringing>(&connection).expect("the global list");
        let bell: XdgSystemBellV1 = globals
            .bind(&queue.handle(), 1..=1, ())
            .expect("the session advertises xdg_system_bell_v1");
        Self {
            _connection: connection,
            queue,
            bell,
        }
    }

    fn ring(&mut self) {
        self.bell.ring(None);
        self.queue.roundtrip(&mut Ringing).expect("the ring is delivered");
    }
}

#[test]
fn a_native_shell_calls_a_verb_and_hears_the_answer_and_a_signal_over_the_owner_socket() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().verbs, &[], &[]);
    session.wait_serving();
    session.wait_log("the shell is ready", |line| line.contains("[shell-verbs] ready"));

    // The verb crossed the owner socket: the compositor holds the session awake under the shell's holder name, and the shell was
    // told it was done (a bare `done` has no value).
    session.wait_log("the inhibit is answered", |line| {
        line.contains("[shell-verbs] inhibit answered ok result=\"\"")
    });
    let ipc = session.ipc();
    session.wait_for("the compositor counts the shell's hold", |_| inhibitors(&ipc) == 1);

    // A verb on an object the compositor does not have is refused with the compositor's code, never a hang; a wrong argument list
    // never leaves the shell.
    session.wait_log("the missing object is refused", |line| {
        line.contains("[shell-verbs] logout on a missing object answered refused code=")
    });
    session.wait_log("the wrong arguments are refused before they are sent", |line| {
        line.contains("[shell-verbs] inhibit with no holder refused:")
    });
    assert_eq!(
        inhibitors(&ipc),
        1,
        "the refused calls made no hold: {}",
        session.log_text()
    );

    // The signal: an application rings the bell, the compositor counts it, and the shell hears it with the count as its payload.
    let mut ringer = Ringer::connect(&session.wayland());
    ringer.ring();
    session.wait_log("the shell hears the first bell", |line| {
        line.contains("[shell-verbs] bell count=1 ")
    });
    ringer.ring();
    session.wait_log("and the second, with the count the compositor kept", |line| {
        line.contains("[shell-verbs] bell count=2 ")
    });
    assert!(
        session.log().iter().all(|line| !line.text.contains("wrong shape")),
        "{}",
        session.log_text()
    );
    assert!(session.exited().is_none(), "{}", session.log_text());
}

/// What a script shell of either language does with the generated verbs: the answer of `idle.inhibit` reaches its function, a verb on
/// an object that does not exist is refused, a wrong argument never leaves the script, and the bell reaches the subscription with
/// the count. `prefix` is what each line the script logs starts with.
fn a_script_shell_calls_the_generated_verbs(shell: &Path, prefix: &str, answered: &str, refused: &str, wrong: &str) {
    let mut session = start_script(Start::Compositor, shell, &[]);
    session.wait_serving();
    session.wait_log("the script is ready", |line| line.contains(&format!("{prefix} ready")));
    session.wait_log("the inhibit is answered with the compositor's done", |line| {
        line.contains(answered)
    });
    let ipc = session.ipc();
    session.wait_for("the compositor counts the script's hold", |_| inhibitors(&ipc) == 1);
    session.wait_log("the missing object is refused with a code", |line| {
        line.contains(refused)
    });
    session.wait_log("the wrong argument never left the script", |line| line.contains(wrong));
    assert_eq!(inhibitors(&ipc), 1, "{}", session.log_text());

    let mut ringer = Ringer::connect(&session.wayland());
    ringer.ring();
    session.wait_log("the script hears the first bell", |line| {
        line.contains(&format!("{prefix} bell count=1"))
    });
    ringer.ring();
    session.wait_log("and the second", |line| {
        line.contains(&format!("{prefix} bell count=2"))
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn a_python_shell_calls_the_generated_verbs_and_hears_the_bell() {
    let directory = tempfile::tempdir().expect("a directory for the shell");
    let shell = directory.path().join("shell.py");
    std::fs::write(
        &shell,
        "from gnoblin import gnoblin, verbs\n\n\n\
         def answered(result, error):\n    gnoblin.log(\"python inhibit answered result=%r error=%r\" % (bytes(result), error), \"info\")\n\n\n\
         def refused(result, error):\n    gnoblin.log(\"python logout on a missing object refused error=%s\" % (error != None), \"info\")\n\n\n\
         def rang(count):\n    gnoblin.log(\"python bell count=%d\" % count, \"info\")\n\n\n\
         verbs.idle.inhibit(\"python-shell\", on_answer=answered)\n\
         verbs.session.logout(on_answer=refused, object_id=2147483647)\n\
         verbs.session.on_bell(rang)\n\
         try:\n    verbs.idle.inhibit()\nexcept TypeError:\n    gnoblin.log(\"python inhibit with no holder raised TypeError\", \"info\")\n\
         gnoblin.log(\"python ready\", \"info\")\n",
    )
    .expect("the shell");
    a_script_shell_calls_the_generated_verbs(
        &shell,
        "python",
        "python inhibit answered result=b'' error=None",
        "python logout on a missing object refused error=True",
        "python inhibit with no holder raised TypeError",
    );
}

#[test]
fn a_typescript_shell_calls_the_generated_verbs_and_hears_the_bell() {
    let directory = tempfile::tempdir().expect("a directory for the shell");
    let shell = directory.path().join("shell.ts");
    std::fs::write(
        &shell,
        "import { verbs } from \"gnoblin-core/generated/verbs\";\n\n\
         verbs.idle.inhibit(\"ts-shell\", (answer) => console.log(`ts inhibit answered value=${answer.value} error=${answer.error}`));\n\
         verbs.session.logout((answer) => console.log(`ts logout on a missing object refused error=${answer.error !== null}`), 2147483647);\n\
         verbs.session.onBell((count) => console.log(`ts bell count=${count}`));\n\
         try {\n    verbs.idle.inhibit(42 as unknown as string);\n} catch (error) {\n    console.log(`ts inhibit with a number raised ${(error as Error).name}`);\n}\n\
         console.log(\"ts ready\");\n",
    )
    .expect("the shell");
    a_script_shell_calls_the_generated_verbs(
        &shell,
        "ts",
        "ts inhibit answered value=undefined error=null",
        "ts logout on a missing object refused error=true",
        "ts inhibit with a number raised TypeError",
    );
}

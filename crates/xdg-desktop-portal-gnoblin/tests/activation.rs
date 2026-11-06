//! The built backend, started on a private bus with no compositor IPC.
//!
//! This is the process a bus activation starts: the binary, the session bus it is given, and
//! no inherited descriptor and no reachable socket. The test checks what a caller can rely
//! on: the name is claimed, `Settings` answers, a capture is refused with a reason rather
//! than a crash, and the process ends when the bus does.

mod support;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use support::bus::Bus;
use zbus::connection::Builder;
use zbus::{Connection, ObjectPath, OwnedVariant, Proxy, Variant, fdo};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.gnoblin";
const PATH: &str = "/org/freedesktop/portal/desktop";
const DEADLINE: Duration = Duration::from_secs(15);

/// The backend, spawned against one bus, killed on drop.
struct Backend {
    child: Child,
}

impl Backend {
    /// Start the binary with the session bus given and no compositor IPC reachable.
    fn start(bus: &Bus, directory: &Path) -> Backend {
        let child = Command::new(env!("CARGO_BIN_EXE_xdg-desktop-portal-gnoblin"))
            .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
            .env_remove("GNOBLIN_IPC_FD")
            // A path that does not exist: the connect fails, and the backend degrades.
            .env("GNOBLIN_IPC_SOCKET", directory.join("no-compositor.sock"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the backend binary starts");
        Backend { child }
    }

    /// The exit status, if the process has exited.
    fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().expect("the child can be polled")
    }

    /// Block until the process exits, or panic at the deadline.
    fn wait_for_exit(&mut self) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = self.exited() {
                return status;
            }
            assert!(start.elapsed() < DEADLINE, "the backend outlived its bus");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A fresh directory under the target's temporary space, for the socket path.
fn scratch(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("the scratch directory is created");
    directory
}

/// A caller connection on the private bus.
async fn caller(bus: &Bus) -> Connection {
    Builder::address(bus.address())
        .build()
        .await
        .expect("a caller connects to the private bus")
}

/// Wait until something owns the backend's name, or panic at the deadline.
async fn wait_for_name(connection: &Connection, backend: &mut Backend) {
    let dbus = fdo::DBusProxy::new(connection).await.expect("the bus proxy");
    let start = Instant::now();
    loop {
        if dbus
            .name_has_owner(BUS_NAME.try_into().expect("the name is valid"))
            .await
            .expect("the name query answers")
        {
            return;
        }
        if let Some(status) = backend.exited() {
            panic!("the backend exited before claiming its name: {status}");
        }
        assert!(start.elapsed() < DEADLINE, "the backend never claimed {BUS_NAME}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn starts_without_compositor_ipc_and_serves_settings() {
    let bus = Bus::start();
    let directory = scratch("activation-settings");
    let mut backend = Backend::start(&bus, &directory);
    zbus::block_on(async {
        let connection = caller(&bus).await;
        wait_for_name(&connection, &mut backend).await;

        let settings = Proxy::new(&connection, BUS_NAME, PATH, "org.freedesktop.impl.portal.Settings")
            .await
            .expect("the settings proxy");
        // An empty filter asks for every namespace. The call answers with no compositor
        // behind it: the table is empty, and an empty table is a valid answer.
        let namespaces: Vec<String> = Vec::new();
        let _all: BTreeMap<String, BTreeMap<String, OwnedVariant>> = settings
            .call("ReadAll", &(namespaces,))
            .await
            .expect("ReadAll answers with no compositor behind it");
    });

    assert!(
        backend.exited().is_none(),
        "the backend is still serving after a settings read"
    );
}

#[test]
fn refuses_capture_without_a_shell_and_keeps_serving() {
    let bus = Bus::start();
    let directory = scratch("activation-refuse");
    let mut backend = Backend::start(&bus, &directory);
    let text = zbus::block_on(async {
        let connection = caller(&bus).await;
        wait_for_name(&connection, &mut backend).await;

        let screenshot = Proxy::new(&connection, BUS_NAME, PATH, "org.freedesktop.impl.portal.Screenshot")
            .await
            .expect("the screenshot proxy");
        let handle = ObjectPath::try_from("/org/freedesktop/portal/desktop/request/activation/shot")
            .expect("the handle is a valid object path");
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let refusal = screenshot
            .call_method("Screenshot", &(handle, "", "", options))
            .await
            .expect_err("a capture with no shell is refused");
        refusal.to_string()
    });

    assert!(
        text.contains("portal.ui"),
        "the refusal names the missing slot, got: {text}"
    );
    assert!(
        backend.exited().is_none(),
        "the backend is still serving after a refused capture"
    );
}

#[test]
fn exits_when_the_bus_goes_away() {
    let bus = Bus::start();
    let directory = scratch("activation-exit");
    let mut backend = Backend::start(&bus, &directory);
    zbus::block_on(async {
        let connection = caller(&bus).await;
        wait_for_name(&connection, &mut backend).await;
    });

    drop(bus);
    let status = backend.wait_for_exit();
    assert!(status.success(), "a bus loss is a clean exit, got {status}");
}

//! org.freedesktop.Notifications server on a background zbus thread. Notify/
//! Close calls are channelled to the Slint render thread; close/action signals
//! are emitted back when the UI dismisses or a notification expires.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use zbus::zvariant::OwnedValue;

const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";

/// A notification pushed from the bus to the UI thread.
#[derive(Clone)]
pub struct Incoming {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub expire_ms: i32, // -1 = server default, 0 = never
    pub urgency: u8,    // 0 = low, 1 = normal, 2 = critical
}

/// Events from the bus to the UI thread.
pub enum NotifyEvent {
    Show(Incoming),
    Close(u32),
}

/// Commands from the UI thread back to the bus (emit signals).
pub enum NotifyCommand {
    Closed { id: u32, reason: u32 },
    Action { id: u32, action: String },
}

struct Server {
    tx: Sender<NotifyEvent>,
    next_id: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };
        let urgency = hints
            .get("urgency")
            .and_then(|v| u8::try_from(v.try_clone().ok()?).ok())
            .unwrap_or(1);
        let _ = self.tx.send(NotifyEvent::Show(Incoming {
            id,
            app_name,
            app_icon,
            summary,
            body,
            actions,
            expire_ms: expire_timeout,
            urgency,
        }));
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(NotifyEvent::Close(id));
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into(), "icon-static".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "gnoblin-notifyd".into(),
            "gnoblin".into(),
            "0.1".into(),
            "1.2".into(),
        )
    }
}

/// Spawn the daemon: `events` carries Notify/Close to the UI; `commands` carries
/// dismiss/action results back so the server emits the spec signals.
pub fn spawn(events: Sender<NotifyEvent>, commands: Receiver<NotifyCommand>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(run(events, commands)) {
            eprintln!("gnoblin-notifyd: server stopped: {e}");
        }
    });
}

async fn run(events: Sender<NotifyEvent>, commands: Receiver<NotifyCommand>) -> zbus::Result<()> {
    let server = Server {
        tx: events,
        next_id: Arc::new(AtomicU32::new(1)),
    };
    let conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at(PATH, server)?
        .build()
        .await?;

    loop {
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                NotifyCommand::Closed { id, reason } => {
                    let _ = conn
                        .emit_signal(None::<()>, PATH, IFACE, "NotificationClosed", &(id, reason))
                        .await;
                }
                NotifyCommand::Action { id, action } => {
                    let _ = conn
                        .emit_signal(None::<()>, PATH, IFACE, "ActionInvoked", &(id, action))
                        .await;
                }
            }
        }
        async_io::Timer::after(Duration::from_millis(80)).await;
    }
}

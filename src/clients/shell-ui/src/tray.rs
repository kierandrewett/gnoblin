//! Minimal StatusNotifierItem (SNI) tray host for the Slint topbar.
//!
//! Owns `org.kde.StatusNotifierWatcher` and acts as a host: apps register their
//! tray items with us, we fetch each item's icon/title, and channel the list to
//! the main (Slint) thread. zbus runs on a background thread; the bar's sync
//! calloop loop drains the channel each tick and forwards clicks back.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use zbus::object_server::SignalEmitter;

/// One tray item, as the main thread needs it.
#[derive(Clone, PartialEq)]
pub struct TrayItem {
    pub id: i32,
    pub title: String,
    pub icon_name: String,
    pub icon_theme_path: String,
    pub service: String,
    pub object_path: String,
}

/// A click action from the main thread to dispatch on an item.
pub enum TrayCommand {
    Activate { service: String, path: String },
    ContextMenu { service: String, path: String },
}

type ItemList = Arc<Mutex<Vec<(String, String)>>>; // (service, object_path)

fn lock_items(items: &ItemList) -> MutexGuard<'_, Vec<(String, String)>> {
    items.lock().unwrap_or_else(|poisoned| {
        eprintln!("gnoblin-topbar: tray item list lock was poisoned; recovering");
        poisoned.into_inner()
    })
}

struct Watcher {
    items: ItemList,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) {
        // `service` is either a bus name (path defaults to /StatusNotifierItem)
        // or an object path (the sender is the bus name).
        let (svc, path) = if service.starts_with('/') {
            (
                hdr.sender().map(|s| s.to_string()).unwrap_or_default(),
                service.to_string(),
            )
        } else {
            (service.to_string(), "/StatusNotifierItem".to_string())
        };
        let mut items = lock_items(&self.items);
        if !items.iter().any(|(s, p)| s == &svc && p == &path) {
            items.push((svc, path));
        }
    }

    async fn register_status_notifier_host(&mut self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        lock_items(&self.items)
            .iter()
            .map(|(s, p)| format!("{s}{p}"))
            .collect()
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
trait Item {
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
}

/// Spawn the SNI host on a background thread. `updates` receives the item list
/// whenever it changes; `commands` carries click actions back to dispatch.
pub fn spawn(updates: Sender<Vec<TrayItem>>, commands: Receiver<TrayCommand>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(run(updates, commands)) {
            eprintln!("gnoblin-topbar: tray host stopped: {e}");
        }
    });
}

async fn run(updates: Sender<Vec<TrayItem>>, commands: Receiver<TrayCommand>) -> zbus::Result<()> {
    let items: ItemList = Arc::new(Mutex::new(Vec::new()));
    let conn = zbus::connection::Builder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at(
            "/StatusNotifierWatcher",
            Watcher {
                items: items.clone(),
            },
        )?
        .build()
        .await?;

    let mut last: Vec<TrayItem> = Vec::new();
    let mut next_id = 0i32;
    let mut ids: HashMap<(String, String), i32> = HashMap::new();

    loop {
        // Dispatch any click commands from the main thread.
        while let Ok(cmd) = commands.try_recv() {
            let (svc, path, ctx) = match cmd {
                TrayCommand::Activate { service, path } => (service, path, false),
                TrayCommand::ContextMenu { service, path } => (service, path, true),
            };
            if let Ok(proxy) = item_proxy(&conn, &svc, &path).await {
                if ctx {
                    let _ = proxy.context_menu(0, 0).await;
                } else {
                    let _ = proxy.activate(0, 0).await;
                }
            }
        }

        // Snapshot + refresh each item's icon/title.
        let snapshot = lock_items(&items).clone();
        let mut out = Vec::new();
        for (svc, path) in snapshot {
            let proxy = match item_proxy(&conn, &svc, &path).await {
                Ok(p) => p,
                Err(_) => continue,
            };
            let id = *ids.entry((svc.clone(), path.clone())).or_insert_with(|| {
                next_id += 1;
                next_id
            });
            out.push(TrayItem {
                id,
                title: proxy.title().await.unwrap_or_default(),
                icon_name: proxy.icon_name().await.unwrap_or_default(),
                icon_theme_path: proxy.icon_theme_path().await.unwrap_or_default(),
                service: svc,
                object_path: path,
            });
        }
        if out != last {
            let _ = updates.send(out.clone());
            last = out;
        }

        async_io::Timer::after(Duration::from_millis(1500)).await;
    }
}

async fn item_proxy<'a>(
    conn: &zbus::Connection,
    service: &str,
    path: &str,
) -> zbus::Result<ItemProxy<'a>> {
    ItemProxy::builder(conn)
        .destination(service.to_string())?
        .path(path.to_string())?
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_item_list_recovers_after_poisoned_lock() {
        let items: ItemList = Arc::new(Mutex::new(Vec::new()));
        let poison = items.clone();
        let _ = std::thread::spawn(move || {
            let mut guard = poison.lock().unwrap();
            guard.push((
                "org.example.App".to_string(),
                "/StatusNotifierItem".to_string(),
            ));
            panic!("poison tray item list");
        })
        .join();

        let guard = lock_items(&items);
        assert_eq!(
            guard.as_slice(),
            &[(
                "org.example.App".to_string(),
                "/StatusNotifierItem".to_string()
            )]
        );
    }
}

//! A read-once snapshot of quick-settings state for the control-centre popout:
//! volume/mic (wpctl), network (nmcli), bluetooth (BlueZ). Best-effort — any
//! unavailable subsystem just reports a sensible default.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::Sender;

use zbus::zvariant::OwnedValue;

#[derive(Default, Clone, PartialEq)]
pub struct QuickState {
    pub volume: f32,
    pub muted: bool,
    pub mic: f32,
    pub wired: bool,
    pub wifi: bool,
    pub wifi_name: String,
    pub bt: bool,
    pub bt_device: String,
    pub power_mode: String, // "Balanced" / "Performance" / "Power Saver"
}

pub fn read() -> QuickState {
    let (volume, muted) = wpctl_volume("@DEFAULT_AUDIO_SINK@").unwrap_or((0.0, false));
    let mic = wpctl_volume("@DEFAULT_AUDIO_SOURCE@")
        .map(|(v, _)| v)
        .unwrap_or(0.0);
    let (wired, wifi, wifi_name) = network();
    let (bt, bt_device) = zbus::block_on(bluetooth());
    let power_mode = zbus::block_on(power_profile())
        .map(|p| prettify_profile(&p))
        .unwrap_or_else(|| "Balanced".to_string());
    QuickState {
        volume,
        muted,
        mic,
        wired,
        wifi,
        wifi_name,
        bt,
        bt_device,
        power_mode,
    }
}

/// Poll quick-settings state on a background thread (network/audio/bluetooth
/// reads are too heavy for the render tick) and send a fresh `QuickState` on
/// every change. The topbar drains these in its tick to keep the status cluster
/// (network glyph, mute) and the control-centre popout live.
pub fn spawn(tx: Sender<QuickState>) {
    std::thread::spawn(move || {
        let mut last: Option<QuickState> = None;
        loop {
            let st = read();
            if last.as_ref() != Some(&st) {
                if tx.send(st.clone()).is_err() {
                    break; // receiver gone
                }
                last = Some(st);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

const PPD_SERVICE: &str = "net.hadess.PowerProfiles";
const PPD_PATH: &str = "/net/hadess/PowerProfiles";

fn prettify_profile(p: &str) -> String {
    match p {
        "power-saver" => "Power Saver".to_string(),
        "performance" => "Performance".to_string(),
        "balanced" => "Balanced".to_string(),
        other => other.to_string(),
    }
}

async fn power_profile() -> Option<String> {
    let conn = zbus::Connection::system().await.ok()?;
    let proxy = zbus::Proxy::new(&conn, PPD_SERVICE, PPD_PATH, PPD_SERVICE)
        .await
        .ok()?;
    proxy.get_property::<String>("ActiveProfile").await.ok()
}

/// Cycle the active power profile (power-saver → balanced → performance → …).
pub fn cycle_power_profile() {
    let _ = zbus::block_on(async {
        let conn = zbus::Connection::system().await?;
        let proxy = zbus::Proxy::new(&conn, PPD_SERVICE, PPD_PATH, PPD_SERVICE).await?;
        let current = proxy
            .get_property::<String>("ActiveProfile")
            .await
            .unwrap_or_default();
        let next = match current.as_str() {
            "power-saver" => "balanced",
            "balanced" => "performance",
            _ => "power-saver",
        };
        proxy.set_property("ActiveProfile", next).await
    });
}

/// Parse `wpctl get-volume` → (level 0..1, muted). Output is
/// `Volume: 0.50` or `Volume: 0.50 [MUTED]`.
fn wpctl_volume(target: &str) -> Option<(f32, bool)> {
    let out = Command::new("wpctl")
        .args(["get-volume", target])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let level = text.split_whitespace().nth(1)?.parse::<f32>().ok()?;
    Some((level, text.contains("MUTED")))
}

/// (wired_connected, wifi_connected, wifi_connection_name) via nmcli.
fn network() -> (bool, bool, String) {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
        .output()
    else {
        return (false, false, String::new());
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut wired, mut wifi, mut name) = (false, false, String::new());
    for line in text.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 3 || !f[1].starts_with("connected") {
            continue;
        }
        match f[0] {
            "ethernet" => wired = true,
            "wifi" => {
                wifi = true;
                name = f[2].to_string();
            }
            _ => {}
        }
    }
    (wired, wifi, name)
}

type Managed =
    HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// (adapter_powered, connected_device_name) via BlueZ on the system bus.
async fn bluetooth() -> (bool, String) {
    let Ok(conn) = zbus::Connection::system().await else {
        return (false, String::new());
    };
    let Ok(proxy) = zbus::Proxy::new(
        &conn,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    else {
        return (false, String::new());
    };
    let Ok(objs) = proxy.call::<_, _, Managed>("GetManagedObjects", &()).await else {
        return (false, String::new());
    };

    let prop_bool = |m: &HashMap<String, OwnedValue>, k: &str| {
        m.get(k)
            .and_then(|v| bool::try_from(v.try_clone().ok()?).ok())
            .unwrap_or(false)
    };
    let (mut powered, mut device) = (false, String::new());
    for ifaces in objs.values() {
        if let Some(ad) = ifaces.get("org.bluez.Adapter1") {
            if prop_bool(ad, "Powered") {
                powered = true;
            }
        }
        if let Some(dev) = ifaces.get("org.bluez.Device1") {
            if prop_bool(dev, "Connected") {
                if let Some(alias) = dev
                    .get("Alias")
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                {
                    device = alias;
                }
            }
        }
    }
    (powered, device)
}

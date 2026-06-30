//! org.gtk.Menus / org.gtk.Actions and com.canonical.dbusmenu clients for the
//! KDE-style global menu.
//!
//! mutter exposes the focused window's menu export via
//! `dev.gnoblin.Shell.GetActiveWindowMenu`. GTK apps use `org.gtk.Menus`;
//! KDE/Qt apps use the Plasma appmenu Wayland protocol to point us at a
//! `com.canonical.dbusmenu` object. All async work runs on a background zbus
//! thread (same pattern as the tray / shell poll).

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

/// Where the focused window's menu lives on the bus.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct MenuAddr {
    /// `gtk` for org.gtk.Menus / org.gtk.Actions, `dbusmenu` for
    /// com.canonical.dbusmenu.
    pub kind: String,
    pub bus: String,
    pub app_path: String,
    pub win_path: String,
    pub menubar_path: String,
}

impl MenuAddr {
    pub fn is_empty(&self) -> bool {
        self.bus.is_empty() || self.menubar_path.is_empty()
    }

    fn is_dbusmenu(&self) -> bool {
        self.kind == "dbusmenu" || self.kind == "kde"
    }
}

/// One top-level bar entry (File / Edit / …). `group`/`menu` address its
/// submenu in the org.gtk.Menus model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarEntry {
    pub label: String,
    pub group: u32,
    pub menu: u32,
}

/// One row inside a dropdown.
#[derive(Clone, Debug, Default)]
pub struct MenuRow {
    pub label: String,
    /// Full action name ("app.quit"); empty for separators/submenu headers.
    pub action: String,
    pub enabled: bool,
    pub separator: bool,
    pub has_submenu: bool,
    pub group: u32,
    pub menu: u32,
}

/// A request from the UI thread to the appmenu worker.
pub enum MenuCommand {
    /// Fetch the dropdown rows for a top-level entry's (group, menu).
    OpenSubmenu {
        addr: MenuAddr,
        group: u32,
        menu: u32,
    },
    /// Activate an action ("app.quit" / "win.foo") on the menu's action groups.
    Activate { addr: MenuAddr, action: String },
}

/// A reply from the worker to the UI thread.
pub enum MenuReply {
    /// Dropdown rows for the (group, menu) the UI asked to open.
    Submenu {
        group: u32,
        menu: u32,
        rows: Vec<MenuRow>,
    },
}

fn as_str(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn as_uu(v: &OwnedValue) -> Option<(u32, u32)> {
    if let Value::Structure(s) = &**v {
        if let [Value::U32(g), Value::U32(m)] = s.fields() {
            return Some((*g, *m));
        }
    }
    None
}

type GtkMenuGroup = (u32, u32, Vec<HashMap<String, OwnedValue>>);

/// Call `org.gtk.Menus.Start(groups)` → the requested menu groups.
async fn gtk_start(conn: &Connection, addr: &MenuAddr, groups: &[u32]) -> Vec<GtkMenuGroup> {
    let proxy = match zbus::Proxy::new(
        conn,
        addr.bus.clone(),
        addr.menubar_path.clone(),
        "org.gtk.Menus",
    )
    .await
    {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    proxy.call("Start", &(groups,)).await.unwrap_or_default()
}

/// dbusmenu labels carry `_`/`__` mnemonic markers — strip a single `_`.
fn clean_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            // `__` is a literal underscore; a lone `_` is a mnemonic marker.
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse one org.gtk.Menus item dict into a `MenuRow` (None to drop it).
/// `:section` items are flattened by the caller; this handles leaf actions
/// and submenu headers.
fn parse_row(item: &HashMap<String, OwnedValue>) -> Option<MenuRow> {
    let label = item.get("label").and_then(as_str).map(|l| clean_label(&l));
    let action = item.get("action").and_then(as_str);
    let submenu = item.get(":submenu").and_then(as_uu);

    match (label, action, submenu) {
        (Some(label), action, submenu) => Some(MenuRow {
            label,
            action: action.unwrap_or_default(),
            enabled: true,
            separator: false,
            has_submenu: submenu.is_some(),
            group: submenu.map(|(g, _)| g).unwrap_or(0),
            menu: submenu.map(|(_, m)| m).unwrap_or(0),
        }),
        _ => None,
    }
}

/// Fetch the top-level menu bar (File / Edit / …) for `addr`. The bar lives at
/// group 0 / menu 0; each item references its submenu's (group, menu).
async fn fetch_gtk_bar(conn: &Connection, addr: &MenuAddr) -> Vec<BarEntry> {
    let groups = gtk_start(conn, addr, &[0]).await;
    let mut out = Vec::new();
    for (g, m, items) in &groups {
        if *g != 0 || *m != 0 {
            continue;
        }
        for item in items {
            let Some(label) = item.get("label").and_then(as_str) else {
                continue;
            };
            let Some((sg, sm)) = item.get(":submenu").and_then(as_uu) else {
                continue;
            };
            out.push(BarEntry {
                label: clean_label(&label),
                group: sg,
                menu: sm,
            });
        }
    }
    out
}

/// Fetch the rows of one dropdown (group, menu). `:section` entries are
/// flattened in place with a separator between sections.
async fn fetch_gtk_submenu(
    conn: &Connection,
    addr: &MenuAddr,
    group: u32,
    menu: u32,
) -> Vec<MenuRow> {
    let groups = gtk_start(conn, addr, &[group]).await;
    let lookup = |g: u32, m: u32| -> Vec<HashMap<String, OwnedValue>> {
        groups
            .iter()
            .find(|(gg, mm, _)| *gg == g && *mm == m)
            .map(|(_, _, items)| items.clone())
            .unwrap_or_default()
    };

    let mut rows = Vec::new();
    let mut first_section = true;
    let items = lookup(group, menu);
    for item in &items {
        if let Some((sg, sm)) = item.get(":section").and_then(as_uu) {
            if !first_section && !rows.is_empty() {
                rows.push(MenuRow {
                    separator: true,
                    ..Default::default()
                });
            }
            first_section = false;
            // Section contents may live in another (group, menu).
            let section_items = if sg == group {
                lookup(sg, sm)
            } else {
                fetch_gtk_section(conn, addr, sg, sm).await
            };
            for si in &section_items {
                if let Some(row) = parse_row(si) {
                    rows.push(row);
                }
            }
            continue;
        }
        if let Some(row) = parse_row(item) {
            rows.push(row);
        }
    }
    rows
}

/// Fetch a section that lives in a different group than its parent menu.
async fn fetch_gtk_section(
    conn: &Connection,
    addr: &MenuAddr,
    group: u32,
    menu: u32,
) -> Vec<HashMap<String, OwnedValue>> {
    let groups = gtk_start(conn, addr, &[group]).await;
    groups
        .into_iter()
        .find(|(g, m, _)| *g == group && *m == menu)
        .map(|(_, _, items)| items)
        .unwrap_or_default()
}

/// Activate `action` ("app.quit" / "win.foo"). The prefix selects the action
/// group: `app` → the application object path, `win` → the window object path.
async fn activate_gtk(conn: &Connection, addr: &MenuAddr, action: &str) {
    let (prefix, name) = match action.split_once('.') {
        Some((p, n)) => (p, n),
        None => ("app", action),
    };
    let path = match prefix {
        "win" if !addr.win_path.is_empty() => addr.win_path.clone(),
        _ => addr.app_path.clone(),
    };
    if path.is_empty() {
        return;
    }
    let proxy = match zbus::Proxy::new(conn, addr.bus.clone(), path, "org.gtk.Actions").await {
        Ok(p) => p,
        Err(_) => return,
    };
    let target: Vec<Value<'_>> = Vec::new();
    let platform: HashMap<String, Value<'_>> = HashMap::new();
    let _ = proxy
        .call::<_, _, ()>("Activate", &(name, target, platform))
        .await;
}

type DBusMenuNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn prop_str(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    props.get(key).and_then(as_str)
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    match &**props.get(key)? {
        Value::Bool(v) => Some(*v),
        _ => None,
    }
}

async fn dbusmenu_get_layout(
    conn: &Connection,
    addr: &MenuAddr,
    parent_id: i32,
    depth: i32,
) -> Option<DBusMenuNode> {
    let proxy = zbus::Proxy::new(
        conn,
        addr.bus.clone(),
        addr.menubar_path.clone(),
        "com.canonical.dbusmenu",
    )
    .await
    .ok()?;
    let properties = vec!["label", "enabled", "visible", "type", "children-display"];
    let (_revision, layout): (u32, DBusMenuNode) = proxy
        .call("GetLayout", &(parent_id, depth, properties))
        .await
        .ok()?;
    Some(layout)
}

async fn dbusmenu_about_to_show(conn: &Connection, addr: &MenuAddr, id: i32) {
    let Ok(proxy) = zbus::Proxy::new(
        conn,
        addr.bus.clone(),
        addr.menubar_path.clone(),
        "com.canonical.dbusmenu",
    )
    .await
    else {
        return;
    };
    let _: zbus::Result<bool> = proxy.call("AboutToShow", &(id,)).await;
}

fn dbusmenu_child(v: &OwnedValue) -> Option<DBusMenuNode> {
    v.clone().try_into().ok()
}

fn dbusmenu_bar_entry(node: &DBusMenuNode) -> Option<BarEntry> {
    let (id, props, children) = node;
    if *id < 0 || !prop_bool(props, "visible").unwrap_or(true) || children.is_empty() {
        return None;
    }
    if prop_str(props, "type").as_deref() == Some("separator") {
        return None;
    }
    let label = clean_label(&prop_str(props, "label")?);
    if label.is_empty() {
        return None;
    }
    Some(BarEntry {
        label,
        group: 0,
        menu: *id as u32,
    })
}

fn dbusmenu_row(node: &DBusMenuNode) -> Option<MenuRow> {
    let (id, props, children) = node;
    if !prop_bool(props, "visible").unwrap_or(true) {
        return None;
    }
    let separator = prop_str(props, "type").as_deref() == Some("separator");
    if separator {
        return Some(MenuRow {
            separator: true,
            enabled: false,
            ..Default::default()
        });
    }
    if *id < 0 {
        return None;
    }
    let label = clean_label(&prop_str(props, "label").unwrap_or_default());
    if label.is_empty() {
        return None;
    }
    let has_submenu =
        prop_str(props, "children-display").as_deref() == Some("submenu") || !children.is_empty();
    Some(MenuRow {
        label,
        action: if has_submenu {
            String::new()
        } else {
            format!("dbusmenu:{id}")
        },
        enabled: prop_bool(props, "enabled").unwrap_or(true),
        separator: false,
        has_submenu,
        group: 0,
        menu: *id as u32,
    })
}

async fn fetch_dbusmenu_bar(conn: &Connection, addr: &MenuAddr) -> Vec<BarEntry> {
    let Some((_id, _props, children)) = dbusmenu_get_layout(conn, addr, 0, 1).await else {
        return Vec::new();
    };
    children
        .iter()
        .filter_map(dbusmenu_child)
        .filter_map(|node| dbusmenu_bar_entry(&node))
        .collect()
}

async fn fetch_dbusmenu_submenu(conn: &Connection, addr: &MenuAddr, menu: u32) -> Vec<MenuRow> {
    let id = menu as i32;
    dbusmenu_about_to_show(conn, addr, id).await;
    let Some((_id, _props, children)) = dbusmenu_get_layout(conn, addr, id, 1).await else {
        return Vec::new();
    };
    children
        .iter()
        .filter_map(dbusmenu_child)
        .filter_map(|node| dbusmenu_row(&node))
        .collect()
}

async fn activate_dbusmenu(conn: &Connection, addr: &MenuAddr, action: &str) {
    let Some(id) = action
        .strip_prefix("dbusmenu:")
        .and_then(|id| id.parse::<i32>().ok())
    else {
        return;
    };
    let Ok(proxy) = zbus::Proxy::new(
        conn,
        addr.bus.clone(),
        addr.menubar_path.clone(),
        "com.canonical.dbusmenu",
    )
    .await
    else {
        return;
    };
    let data = Value::from(0i32);
    let _ = proxy
        .call::<_, _, ()>("Event", &(id, "clicked", data, 0u32))
        .await;
}

/// Fetch the top-level menu bar (File / Edit / …) for `addr`.
pub async fn fetch_bar(conn: &Connection, addr: &MenuAddr) -> Vec<BarEntry> {
    if addr.is_dbusmenu() {
        fetch_dbusmenu_bar(conn, addr).await
    } else {
        fetch_gtk_bar(conn, addr).await
    }
}

/// Fetch the rows of one dropdown.
pub async fn fetch_submenu(
    conn: &Connection,
    addr: &MenuAddr,
    group: u32,
    menu: u32,
) -> Vec<MenuRow> {
    if addr.is_dbusmenu() {
        fetch_dbusmenu_submenu(conn, addr, menu).await
    } else {
        fetch_gtk_submenu(conn, addr, group, menu).await
    }
}

pub async fn activate(conn: &Connection, addr: &MenuAddr, action: &str) {
    if addr.is_dbusmenu() {
        activate_dbusmenu(conn, addr, action).await;
    } else {
        activate_gtk(conn, addr, action).await;
    }
}

/// Spawn the appmenu worker: handles submenu fetches + activations on a
/// background zbus thread, replying over `replies`.
pub fn spawn(commands: Receiver<MenuCommand>, replies: Sender<MenuReply>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(worker(commands, replies)) {
            eprintln!("gnoblin-topbar: appmenu worker stopped: {e}");
        }
    });
}

async fn worker(commands: Receiver<MenuCommand>, replies: Sender<MenuReply>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    loop {
        match commands.recv() {
            Ok(MenuCommand::OpenSubmenu { addr, group, menu }) => {
                let rows = fetch_submenu(&conn, &addr, group, menu).await;
                let _ = replies.send(MenuReply::Submenu { group, menu, rows });
            }
            Ok(MenuCommand::Activate { addr, action }) => {
                activate(&conn, &addr, &action).await;
            }
            Err(_) => return Ok(()), // UI thread gone
        }
    }
}

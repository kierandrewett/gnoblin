//! `xr-smoke-tree`: prints the node tree the owner socket shows, one node per line, for the XR smoke script.
//!
//! It attaches as any tree client does, reads for `--settle-ms` (default 500) so the changes that follow the
//! snapshot are in the model, and prints. The first lines are summaries a script can grep:
//!
//! ```text
//! seats: xr
//! room: present
//! head: camera stereo
//! views: 2
//! ```
//!
//! Then one `node` line per node under each root, indented by depth.
//!
//! With `--record FILE` it also writes every frame the owner sends to FILE as it arrives, which
//! `xr_tree::replay` serves back to a test. Run it for as long as the stream should be, and stop it with a signal:
//! each frame is in the file when it has been read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use xr_scene::{CameraProjection, Class, NodeId, TreeView};
use xr_tree::TreeClient;

fn main() -> std::process::ExitCode {
    let mut settle = Duration::from_millis(500);
    let mut record = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--settle-ms" => match args.next().and_then(|value| value.parse().ok()) {
                Some(ms) => settle = Duration::from_millis(ms),
                None => return usage(),
            },
            "--record" => match args.next() {
                Some(path) => record = Some(PathBuf::from(path)),
                None => return usage(),
            },
            _ => return usage(),
        }
    }
    match run(settle, record.as_deref()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xr-smoke-tree: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn usage() -> std::process::ExitCode {
    eprintln!("usage: xr-smoke-tree [--settle-ms N] [--record FILE]");
    std::process::ExitCode::from(2)
}

fn run(settle: Duration, record: Option<&Path>) -> Result<(), String> {
    let socket = gnoblin_client::unix::socket_path(|name| std::env::var(name).ok())?;
    let mut tree = match record {
        Some(file) => TreeClient::connect_recording(&socket, file),
        None => TreeClient::connect(&socket),
    }
    .map_err(|error| format!("owner socket: {error}"))?;
    let deadline = Instant::now() + settle;
    while Instant::now() < deadline {
        tree.poll().map_err(|error| error.to_string())?;
        std::thread::sleep(Duration::from_millis(20));
    }
    print!("{}", render(tree.model()));
    Ok(())
}

fn class_name(class: &Class) -> String {
    match class {
        Class::Node => "node".to_owned(),
        Class::View {
            source,
            camera,
            width,
            height,
        } => format!(
            "view {width}x{height} source={} camera={}",
            source.map_or_else(|| "none".to_owned(), |id| id.to_string()),
            camera.map_or_else(|| "none".to_owned(), |id| id.to_string())
        ),
        Class::Camera { projection, .. } => format!(
            "camera {}",
            match projection {
                CameraProjection::Stereo => "stereo",
                CameraProjection::Orthographic => "orthographic",
            }
        ),
        Class::Seat => "seat".to_owned(),
        Class::Other => "other".to_owned(),
    }
}

/// The summary lines, then the tree.
fn render(view: &TreeView) -> String {
    let mut out = String::new();
    let seats: Vec<&str> = view
        .nodes
        .values()
        .filter(|node| matches!(node.class, Class::Seat))
        .filter_map(|node| node.name.as_deref())
        .collect();
    out.push_str(&format!("seats: {}\n", seats.join(",")));
    let room = view.nodes.values().find(|node| node.name.as_deref() == Some("room"));
    out.push_str(&format!(
        "room: {}\n",
        if room.is_some() { "present" } else { "absent" }
    ));
    if let Some(room) = room {
        let mut members = BTreeSet::new();
        let mut stack = vec![room.id];
        let mut views = 0;
        while let Some(id) = stack.pop() {
            if !members.insert(id) {
                continue;
            }
            let Some(node) = view.nodes.get(&id) else { continue };
            if matches!(node.class, Class::View { .. }) {
                views += 1;
            }
            if node.name.as_deref() == Some("head") {
                out.push_str(&format!("head: {}\n", class_name(&node.class)));
            }
            stack.extend(node.children.iter().copied());
        }
        out.push_str(&format!("views: {views}\n"));
    }
    for root in &view.roots {
        node_lines(view, *root, 0, &mut BTreeSet::new(), &mut out);
    }
    out
}

fn node_lines(view: &TreeView, id: NodeId, depth: usize, seen: &mut BTreeSet<NodeId>, out: &mut String) {
    let Some(node) = view.nodes.get(&id) else { return };
    if !seen.insert(id) {
        return;
    }
    out.push_str(&format!(
        "node {}id={id} class={}{}{} pos=[{:.1},{:.1},{:.1}] rot=[{:.1},{:.1},{:.1}] scale={:.4} visible={} input={}\n",
        "  ".repeat(depth),
        class_name(&node.class),
        node.name
            .as_deref()
            .map(|name| format!(" name={name}"))
            .unwrap_or_default(),
        node.metres.map(|m| format!(" metres={m}")).unwrap_or_default(),
        node.position[0],
        node.position[1],
        node.position[2],
        node.rotation[0],
        node.rotation[1],
        node.rotation[2],
        node.scale,
        node.visible,
        node.input,
    ));
    for child in &node.children {
        node_lines(view, *child, depth + 1, seen, out);
    }
}

//! Launcher usage counts — a simple per-app launch frequency, persisted to the
//! cache dir, so the most-used apps sort to the top (especially on an empty
//! query). Plain `<count> <app-id>` lines; best-effort (any error → no history).

use std::collections::HashMap;
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let dir = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".cache"))
        })?;
    Some(dir.join("gnoblin").join("launcher-usage"))
}

/// Load the saved usage counts (app-id → launch count).
pub fn load() -> HashMap<String, u32> {
    let mut m = HashMap::new();
    let Some(p) = path() else { return m };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return m;
    };
    for line in text.lines() {
        if let Some((count, id)) = line.split_once(' ') {
            if let Ok(c) = count.trim().parse::<u32>() {
                let id = id.trim();
                if !id.is_empty() {
                    m.insert(id.to_string(), c);
                }
            }
        }
    }
    m
}

/// Persist the usage counts.
pub fn save(map: &HashMap<String, u32>) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for (id, c) in map {
        out.push_str(c.to_string().as_str());
        out.push(' ');
        out.push_str(id);
        out.push('\n');
    }
    let _ = std::fs::write(&p, out);
}

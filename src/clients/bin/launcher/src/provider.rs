//! Launcher provider host — process/command search sources (the same ethos as
//! the QS plugins). Each provider is a command declared in `gnoblin.conf`:
//!
//! ```ini
//! [launcher-provider.files]
//! command = ~/.config/gnoblin/providers/files
//! prefix  = "f "         # optional keyword; the provider only runs when the
//!                        # query starts with it (the prefix is stripped before
//!                        # the command sees the query). Omit to run on every
//!                        # query (use sparingly — it runs per keystroke).
//! ```
//!
//! The command receives the (prefix-stripped) query in `$GNOBLIN_QUERY` and as
//! `$1`, and prints one result per line as TAB-separated fields:
//!
//! ```text
//! title <TAB> subtitle <TAB> icon <TAB> action
//! ```
//!
//! `icon` is a theme icon name (resolved via find_icon) or empty; `action` is a
//! shell command run via `sh -c` on activation (empty → the title is copied).
//! Dependency-free (no JSON) so providers are trivial to write in shell.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A provider declared under `[launcher-provider.NAME]`.
pub struct Provider {
    /// The NAME (kept for diagnostics / future per-provider ordering).
    #[allow(dead_code)]
    pub id: String,
    pub command: String,
    /// Keyword the query must start with for this provider to run (stripped
    /// before the command sees it). Empty = run on every query.
    pub prefix: String,
}

/// One result row emitted by a provider.
pub struct ProviderResult {
    pub title: String,
    pub subtitle: String,
    /// Theme icon name (find_icon) or empty.
    pub icon: String,
    /// Shell command run on activation; empty → copy the title.
    pub action: String,
}

/// Hard cap on how long a provider may run per keystroke (kept tight so the
/// synchronous run never janks typing; slow sources should be fast or paginate).
const PROVIDER_TIMEOUT: Duration = Duration::from_millis(450);

/// Load all `[launcher-provider.NAME]` declarations, in config order.
pub fn load() -> Vec<Provider> {
    let cfg = gnoblin_core::config::Config::load();
    let mut out = Vec::new();
    for section in cfg.sections_with_prefix("launcher-provider.") {
        let id = section.trim_start_matches("launcher-provider.").trim();
        if id.is_empty() {
            continue;
        }
        let Some(command) = cfg
            .get(section, "command")
            .map(str::trim)
            .filter(|c| !c.is_empty())
        else {
            continue;
        };
        out.push(Provider {
            id: id.to_string(),
            command: command.to_string(),
            prefix: cfg.get(section, "prefix").unwrap_or("").to_string(),
        });
    }
    out
}

/// Run `provider` for `query` (if its prefix matches), returning its results.
/// Returns empty on prefix mismatch, empty arg, spawn error, or timeout.
pub fn run(provider: &Provider, query: &str) -> Vec<ProviderResult> {
    let arg = if provider.prefix.is_empty() {
        query.trim()
    } else {
        match query.strip_prefix(&provider.prefix) {
            Some(rest) => rest.trim(),
            None => return Vec::new(),
        }
    };
    if arg.is_empty() {
        return Vec::new();
    }

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&provider.command)
        .arg("gnoblin-launcher") // $0
        .arg(arg) // $1
        .env("GNOBLIN_QUERY", arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let deadline = Instant::now() + PROVIDER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Vec::new();
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            Err(_) => return Vec::new(),
        }
    }

    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    out.lines().filter_map(parse_line).take(20).collect()
}

fn parse_line(line: &str) -> Option<ProviderResult> {
    let line = line.trim_end_matches('\r');
    if line.trim().is_empty() {
        return None;
    }
    let mut f = line.splitn(4, '\t');
    let title = f.next().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return None;
    }
    Some(ProviderResult {
        title,
        subtitle: f.next().unwrap_or("").trim().to_string(),
        icon: f.next().unwrap_or("").trim().to_string(),
        action: f.next().unwrap_or("").trim().to_string(),
    })
}

//! Command/process-driven quick-settings plugin host.
//!
//! A *plugin* is an external command, declared in `gnoblin.conf`, that emits the
//! quick-settings protocol as JSON on stdout and (optionally) receives events on
//! stdin. The shell renders each plugin as a tile in the control centre, plus an
//! optional submenu of rows.
//!
//! Two modes:
//!   * `persistent` — the process stays alive; it writes one ndjson line per
//!     update on stdout, and we write event ndjson lines to its stdin. A dead
//!     persistent process is restarted with exponential backoff.
//!   * `oneshot` — the process is run on an interval (and on every event); we
//!     capture its stdout, parse one JSON object, and exit. Events for a oneshot
//!     plugin are passed via `GNOBLIN_QS_EVENT` in the environment and re-run
//!     immediately.
//!
//! All process I/O happens on a dedicated background thread per plugin (matching
//! the tray / appmenu / quicksettings thread+channel pattern). The host
//! aggregates every plugin's latest [`PluginState`] and pushes a fresh snapshot
//! (the full ordered list) onto an mpsc channel; the topbar drains it each tick
//! and rebuilds the Slint model. Events flow back the other way over a per-host
//! command channel.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// How a plugin command is driven.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PluginMode {
    /// Long-lived process; ndjson updates on stdout, events on stdin.
    Persistent,
    /// Run-to-completion process; re-run on an interval and on each event.
    Oneshot,
}

/// A plugin declaration read from `[qs-plugin.NAME]` (or the `[providers]`
/// shorthand). `id` is the `NAME` part and is stable across config reloads.
#[derive(Clone, Debug)]
pub struct PluginConfig {
    pub id: String,
    pub command: String,
    pub mode: PluginMode,
    pub interval: Duration,
}

// ── Protocol: process → shell (deserialized from JSON) ───────────────────────

/// The tile face of a plugin: what the control-centre grid shows.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TileSpec {
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub chevron: bool,
    /// Grid span, 1..4 columns of the 4-wide grid. 0 = let the shell pick a
    /// sensible default for the layout (2 for a toggle, 4 for slider/row).
    #[serde(default)]
    pub span: i32,
    /// Tile shape: `toggle` (pill) | `slider` | `row` (full-width shortcut).
    /// Empty = `toggle`. A `slider`/`row` implies a full-width (span 4) tile.
    #[serde(default)]
    pub layout: String,
    /// Slider position 0..1 for `layout == "slider"` tiles (e.g. volume). Ignored
    /// by other layouts.
    #[serde(default)]
    pub value: f32,
}

/// One row of a plugin's submenu.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(from = "MenuRowWire")]
pub struct MenuRowSpec {
    /// Stable row id, echoed back in `row`/`toggle`/`slider` events. Empty for
    /// `separator`/`section`.
    pub id: String,
    /// `item | toggle | slider | action | separator | section`.
    pub kind: String,
    pub label: String,
    pub sublabel: String,
    pub icon: String,
    /// Slider position 0..1 (only for `slider`).
    pub value: f32,
    /// Toggle state (only for `toggle`).
    pub on: bool,
}

/// On-the-wire row shape. Per the protocol, `value` is a number for sliders but
/// a bool for toggles, so we accept it as a free-form JSON value and normalize:
/// a bool sets `on`; a number sets `value`. An explicit `on` always wins.
#[derive(Deserialize)]
struct MenuRowWire {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    sublabel: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    on: Option<bool>,
}

impl From<MenuRowWire> for MenuRowSpec {
    fn from(w: MenuRowWire) -> Self {
        let (mut value, mut on) = (0.0, false);
        match &w.value {
            Some(serde_json::Value::Bool(b)) => on = *b,
            Some(serde_json::Value::Number(n)) => value = n.as_f64().unwrap_or(0.0) as f32,
            _ => {}
        }
        if let Some(b) = w.on {
            on = b;
        }
        MenuRowSpec {
            id: w.id,
            kind: w.kind,
            label: w.label,
            sublabel: w.sublabel,
            icon: w.icon,
            value,
            on,
        }
    }
}

/// The optional submenu attached to a plugin tile.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MenuSpec {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub rows: Vec<MenuRowSpec>,
}

/// One full update emitted by a plugin (one JSON object / one ndjson line).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PluginUpdate {
    #[serde(default)]
    pub tile: TileSpec,
    #[serde(default)]
    pub menu: Option<MenuSpec>,
}

/// The host's view of one plugin: its declared id plus its latest update (if it
/// has produced one yet).
#[derive(Clone, Debug)]
pub struct PluginState {
    pub id: String,
    pub update: PluginUpdate,
    /// True once the plugin has emitted at least one valid update.
    pub ready: bool,
}

// ── Protocol: shell → process (events) ───────────────────────────────────────

/// An interaction the shell forwards to a plugin. `id` is the plugin id;
/// `row_id` (where present) is the row that was acted on.
#[derive(Clone, Debug)]
pub enum PluginEvent {
    TileClicked {
        id: String,
    },
    Row {
        id: String,
        row_id: String,
    },
    Toggle {
        id: String,
        row_id: String,
        value: bool,
    },
    Slider {
        id: String,
        row_id: String,
        value: f32,
    },
    /// The control centre was opened / closed (lets a plugin start/stop polling).
    Opened {
        id: String,
    },
    Closed {
        id: String,
    },
}

impl PluginEvent {
    fn plugin_id(&self) -> &str {
        match self {
            PluginEvent::TileClicked { id }
            | PluginEvent::Row { id, .. }
            | PluginEvent::Toggle { id, .. }
            | PluginEvent::Slider { id, .. }
            | PluginEvent::Opened { id }
            | PluginEvent::Closed { id } => id,
        }
    }

    /// Serialize to the on-the-wire ndjson event line (without trailing `\n`).
    fn to_json(&self) -> String {
        match self {
            PluginEvent::TileClicked { .. } => r#"{"event":"tile-clicked"}"#.to_string(),
            PluginEvent::Row { row_id, .. } => {
                format!(r#"{{"event":"row","id":{}}}"#, json_str(row_id))
            }
            PluginEvent::Toggle { row_id, value, .. } => format!(
                r#"{{"event":"toggle","id":{},"value":{}}}"#,
                json_str(row_id),
                value
            ),
            PluginEvent::Slider { row_id, value, .. } => format!(
                r#"{{"event":"slider","id":{},"value":{}}}"#,
                json_str(row_id),
                value
            ),
            PluginEvent::Opened { .. } => r#"{"event":"opened"}"#.to_string(),
            PluginEvent::Closed { .. } => r#"{"event":"closed"}"#.to_string(),
        }
    }
}

/// Minimal JSON string escaper for the small set of event payloads we emit
/// (avoids pulling serde_json's Serialize into the hot event path).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── Host ─────────────────────────────────────────────────────────────────────

/// Handle the topbar holds onto: drains state snapshots, sends events,
/// re-applies config on reload.
pub struct Host {
    /// Latest full snapshot (ordered as declared in config).
    state_rx: Receiver<Vec<PluginState>>,
    /// Per-plugin event senders (id → event channel into the worker thread).
    workers: HashMap<String, Worker>,
    /// The config the workers were last spawned with (to detect reloads).
    configs: Vec<PluginConfig>,
    /// Shared aggregated state + the snapshot publisher, handed to each worker.
    shared: Shared,
}

struct Worker {
    config: PluginConfig,
    event_tx: Sender<PluginEvent>,
}

/// State shared across worker threads: the aggregated per-plugin map plus the
/// channel a worker publishes a fresh ordered snapshot onto after each change.
#[derive(Clone)]
struct Shared {
    states: Arc<Mutex<HashMap<String, PluginState>>>,
    order: Arc<Mutex<Vec<String>>>,
    state_tx: Sender<Vec<PluginState>>,
}

impl Shared {
    /// Record a plugin's latest update and publish a full ordered snapshot.
    fn publish(&self, id: &str, update: PluginUpdate) {
        {
            let mut states = self.states.lock().unwrap_or_else(|p| p.into_inner());
            states.insert(
                id.to_string(),
                PluginState {
                    id: id.to_string(),
                    update,
                    ready: true,
                },
            );
        }
        self.emit();
    }

    /// Build + send the ordered snapshot (declared order; unready plugins are
    /// skipped so the grid only shows plugins that have produced data).
    fn emit(&self) {
        let order = self.order.lock().unwrap_or_else(|p| p.into_inner());
        let states = self.states.lock().unwrap_or_else(|p| p.into_inner());
        let snapshot: Vec<PluginState> = order
            .iter()
            .filter_map(|id| states.get(id).cloned())
            .filter(|s| s.ready)
            .collect();
        let _ = self.state_tx.send(snapshot);
    }
}

impl Host {
    /// Spawn workers for `configs`. Returns immediately; updates arrive on the
    /// channel drained by [`Host::poll`].
    pub fn spawn(configs: Vec<PluginConfig>) -> Self {
        let (state_tx, state_rx) = std::sync::mpsc::channel();
        let shared = Shared {
            states: Arc::new(Mutex::new(HashMap::new())),
            order: Arc::new(Mutex::new(configs.iter().map(|c| c.id.clone()).collect())),
            state_tx,
        };
        let mut host = Host {
            state_rx,
            workers: HashMap::new(),
            configs: Vec::new(),
            shared,
        };
        host.apply(configs);
        host
    }

    /// Drain to the latest snapshot, if any plugin changed since the last poll.
    pub fn poll(&self) -> Option<Vec<PluginState>> {
        let mut latest = None;
        while let Ok(snap) = self.state_rx.try_recv() {
            latest = Some(snap);
        }
        latest
    }

    /// Route an event to its plugin's worker.
    pub fn send_event(&self, event: PluginEvent) {
        if let Some(w) = self.workers.get(event.plugin_id()) {
            let _ = w.event_tx.send(event);
        }
    }

    /// Broadcast an open/close event to every plugin (so polling plugins can
    /// idle while the popout is closed).
    pub fn broadcast_open(&self, opened: bool) {
        for (id, w) in &self.workers {
            let ev = if opened {
                PluginEvent::Opened { id: id.clone() }
            } else {
                PluginEvent::Closed { id: id.clone() }
            };
            let _ = w.event_tx.send(ev);
        }
    }

    /// Whether the live config differs from what's running (caller compares).
    pub fn configs(&self) -> &[PluginConfig] {
        &self.configs
    }

    /// Re-spawn workers to match `configs`. Workers whose declaration is
    /// unchanged keep running; removed plugins have their threads stopped (the
    /// event channel drops, the worker loop notices and exits); new plugins are
    /// spawned. Cheap to call on every config-file change.
    pub fn apply(&mut self, configs: Vec<PluginConfig>) {
        if configs == self.configs {
            return;
        }
        // Stop workers no longer present (or whose declaration changed).
        let keep: HashMap<&str, &PluginConfig> =
            configs.iter().map(|c| (c.id.as_str(), c)).collect();
        self.workers
            .retain(|id, w| keep.get(id.as_str()).is_some_and(|c| **c == w.config));
        // Drop aggregated state for plugins that went away.
        {
            let live: std::collections::HashSet<&str> =
                configs.iter().map(|c| c.id.as_str()).collect();
            let mut states = self.shared.states.lock().unwrap_or_else(|p| p.into_inner());
            states.retain(|id, _| live.contains(id.as_str()));
        }
        *self.shared.order.lock().unwrap_or_else(|p| p.into_inner()) =
            configs.iter().map(|c| c.id.clone()).collect();
        // Spawn workers for any plugin not already running.
        for cfg in &configs {
            if self.workers.contains_key(&cfg.id) {
                continue;
            }
            let (event_tx, event_rx) = std::sync::mpsc::channel();
            spawn_worker(cfg.clone(), event_rx, self.shared.clone());
            self.workers.insert(
                cfg.id.clone(),
                Worker {
                    config: cfg.clone(),
                    event_tx,
                },
            );
        }
        self.configs = configs;
        // Re-emit so the topbar drops any removed tiles immediately.
        self.shared.emit();
    }
}

// ── Workers ──────────────────────────────────────────────────────────────────

/// Spawn a thread that drives one plugin for the lifetime of its event channel.
fn spawn_worker(config: PluginConfig, event_rx: Receiver<PluginEvent>, shared: Shared) {
    std::thread::Builder::new()
        .name(format!("qs-plugin-{}", config.id))
        .spawn(move || match config.mode {
            PluginMode::Persistent => run_persistent(config, event_rx, shared),
            PluginMode::Oneshot => run_oneshot(config, event_rx, shared),
        })
        .ok();
}

/// Build the `sh -c <command>` invocation shared by both modes.
fn base_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

const RESTART_BACKOFF_MIN: Duration = Duration::from_millis(500);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(30);
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(10);

/// Persistent plugin: keep the process alive, read ndjson updates on a reader
/// thread, write event ndjson to stdin, restart on death with backoff.
fn run_persistent(config: PluginConfig, event_rx: Receiver<PluginEvent>, shared: Shared) {
    let mut backoff = RESTART_BACKOFF_MIN;
    loop {
        let mut child = match base_command(&config.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("gnoblin qs-plugin {}: spawn failed: {e}", config.id);
                if !sleep_or_quit(&event_rx, backoff) {
                    return;
                }
                backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
                continue;
            }
        };
        let started = Instant::now();

        // Reader thread: parse each stdout line and publish on change.
        let stdout = child.stdout.take();
        let reader_shared = shared.clone();
        let reader_id = config.id.clone();
        let reader = std::thread::spawn(move || {
            if let Some(out) = stdout {
                let buf = BufReader::new(out);
                for line in buf.lines().map_while(Result::ok) {
                    if let Some(update) = parse_update(&line) {
                        reader_shared.publish(&reader_id, update);
                    }
                }
            }
        });

        let mut stdin = child.stdin.take();
        // Event pump: block on the event channel, write ndjson to stdin, and
        // bail out if the child died (so we can restart it).
        loop {
            match event_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
                    if let Some(stdin) = stdin.as_mut() {
                        let line = format!("{}\n", event.to_json());
                        if stdin.write_all(line.as_bytes()).is_err() || stdin.flush().is_err() {
                            break;
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Plugin removed from config: kill + stop for good.
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return;
                }
            }
            if let Ok(Some(_)) = child.try_wait() {
                break; // child exited; fall through to restart
            }
        }

        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();

        // Reset backoff if the process ran for a healthy while before dying.
        if started.elapsed() > Duration::from_secs(10) {
            backoff = RESTART_BACKOFF_MIN;
        }
        if !sleep_or_quit(&event_rx, backoff) {
            return;
        }
        backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
    }
}

/// Oneshot plugin: run on the configured interval and immediately on every
/// event. The event is passed via `GNOBLIN_QS_EVENT` so the script can react.
fn run_oneshot(config: PluginConfig, event_rx: Receiver<PluginEvent>, shared: Shared) {
    // Initial read.
    if let Some(update) = run_oneshot_once(&config, None) {
        shared.publish(&config.id, update);
    }
    loop {
        match event_rx.recv_timeout(config.interval) {
            Ok(event) => {
                // Coalesce a burst: a slider drag emits many events while one
                // run is in flight. Drain everything queued and act on only the
                // LATEST, so we spawn the command once per burst (one wpctl call)
                // instead of once per drag pixel — that's the drag-lag smoothing.
                let mut latest = event;
                while let Ok(next) = event_rx.try_recv() {
                    latest = next;
                }
                if let Some(update) = run_oneshot_once(&config, Some(&latest)) {
                    shared.publish(&config.id, update);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Some(update) = run_oneshot_once(&config, None) {
                    shared.publish(&config.id, update);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Run a oneshot plugin once with a timeout, returning its parsed update.
fn run_oneshot_once(config: &PluginConfig, event: Option<&PluginEvent>) -> Option<PluginUpdate> {
    let mut cmd = base_command(&config.command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(ev) = event {
        cmd.env("GNOBLIN_QS_EVENT", ev.to_json());
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gnoblin qs-plugin {}: spawn failed: {e}", config.id);
            return None;
        }
    };
    // Bounded wait: kill a oneshot that overruns its timeout.
    let deadline = Instant::now() + ONESHOT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("gnoblin qs-plugin {}: timed out", config.id);
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    // Oneshot may emit one object (last non-empty line wins, tolerating ndjson).
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(parse_update)
}

/// Parse one line as a [`PluginUpdate`]; malformed lines are ignored.
fn parse_update(line: &str) -> Option<PluginUpdate> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str::<PluginUpdate>(line).ok()
}

/// Sleep for `dur` while still draining (and discarding) events, so a removed
/// plugin's channel disconnect is noticed promptly. Returns false if the channel
/// disconnected (caller should stop), true on timeout.
fn sleep_or_quit(event_rx: &Receiver<PluginEvent>, dur: Duration) -> bool {
    !matches!(
        event_rx.recv_timeout(dur),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    )
}

// ── Config parsing ───────────────────────────────────────────────────────────

/// Parse a duration like `5s`, `500ms`, `2m`, or a bare integer (seconds).
pub fn parse_interval(raw: &str, fallback: Duration) -> Duration {
    let raw = raw.trim();
    if raw.is_empty() {
        return fallback;
    }
    let (num, unit) = raw
        .find(|c: char| c.is_alphabetic())
        .map(|i| (raw[..i].trim(), raw[i..].trim()))
        .unwrap_or((raw, ""));
    let Ok(n) = num.parse::<f64>() else {
        return fallback;
    };
    let secs = match unit {
        "ms" => n / 1000.0,
        "m" | "min" => n * 60.0,
        "h" => n * 3600.0,
        "" | "s" | "sec" | "secs" => n,
        _ => return fallback,
    };
    if secs <= 0.0 {
        fallback
    } else {
        Duration::from_secs_f64(secs)
    }
}

/// Read the `[qs-plugin.NAME]` sections of one config into `out`, overlaying:
/// a section whose id already exists replaces it; `enabled = off` removes it;
/// otherwise it's appended (declaration order).
fn read_qs_sections(cfg: &crate::config::Config, out: &mut Vec<PluginConfig>) {
    for section in cfg.sections_with_prefix("qs-plugin.") {
        let id = section.trim_start_matches("qs-plugin.").trim();
        if id.is_empty() {
            continue;
        }
        if cfg.get(section, "enabled").map(str::trim) == Some("off") {
            out.retain(|p| p.id != id);
            continue;
        }
        let Some(command) = cfg.get(section, "command").filter(|c| !c.trim().is_empty()) else {
            continue;
        };
        let mode = match cfg.get(section, "mode").map(str::trim) {
            Some("persistent") => PluginMode::Persistent,
            _ => PluginMode::Oneshot,
        };
        let interval = parse_interval(
            cfg.get(section, "interval").unwrap_or(""),
            Duration::from_secs(5),
        );
        let pc = PluginConfig {
            id: id.to_string(),
            command: command.to_string(),
            mode,
            interval,
        };
        match out.iter_mut().find(|p| p.id == id) {
            Some(slot) => *slot = pc,
            None => out.push(pc),
        }
    }
}

/// The quick-settings plugin set: gnoblin's shipped defaults
/// (`data/gnoblin.defaults.conf`) overlaid by the user's `gnoblin.conf` — the
/// user can override a plugin by id, disable one with `enabled = off`, add new
/// ones, and reorder them all with `[quicksettings] order = a, b, c`. The tiles
/// are config-declared plugins, NOT hardcoded in the shell.
pub fn load_configs(cfg: &crate::config::Config) -> Vec<PluginConfig> {
    let defaults =
        crate::config::Config::from_text(include_str!("../../../data/gnoblin.defaults.conf"));
    let mut out = Vec::new();
    read_qs_sections(&defaults, &mut out);
    read_qs_sections(cfg, &mut out);

    // `[providers] get-foo = cmd` shorthand (user only): data-only oneshot providers.
    for (key, command) in cfg.entries_with_prefix("providers", "get-") {
        let id = key.trim_start_matches("get-").trim();
        if id.is_empty() || command.trim().is_empty() {
            continue;
        }
        if out.iter().any(|p| p.id == id) {
            continue;
        }
        out.push(PluginConfig {
            id: id.to_string(),
            command: command.to_string(),
            mode: PluginMode::Oneshot,
            interval: Duration::from_secs(5),
        });
    }

    // Order: `[quicksettings] order = ...` (user's wins, else the default's).
    // Listed ids come first in that order; anything unlisted keeps its relative
    // position at the end (stable sort).
    if let Some(order) = cfg
        .get("quicksettings", "order")
        .or_else(|| defaults.get("quicksettings", "order"))
    {
        let want: Vec<&str> = order
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        out.sort_by_key(|p| want.iter().position(|w| *w == p.id).unwrap_or(usize::MAX));
    }
    out
}

impl PartialEq for PluginConfig {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.command == other.command
            && self.mode == other.mode
            && self.interval == other.interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_parsing() {
        let fb = Duration::from_secs(5);
        assert_eq!(parse_interval("5s", fb), Duration::from_secs(5));
        assert_eq!(parse_interval("500ms", fb), Duration::from_millis(500));
        assert_eq!(parse_interval("2m", fb), Duration::from_secs(120));
        assert_eq!(parse_interval("3", fb), Duration::from_secs(3));
        assert_eq!(parse_interval("", fb), fb);
        assert_eq!(parse_interval("garbage", fb), fb);
        assert_eq!(parse_interval("0s", fb), fb);
    }

    #[test]
    fn parses_full_protocol_update() {
        let line = r#"{"tile":{"icon":"tailscale","title":"Tailscale","subtitle":"Connected","active":true,"chevron":true},"menu":{"title":"Tailscale","rows":[{"id":"exit","type":"toggle","label":"Use exit node","value":true},{"type":"separator"},{"id":"admin","type":"action","label":"Admin"}]}}"#;
        let u = parse_update(line).expect("valid");
        assert_eq!(u.tile.title, "Tailscale");
        assert!(u.tile.active);
        assert!(u.tile.chevron);
        let menu = u.menu.expect("menu present");
        assert_eq!(menu.rows.len(), 3);
        assert_eq!(menu.rows[0].kind, "toggle");
        assert_eq!(menu.rows[0].id, "exit");
        assert_eq!(menu.rows[1].kind, "separator");
        assert_eq!(menu.rows[2].kind, "action");
    }

    #[test]
    fn tile_only_update_has_no_menu() {
        let u = parse_update(r#"{"tile":{"title":"X"}}"#).expect("valid");
        assert_eq!(u.tile.title, "X");
        assert!(u.menu.is_none());
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(parse_update("not json").is_none());
        assert!(parse_update("").is_none());
        assert!(parse_update("   ").is_none());
        // Partial/garbage still rejected.
        assert!(parse_update("{\"tile\":").is_none());
    }

    #[test]
    fn event_serialization() {
        let row = PluginEvent::Row {
            id: "p".into(),
            row_id: "peer-1".into(),
        };
        assert_eq!(row.to_json(), r#"{"event":"row","id":"peer-1"}"#);
        let tog = PluginEvent::Toggle {
            id: "p".into(),
            row_id: "exit".into(),
            value: false,
        };
        assert_eq!(
            tog.to_json(),
            r#"{"event":"toggle","id":"exit","value":false}"#
        );
        let sld = PluginEvent::Slider {
            id: "p".into(),
            row_id: "vol".into(),
            value: 0.42,
        };
        assert_eq!(
            sld.to_json(),
            r#"{"event":"slider","id":"vol","value":0.42}"#
        );
        assert_eq!(
            PluginEvent::TileClicked { id: "p".into() }.to_json(),
            r#"{"event":"tile-clicked"}"#
        );
        assert_eq!(
            PluginEvent::Opened { id: "p".into() }.to_json(),
            r#"{"event":"opened"}"#
        );
    }

    #[test]
    fn json_string_escaping() {
        assert_eq!(json_str(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_str("tab\there"), r#""tab\there""#);
    }

    #[test]
    fn load_configs_overlays_user_onto_defaults() {
        // Empty user config → the shipped defaults, ordered by the defaults'
        // [quicksettings] order (wifi first).
        let defs = load_configs(&crate::config::Config::from_text(""));
        assert!(defs.iter().any(|p| p.id == "wifi"));
        assert!(defs.iter().any(|p| p.id == "darkstyle"));
        assert_eq!(defs.first().map(|p| p.id.as_str()), Some("wifi"));

        // User overrides wifi (command/mode/interval win), adds mpris + a
        // [providers] shorthand; defaults stay present.
        let cfg = crate::config::Config::from_text(
            "[qs-plugin.wifi]\n\
             command = /my/wifi\n\
             mode = persistent\n\
             interval = 9s\n\
             [qs-plugin.mpris]\n\
             command = /path/to/mpris\n\
             mode = persistent\n\
             [providers]\n\
             get-battery = cat /sys/class/power_supply/BAT0/capacity\n",
        );
        let p = load_configs(&cfg);
        let wifi = p.iter().find(|x| x.id == "wifi").expect("wifi present");
        assert!(wifi.command.contains("/my/wifi")); // user override won
        assert_eq!(wifi.mode, PluginMode::Persistent);
        assert_eq!(wifi.interval, Duration::from_secs(9));
        assert!(p.iter().any(|x| x.id == "mpris")); // user-added
        assert!(p.iter().any(|x| x.id == "battery")); // provider shorthand
        assert!(p.iter().any(|x| x.id == "bluetooth")); // default still there

        // `enabled = off` removes a default.
        let off = load_configs(&crate::config::Config::from_text(
            "[qs-plugin.bluetooth]\nenabled = off\n",
        ));
        assert!(!off.iter().any(|x| x.id == "bluetooth"));
        assert!(off.iter().any(|x| x.id == "wifi"));
    }
}

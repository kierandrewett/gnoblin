//! A soak of the real session's memory: the compositor, the stock shell and a client that churns (design/gpu-memory.md).
//!
//! The session is [`Compositor::run`] on a headless backend with the renderer the soak is told to use, the stock
//! JavaScript shell runs over its owner socket as it does in a real session, and a Wayland client opens, resizes and
//! closes windows, takes screenshots, toggles supersampling and hotplugs a second output, for as long as it is told
//! to. Every sample interval one JSON line records what the process holds (resident set, threads, descriptors) and
//! what the compositor says it holds for each output (the `gpu_memory_*` properties of its screen nodes), so growth
//! shows as a series and the allocator that grows is named by the column that rises.
//!
//! It is `#[ignore]`: a soak is minutes, not a unit test. Run it on purpose:
//!
//! ```text
//! GNOBLIN_SOAK_RENDERER=gles GNOBLIN_SOAK_SECONDS=1200 GNOBLIN_SOAK_OUT=soak-gles.jsonl \
//!     cargo test -p gnoblin --test gpu_memory_soak -- --ignored --nocapture
//! ```
//!
//! What it does not prove: a real GPU's memory (this crate counts what it asked the driver for, and a headless
//! software driver does not report video memory at all), the DRM scan-out path, and the session lock, which needs a
//! password prompt and is covered by `js_lock.rs`.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::{Compositor, OutputTap, SessionConfig, StopHandle};
use gnoblin_config::RendererChoice;
use gnoblin_js::{JsRole, JsRoleConfig};
use gnoblin_proto::Version;
use serde_json::json;

mod support;
use support::{Mapper, scene, snapshot};

/// How long the session may take to start listening.
const PATIENCE: Duration = Duration::from_secs(60);

/// The sizes the churn client opens and resizes windows to, in pixels: a small dialog up to a 1080p window.
const SIZES: [(i32, i32); 6] = [
    (320, 240),
    (800, 600),
    (1280, 720),
    (1920, 1080),
    (640, 480),
    (1024, 768),
];

/// The supersample factors the churn cycles an output through.
const FACTORS: [&str; 3] = ["2", "4", "1"];

/// How many windows the long-lived client keeps before it disconnects and starts over.
const MAX_WINDOWS: usize = 6;

/// The environment's value, or `default`.
fn setting(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

/// The renderer the soak runs on: `software`, `gles` or `vulkan`.
fn renderer() -> RendererChoice {
    match setting("GNOBLIN_SOAK_RENDERER", "software").as_str() {
        "software" => RendererChoice::Software,
        "gles" => RendererChoice::Gles,
        "vulkan" => RendererChoice::Vulkan,
        other => panic!("GNOBLIN_SOAK_RENDERER is software, gles or vulkan, not {other:?}"),
    }
}

/// A headless compositor on sockets of its own.
struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    outputs: OutputTap,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn start(renderer: RendererChoice) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let stop = StopHandle::new();
        let outputs = OutputTap::default();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            output_tap: Some(outputs.clone()),
            renderer: Some(renderer),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        // A pause of the loop under load is not a hung application, so the liveness pings stay off.
        config.wayland.check_alive_timeout = Duration::ZERO;
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || {
                Compositor::run(config).expect("the compositor runs until it is stopped");
            })
            .expect("the session thread");
        let session = Self {
            directory,
            stop,
            outputs,
            thread: Some(thread),
        };
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(session.ipc()).is_err() {
            assert!(Instant::now() < deadline, "the compositor never listened");
            std::thread::sleep(Duration::from_millis(5));
        }
        session
    }

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    fn wayland(&self) -> PathBuf {
        self.directory.path().join("wayland.sock")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The stock shell, in a thread of its own as the runtime role is a process of its own.
struct Shell {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Shell {
    /// Starts the shell with an empty configuration directory, so the stock configuration runs.
    fn start(ipc: &Path, config: &Path) -> Self {
        let socket = ipc.to_str().expect("a UTF-8 socket path").to_owned();
        let dir = config.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("js-shell".to_owned())
            .spawn(move || {
                let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
                let mut role = JsRole::connect(&env, JsRoleConfig::new(&dir, None)).expect("the shell connects");
                while !stopped.load(Ordering::Relaxed) {
                    role.tick().expect("the shell runs");
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .expect("the shell thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A fresh owner connection, for the screenshots.
fn owner(ipc: &Path) -> Client<UnixTransport> {
    let transport = UnixTransport::connect(ipc).expect("the session is listening");
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Client::connect(transport, Version::new(major, minor, patch)).expect("the handshake")
}

/// One `gnoblinctl` run against the session.
fn ctl(ipc: &Path, arguments: &[&str]) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        arguments.iter().map(std::ffi::OsString::from),
        gnoblinctl::SocketConnection::at(ipc),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// The object id of the first output `output list` prints.
fn first_output(ipc: &Path) -> Option<u64> {
    let (_, out, _) = ctl(ipc, &["output", "list"]);
    out.lines().next()?.split('\t').next()?.parse().ok()
}

/// Applies a supersample factor to an output, keeping its mode.
fn supersample(ipc: &Path, output: u64, factor: &str) -> bool {
    let id = output.to_string();
    let (_, mode, _) = ctl(ipc, &["output", "get", &id, "mode"]);
    let mode = mode.trim().to_owned();
    let supersample = format!("supersample={factor}");
    let mode = format!("mode={mode}");
    let (code, _, error) = ctl(
        ipc,
        &[
            "output",
            "call",
            &id,
            "apply",
            "x=0",
            "y=0",
            "scale=1",
            "transform=0",
            "enabled=true",
            &mode,
            &supersample,
        ],
    );
    if code != 0 {
        eprintln!("supersample {factor} was refused: {error}");
    }
    code == 0
}

/// A numeric field of `/proc/self/status`: kilobytes for the `Vm*` ones, a count for `Threads`.
fn status_field(name: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix(name)?
                .strip_prefix(':')?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// How many entries a directory of `/proc/self` has.
fn entries(path: &str) -> usize {
    fs::read_dir(path).map_or(0, Iterator::count)
}

/// Every screen node's `gpu_memory_*` properties, as one JSON object per screen.
fn screens(ipc: &Path) -> serde_json::Value {
    let mut rows = Vec::new();
    for node in snapshot(ipc).into_iter().filter(|node| node.class == "screen") {
        let mut row = serde_json::Map::new();
        row.insert("node".to_owned(), json!(node.id));
        for prop in &node.props {
            if prop.key.starts_with("gpu_memory_") || prop.key == "supersample" || prop.key == "mode" {
                let value = if prop.key == "mode" {
                    json!(prop.value_text)
                } else {
                    json!(prop.value_double)
                };
                row.insert(prop.key.clone(), value);
            }
        }
        rows.push(serde_json::Value::Object(row));
    }
    serde_json::Value::Array(rows)
}

/// The sum of `key` over the screens a sample holds.
fn sum(screens: &serde_json::Value, key: &str) -> f64 {
    screens
        .as_array()
        .map_or(0.0, |rows| rows.iter().filter_map(|row| row[key].as_f64()).sum())
}

#[test]
#[ignore = "a soak runs for minutes: GNOBLIN_SOAK_SECONDS, see this file's header"]
fn the_session_keeps_its_memory_bounded_under_churn() {
    let seconds: u64 = setting("GNOBLIN_SOAK_SECONDS", "60")
        .parse()
        .expect("GNOBLIN_SOAK_SECONDS is a number");
    let sample_every: u64 = setting("GNOBLIN_SOAK_SAMPLE_SECONDS", "10")
        .parse()
        .expect("GNOBLIN_SOAK_SAMPLE_SECONDS is a number");
    let out_path = setting("GNOBLIN_SOAK_OUT", "gpu-memory-soak.jsonl");
    let mut out = fs::File::create(&out_path).expect("the series file");
    let renderer_name = setting("GNOBLIN_SOAK_RENDERER", "software");

    let session = Session::start(renderer());
    let config = tempfile::tempdir().expect("a config directory");
    let _shell = Shell::start(&session.ipc(), config.path());
    let ipc = session.ipc();
    let output = {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(output) = first_output(&ipc) {
                break output;
            }
            assert!(Instant::now() < deadline, "the session never listed an output");
            std::thread::sleep(Duration::from_millis(25));
        }
    };

    let started = Instant::now();
    let mut next_sample = started;
    let mut cycle: u64 = 0;
    let mut mapper = Mapper::connect(&session.wayland());
    let mut windows = 0usize;
    let mut extra: Option<u64> = None;
    let mut screenshots = 0u64;
    let mut refused_screenshots = 0u64;
    let mut factor = "1";
    let mut sample = |cycle: u64, windows: usize, factor: &str, screenshots: u64, refused: u64| {
        let screens = screens(&ipc);
        let line = json!({
            "t": started.elapsed().as_secs_f64(),
            "renderer": renderer_name,
            "cycle": cycle,
            "windows_mapped": windows,
            "windows_in_scene": scene(&ipc).len(),
            "supersample": factor,
            "screenshots": screenshots,
            "screenshots_refused": refused,
            "rss": status_field("VmRSS") * 1024,
            "rss_peak": status_field("VmHWM") * 1024,
            "threads": status_field("Threads"),
            "fds": entries("/proc/self/fd"),
            "gpu_memory_bytes": sum(&screens, "gpu_memory_bytes"),
            "gpu_memory_capture_bytes": sum(&screens, "gpu_memory_capture_bytes"),
            "gpu_memory_staging_bytes": sum(&screens, "gpu_memory_staging_bytes"),
            "screens": screens,
        });
        writeln!(out, "{line}").expect("a series line");
        out.flush().expect("a flushed series");
        println!("{line}");
    };
    sample(cycle, windows, factor, screenshots, refused_screenshots);

    while started.elapsed() < Duration::from_secs(seconds) {
        cycle += 1;
        let (width, height) = SIZES[(cycle as usize) % SIZES.len()];
        // Windows come and go: the long-lived client opens one per cycle, resizes an older one, and disconnects whole
        // when it has too many, so both a window's close and a client's teardown are exercised.
        if windows >= MAX_WINDOWS {
            mapper = Mapper::connect(&session.wayland());
            windows = 0;
        }
        mapper.map_window_sized(&format!("org.example.soak{cycle}"), "soak", width, height);
        windows += 1;
        if windows > 1 {
            let (width, height) = SIZES[(cycle as usize + 2) % SIZES.len()];
            mapper.commit_geometry(cycle as usize % (windows - 1), (0, 0, width, height));
        }
        // A client that opens and closes at once: a connection per cycle.
        {
            let mut short = Mapper::connect(&session.wayland());
            short.map_window_sized("org.example.short", "short", 400, 300);
        }
        // Screenshots through the control socket: both the composed frame and the one the display shows.
        let mut control = owner(&ipc);
        let shot = if cycle % 2 == 0 {
            control.capture_screen()
        } else {
            control.capture_screen_display()
        };
        match shot {
            Ok(_) => screenshots += 1,
            Err(_) => refused_screenshots += 1,
        }
        // A factor every fifth cycle: 2, 4, then back to 1.
        if cycle % 5 == 0 {
            let next = FACTORS[(cycle / 5 % 3) as usize];
            if supersample(&ipc, output, next) {
                factor = next;
            }
        }
        // A second output plugged for three cycles of every seven.
        match (cycle % 7, extra) {
            (1, None) => {
                session.outputs.connect("Virtual-9", 1024, 768);
                extra = Some(cycle);
            }
            (4, Some(_)) => {
                if let Some(second) = plugged(&ipc, "Virtual-9") {
                    session.outputs.disconnect(gnoblin_types::OutputId::new(second));
                }
                extra = None;
            }
            _ => {}
        }
        if Instant::now() >= next_sample {
            sample(cycle, windows, factor, screenshots, refused_screenshots);
            next_sample += Duration::from_secs(sample_every);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    // The end: every client gone, the factor back to one, the second output unplugged, then a settled reading.
    drop(mapper);
    let _ = supersample(&ipc, output, "1");
    std::thread::sleep(Duration::from_secs(3));
    sample(cycle, 0, "1", screenshots, refused_screenshots);
}

/// The backend id of the output on `connector`: the screen node is named `output-N` after it.
fn plugged(ipc: &Path, connector: &str) -> Option<u32> {
    snapshot(ipc)
        .into_iter()
        .filter(|node| node.class == "screen")
        .find_map(|node| {
            let prop = |key: &str| {
                node.props
                    .iter()
                    .find(|prop| prop.key == key)
                    .map(|prop| prop.value_text.clone())
            };
            (prop("connector")? == connector)
                .then(|| prop("name")?.strip_prefix("output-")?.parse().ok())
                .flatten()
        })
}

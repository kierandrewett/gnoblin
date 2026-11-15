//! The performance baseline: the same measurements on every target, taken from outside the compositor.
//!
//! `gnoblin-parity perf <target>` starts the target the way a scenario does (a launcher for the two Mutter targets,
//! `gnoblin-parity serve-candidate` for the Rust candidate, so all three are separate processes with the same
//! accounting), drives it with the shared Wayland client and EIS peer, and measures:
//!
//! - **Frame time.** One window redraws as fast as its `wl_surface.frame` callbacks allow for a few seconds, and the
//!   gaps between callbacks are the frame times the client sees. A virtual monitor refreshes at 60 Hz, so the ideal gap
//!   is 16.7 ms.
//! - **Input-to-photon proxy.** An EIS key press to the client's `wl_keyboard.key` event, and on to the frame callback
//!   of the redraw the client commits as its answer. There is no panel to photograph, so this is the time to the
//!   compositor's own "this frame was presented" signal, which is what a headless session has.
//! - **Idle CPU.** CPU time of every process the launcher started, over an idle window, as a share of one core.
//! - **Memory** with 0, 10 and 50 windows mapped: the proportional set size of those processes where the kernel
//!   reports it, and the resident set size otherwise.
//!
//! The numbers say how the targets compare on one machine under one headless software renderer. They do not say how a
//! compositor performs on a GPU, a real panel or a real input stack, and the report says so.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::candidate;
use crate::ei_peer::{EiPeer, EisDoor, SocketDoor};
use crate::launcher::{BUS_SOCKET, DISPLAY, LauncherSession, Names, private_directory};
use crate::mutter_gnoblin::MutterGnoblinLaunch;
use crate::procfs::{self, Sample};
use crate::remote_desktop::RemoteDesktopDoor;
use crate::target::{RunEnvironment, TargetError};
use crate::wayland_peer::{Toplevel, WaylandPeer};

/// How long the session settles after it starts or after windows map, before it is measured.
const SETTLE: Duration = Duration::from_secs(3);
/// How long the idle CPU is sampled.
const IDLE: Duration = Duration::from_secs(10);
/// How long the frame-time window redraws.
const FRAMES: Duration = Duration::from_secs(5);
/// How many key presses the input latency is taken from.
const KEYS: usize = 100;
/// The window counts memory is read at.
const WINDOW_COUNTS: [usize; 3] = [0, 10, 50];
/// How long one latency sample waits for the compositor before it counts as lost.
const LOST: Duration = Duration::from_secs(2);
/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;

/// A target the baseline can be taken on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfTarget {
    /// The Rust candidate, as a child process.
    Candidate,
    /// Stock Mutter 51 on the headless backend.
    Mutter51,
    /// Gnoblin on the patched Mutter.
    MutterGnoblin,
}

impl PerfTarget {
    /// The target by the name scenarios use.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "candidate" => Some(Self::Candidate),
            "mutter51" => Some(Self::Mutter51),
            "mutter-gnoblin" => Some(Self::MutterGnoblin),
            _ => None,
        }
    }

    /// The name scenarios and reports use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Mutter51 => "mutter51",
            Self::MutterGnoblin => "mutter-gnoblin",
        }
    }
}

/// Distribution of a set of durations, in milliseconds.
#[derive(Debug, Serialize)]
pub struct LatencyStats {
    /// How many samples the numbers are from.
    pub samples: usize,
    pub min_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl LatencyStats {
    /// The distribution of `samples`, or `None` when there are none.
    fn of(samples: &[Duration]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = samples.iter().map(|sample| sample.as_secs_f64() * 1_000.0).collect();
        sorted.sort_by(f64::total_cmp);
        let rank = |fraction: f64| {
            let index = (fraction * sorted.len() as f64).ceil() as usize;
            sorted[index.clamp(1, sorted.len()) - 1]
        };
        Some(Self {
            samples: sorted.len(),
            min_ms: sorted[0],
            mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50_ms: rank(0.50),
            p95_ms: rank(0.95),
            p99_ms: rank(0.99),
            max_ms: sorted[sorted.len() - 1],
        })
    }
}

/// The machine the numbers were taken on.
#[derive(Debug, Serialize)]
pub struct HostSpecs {
    pub kernel: String,
    pub cpu_model: String,
    pub cpus: usize,
    pub memory_total_kib: u64,
}

/// Memory of the target's processes with a number of windows mapped.
#[derive(Debug, Serialize)]
pub struct MemoryPoint {
    pub windows: usize,
    /// What the figures are: `pss` or `rss`.
    pub metric: &'static str,
    pub total_kib: u64,
    /// The largest process by that metric, as `comm (pid)`, so the report says where the memory is.
    pub largest: String,
}

/// CPU used while nothing happens.
#[derive(Debug, Serialize)]
pub struct IdleCpu {
    pub windows: usize,
    pub seconds: f64,
    /// Share of one core, summed over every process of the session.
    pub percent_of_one_core: f64,
    /// The process that used the most, as `comm (pid)`.
    pub busiest: String,
}

/// One baseline.
#[derive(Debug, Serialize)]
pub struct PerfReport {
    pub target: &'static str,
    pub host: HostSpecs,
    /// Seconds from starting the launcher until the Wayland socket accepted.
    pub startup_seconds: f64,
    /// The processes of the session, as `comm`.
    pub processes: Vec<String>,
    pub idle: Vec<IdleCpu>,
    pub memory: Vec<MemoryPoint>,
    pub frame_time: Option<LatencyStats>,
    pub frame_count: usize,
    /// EIS key press to the client's `wl_keyboard.key`.
    pub key_to_client: Option<LatencyStats>,
    /// EIS key press to the frame callback of the redraw the client committed on receiving the key.
    pub key_to_frame: Option<LatencyStats>,
    /// Key presses that produced no key event within the limit.
    pub keys_lost: usize,
    /// What these numbers do not show.
    pub limits: Vec<&'static str>,
}

/// A started target and the ways to reach it.
struct Measured {
    session: LauncherSession,
    socket: PathBuf,
    door: Box<dyn EisDoor>,
    _directory: tempfile::TempDir,
}

fn live(target: PerfTarget, detail: impl Into<String>) -> TargetError {
    TargetError::Live {
        target: target.name().to_owned(),
        detail: detail.into(),
    }
}

impl Measured {
    fn start(target: PerfTarget, environment: &RunEnvironment) -> Result<Self, TargetError> {
        let directory = private_directory(target.name(), environment.runtime_base.as_deref())?;
        let socket = directory.path().join(DISPLAY);
        let (session, door): (LauncherSession, Box<dyn EisDoor>) = match target {
            PerfTarget::Candidate => {
                let binary = std::env::current_exe().map_err(|error| {
                    live(
                        target,
                        format!("cannot find this binary to start the candidate: {error}"),
                    )
                })?;
                let names = Names {
                    target: "candidate",
                    log: "candidate",
                };
                let session =
                    LauncherSession::start(names, binary.as_os_str(), &["serve-candidate"], directory.path())?;
                let door = SocketDoor::new("candidate", directory.path().join(candidate::EIS_SOCKET));
                (session, Box::new(door))
            }
            PerfTarget::Mutter51 => {
                let launcher = environment.mutter51_launcher.clone().ok_or_else(|| {
                    live(
                        target,
                        format!("{} names no launcher", crate::mutter::LAUNCHER_VARIABLE),
                    )
                })?;
                let names = Names {
                    target: "mutter51",
                    log: "mutter",
                };
                let session = LauncherSession::start(names, &launcher, &[], directory.path())?;
                let door = RemoteDesktopDoor::new("mutter51", directory.path().join(BUS_SOCKET));
                (session, Box::new(door))
            }
            PerfTarget::MutterGnoblin => {
                let launch = MutterGnoblinLaunch::from_environment(environment).map_err(|error| live(target, error))?;
                let names = Names {
                    target: "mutter-gnoblin",
                    log: "gnoblin",
                };
                let session = LauncherSession::start(names, &launch.launcher, &[], directory.path())?;
                let door = RemoteDesktopDoor::new("mutter-gnoblin", directory.path().join(BUS_SOCKET));
                (session, Box::new(door))
            }
        };
        Ok(Self {
            session,
            socket,
            door,
            _directory: directory,
        })
    }
}

/// Runs the whole baseline on `target`.
pub fn run(target: PerfTarget, environment: &RunEnvironment) -> Result<PerfReport, TargetError> {
    let started = Instant::now();
    let mut subject = Measured::start(target, environment)?;
    let startup_seconds = started.elapsed().as_secs_f64();
    let name = target.name();
    let group = subject.session.group();
    let mut peer = WaylandPeer::connect(name, &subject.socket)?;

    let mut idle = Vec::new();
    let mut memory = Vec::new();

    // Nothing is mapped: the compositor's own cost.
    std::thread::sleep(SETTLE);
    idle.push(idle_cpu(group, 0));
    memory.push(memory_point(group, 0));
    let processes = procfs::members(group)
        .into_iter()
        .map(|process| process.comm)
        .collect::<Vec<_>>();

    // One window redrawing as fast as it is let.
    let window = peer.map_toplevel("org.gnoblin.parity.perf", "frames")?;
    let (frame_gaps, frame_count) = measure_frames(&mut peer, &window)?;
    peer.destroy_toplevel(window)?;

    // Input, through the same EIS door the input scenarios use. The EI client comes first: a Mutter seat only
    // advertises a keyboard once an input device exists, and EIS is what makes one.
    let stream = subject.door.open().map_err(|error| subject.session.explain(error))?;
    let mut ei = EiPeer::connect(name, stream)?;
    let eis_capabilities = ei.ready()?;
    peer.enable_input(&eis_capabilities)?;
    let fullscreen = peer.map_fullscreen("org.gnoblin.parity.perf", "input")?;
    if !peer.wait_input(Duration::from_secs(5), |log| log.keyboard_entered)? {
        return Err(TargetError::Live {
            target: name.to_owned(),
            detail: format!(
                "the keyboard never entered the fullscreen window, so key latency cannot be measured (the window is \
                 activated: {}; the EIS peer: {}; input seen: {:?})",
                peer.toplevel_state(&fullscreen).activated,
                ei.describe(),
                peer.input()
            ),
        });
    }
    let (to_client, to_frame, lost) = key_latency(&mut peer, &mut ei, &fullscreen)?;
    drop(ei);
    peer.destroy_toplevel(fullscreen)?;

    // Windows: memory at each count, and the idle cost of the largest set.
    let mut windows: Vec<Toplevel> = Vec::new();
    for count in WINDOW_COUNTS.into_iter().filter(|count| *count > 0) {
        while windows.len() < count {
            windows.push(peer.map_toplevel("org.gnoblin.parity.perf", &format!("window-{}", windows.len()))?);
        }
        peer.roundtrip()?;
        std::thread::sleep(SETTLE);
        memory.push(memory_point(group, count));
        if count == WINDOW_COUNTS[WINDOW_COUNTS.len() - 1] {
            idle.push(idle_cpu(group, count));
        }
    }
    for window in windows {
        peer.destroy_toplevel(window)?;
    }
    drop(peer);
    subject.session.stop();

    Ok(PerfReport {
        target: target.name(),
        host: host(),
        startup_seconds,
        processes,
        idle,
        memory,
        frame_time: LatencyStats::of(&frame_gaps),
        frame_count,
        key_to_client: LatencyStats::of(&to_client),
        key_to_frame: LatencyStats::of(&to_frame),
        keys_lost: lost,
        limits: vec![
            "headless: no panel, so the frame callback is the compositor's own presentation signal, not photons",
            "software rendering (llvmpipe or the candidate's software renderer), not a GPU",
            "the input is EIS injection, which skips libinput and the kernel's input path",
            "Agent37 runs under gVisor, so scheduling and /proc accounting are the sandbox's, not a bare machine's",
            "one run per target: the samples show the spread inside that run, not between runs",
        ],
    })
}

/// Redraws `window` for [`FRAMES`]. Returns the gaps between frame callbacks and how many callbacks fired.
fn measure_frames(peer: &mut WaylandPeer, window: &Toplevel) -> Result<(Vec<Duration>, usize), TargetError> {
    let before = peer.frame_times().len();
    peer.redraw(window)?;
    let deadline = Instant::now() + FRAMES;
    let mut seen = before;
    while Instant::now() < deadline {
        peer.dispatch_for(Duration::from_millis(100))?;
        if peer.frame_times().len() > seen {
            seen = peer.frame_times().len();
            peer.redraw(window)?;
        }
    }
    let fired = &peer.frame_times()[before..];
    let gaps = fired.windows(2).map(|pair| pair[1].duration_since(pair[0])).collect();
    Ok((gaps, fired.len()))
}

/// Presses [`KEYS`] keys and times each one to the client's key event and to the frame callback of the redraw that
/// answers it. Returns both sets of latencies and the number of presses that were lost.
fn key_latency(
    peer: &mut WaylandPeer,
    ei: &mut EiPeer,
    window: &Toplevel,
) -> Result<(Vec<Duration>, Vec<Duration>, usize), TargetError> {
    let mut to_client = Vec::new();
    let mut to_frame = Vec::new();
    let mut lost = 0;
    for _ in 0..KEYS {
        peer.clear_input();
        let sent = Instant::now();
        ei.key(KEY_A, true)?;
        if !wait(peer, LOST, |peer| !peer.input().keys.is_empty())? {
            lost += 1;
            ei.key(KEY_A, false)?;
            continue;
        }
        let received = Instant::now();
        let frames = peer.frame_times().len();
        peer.redraw(window)?;
        if wait(peer, LOST, |peer| peer.frame_times().len() > frames)?
            && let Some(frame) = peer.frame_times().last()
        {
            to_frame.push(frame.duration_since(sent));
        }
        to_client.push(received.duration_since(sent));
        ei.key(KEY_A, false)?;
        wait(peer, LOST, |peer| peer.input().keys.len() >= 2)?;
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok((to_client, to_frame, lost))
}

/// Dispatches events until `done` holds or `limit` passes, without sleeping in steps, and reports whether it held.
fn wait(peer: &mut WaylandPeer, limit: Duration, done: impl Fn(&WaylandPeer) -> bool) -> Result<bool, TargetError> {
    let deadline = Instant::now() + limit;
    loop {
        if done(peer) {
            return Ok(true);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        peer.dispatch_for(left.min(Duration::from_millis(50)))?;
    }
}

/// CPU of every process in the session over [`IDLE`].
fn idle_cpu(group: u32, windows: usize) -> IdleCpu {
    let before = procfs::members(group);
    let started = Instant::now();
    std::thread::sleep(IDLE);
    let after = procfs::members(group);
    let seconds = started.elapsed().as_secs_f64();
    let hz = procfs::ticks_per_second();
    let mut total = 0.0;
    let mut busiest = (0.0, String::from("none"));
    for now in &after {
        // A process that started during the window has no earlier sample and is counted from zero.
        let earlier = before
            .iter()
            .find(|process| process.pid == now.pid)
            .map_or(0, |process| process.ticks);
        let used = now.ticks.saturating_sub(earlier) as f64 / hz;
        total += used;
        if used > busiest.0 {
            busiest = (used, format!("{} ({})", now.comm, now.pid));
        }
    }
    IdleCpu {
        windows,
        seconds,
        percent_of_one_core: total / seconds * 100.0,
        busiest: busiest.1,
    }
}

/// Memory of every process in the session now.
fn memory_point(group: u32, windows: usize) -> MemoryPoint {
    let members = procfs::members(group);
    let metric = if members.iter().all(|process| process.pss_kib.is_some()) {
        "pss"
    } else {
        "rss"
    };
    let figure = |process: &Sample| {
        if metric == "pss" {
            process.pss_kib.unwrap_or(process.rss_kib)
        } else {
            process.rss_kib
        }
    };
    let total_kib = members.iter().map(figure).sum();
    let largest = members.iter().max_by_key(|process| figure(process)).map_or_else(
        || "none".to_owned(),
        |process| format!("{} ({})", process.comm, process.pid),
    );
    MemoryPoint {
        windows,
        metric,
        total_kib,
        largest,
    }
}

fn host() -> HostSpecs {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let cpuinfo = read("/proc/cpuinfo");
    let cpu_model = cpuinfo
        .lines()
        .find_map(|line| line.strip_prefix("model name"))
        .and_then(|rest| rest.split_once(':'))
        .map_or_else(|| "unknown".to_owned(), |(_, model)| model.trim().to_owned());
    let cpus = cpuinfo.lines().filter(|line| line.starts_with("processor")).count();
    let memory_total_kib = read("/proc/meminfo")
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|number| number.parse().ok())
        .unwrap_or(0);
    HostSpecs {
        kernel: read("/proc/sys/kernel/osrelease").trim().to_owned(),
        cpu_model,
        cpus,
        memory_total_kib,
    }
}

/// The text a person reads: one block per measurement.
pub fn render(report: &PerfReport) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    let _ = writeln!(text, "perf {}", report.target);
    let _ = writeln!(
        text,
        "  host        {} cpus, {} MiB, {}, kernel {}",
        report.host.cpus,
        report.host.memory_total_kib / 1024,
        report.host.cpu_model,
        report.host.kernel
    );
    let _ = writeln!(text, "  startup     {:.2} s to a served socket", report.startup_seconds);
    let _ = writeln!(text, "  processes   {}", report.processes.join(" "));
    for idle in &report.idle {
        let _ = writeln!(
            text,
            "  idle cpu    {:>2} windows  {:.2}% of one core over {:.0} s  (busiest {})",
            idle.windows, idle.percent_of_one_core, idle.seconds, idle.busiest
        );
    }
    for point in &report.memory {
        let _ = writeln!(
            text,
            "  memory      {:>2} windows  {:.1} MiB {}  (largest {})",
            point.windows,
            point.total_kib as f64 / 1024.0,
            point.metric,
            point.largest
        );
    }
    let stats = |label: &str, stats: &Option<LatencyStats>, text: &mut String| match stats {
        Some(stats) => {
            let _ = writeln!(
                text,
                "  {label:<11} n={} min {:.2} mean {:.2} p50 {:.2} p95 {:.2} p99 {:.2} max {:.2} ms",
                stats.samples, stats.min_ms, stats.mean_ms, stats.p50_ms, stats.p95_ms, stats.p99_ms, stats.max_ms
            );
        }
        None => {
            let _ = writeln!(text, "  {label:<11} no samples");
        }
    };
    stats("frame time", &report.frame_time, &mut text);
    stats("key->client", &report.key_to_client, &mut text);
    stats("key->frame", &report.key_to_frame, &mut text);
    let _ = writeln!(text, "  keys lost   {} of {KEYS}", report.keys_lost);
    for limit in &report.limits {
        let _ = writeln!(text, "  limit       {limit}");
    }
    text
}

/// The candidate's own command: serve the compositor in this process until it is killed, for [`run`] to measure.
pub fn serve_candidate(directory: &Path, display: &str) -> Result<(), TargetError> {
    candidate::serve(directory, display)
}

/// What `gnoblin-parity perf` was asked for, parsed from its arguments.
pub struct PerfRequest {
    pub target: PerfTarget,
    pub json: Option<PathBuf>,
}

impl PerfRequest {
    /// Parses `<target> [--json FILE]`.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let name = args
            .next()
            .ok_or("perf needs a target: candidate, mutter51 or mutter-gnoblin")?;
        let name = name.to_string_lossy().into_owned();
        let target = PerfTarget::parse(&name).ok_or_else(|| format!("`{name}` is not a perf target"))?;
        let mut json = None;
        while let Some(arg) = args.next() {
            if arg == "--json" {
                json = Some(PathBuf::from(args.next().ok_or("--json needs a file")?));
            } else {
                return Err(format!("unexpected argument `{}`", arg.to_string_lossy()));
            }
        }
        Ok(Self { target, json })
    }
}

//! gnoblin-night-light — warms the screen gamma when Night Light is on.
//!
//! A tiny headless wlr-gamma-control client (no UI). It watches the cross-process
//! flag in [`gnoblin_shell_ui::nightlight`] (toggled by the topbar's control-centre
//! tile) and, while on, sets a warm gamma ramp on every output via
//! `zwlr_gamma_control_v1`. While off it destroys its gamma controls, which
//! restores the original tables and releases the per-output exclusivity (so a
//! user can run wlsunset/gammastep instead). This is also the first real consumer
//! of gnoblin's compositor-side gamma-control protocol.

use gnoblin_shell_ui::nightlight;
use std::error::Error;
use std::io::{Seek, Write};
use std::os::fd::AsFd;
use wayland_client::{
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};

type RuntimeError = Box<dyn Error>;

fn runtime_error(message: impl Into<String>) -> RuntimeError {
    Box::new(std::io::Error::other(message.into()))
}

struct Output {
    reg_name: u32,
    wl: wl_output::WlOutput,
    control: Option<ZwlrGammaControlV1>,
    size: u32,            // 0 until the gamma_size event arrives
    applied: Option<u16>, // temperature last pushed (None = nothing applied yet)
    failed: bool, // gamma rejected (another client owns it) — don't re-create until re-enabled
}

struct State {
    conn: Connection,
    qh: QueueHandle<State>,
    manager: Option<ZwlrGammaControlManagerV1>,
    outputs: Vec<Output>,
    on: bool,
    temp: u16,
}

impl State {
    /// Create a gamma control for each output (when on) or tear them down (when
    /// off, restoring the original tables).
    fn sync_controls(&mut self) {
        let Some(mgr) = self.manager.clone() else {
            return;
        };
        for o in &mut self.outputs {
            if self.on && o.control.is_none() && !o.failed {
                o.control = Some(mgr.get_gamma_control(&o.wl, &self.qh, o.reg_name));
                o.size = 0;
                o.applied = None;
            } else if !self.on {
                if let Some(c) = o.control.take() {
                    c.destroy();
                }
                o.applied = None;
                // Re-enabling night-light retries — whatever owned gamma may be gone.
                o.failed = false;
            }
        }
    }

    /// Push the current temperature to any output whose control is ready and
    /// doesn't already have it applied.
    fn apply_all(&mut self) {
        if !self.on {
            return;
        }
        let temp = self.temp;
        let targets: Vec<(ZwlrGammaControlV1, u32, u32)> = self
            .outputs
            .iter()
            .filter_map(|o| match (&o.control, o.size) {
                (Some(c), s) if s > 0 && o.applied != Some(temp) => {
                    Some((c.clone(), o.reg_name, s))
                }
                _ => None,
            })
            .collect();
        for (control, reg_name, size) in targets {
            if let Some(file) = ramp_file(size, temp) {
                control.set_gamma(file.as_fd());
                // Flush so the fd is transmitted before the File closes.
                let _ = self.conn.flush();
                if let Some(o) = self.outputs.iter_mut().find(|o| o.reg_name == reg_name) {
                    o.applied = Some(temp);
                }
            }
        }
    }
}

/// Build an unlinked file holding the R/G/B gamma ramps (each `size` u16, native
/// byte order — the compositor mmaps it) warmed to `temp` Kelvin.
fn ramp_file(size: u32, temp: u16) -> Option<std::fs::File> {
    let (wr, wg, wb) = whitepoint(temp);
    let n = size as usize;
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 3 * 2);
    for w in [wr, wg, wb] {
        for i in 0..n {
            let frac = if n > 1 {
                i as f64 / (n as f64 - 1.0)
            } else {
                0.0
            };
            let u = ((frac * w).clamp(0.0, 1.0) * 65535.0).round() as u16;
            bytes.extend_from_slice(&u.to_ne_bytes());
        }
    }

    let dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let path = format!("{dir}/gnoblin-gamma-{}", std::process::id());
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .ok()?;
    let _ = std::fs::remove_file(&path); // unlink; the fd stays valid
    f.write_all(&bytes).ok()?;
    f.rewind().ok()?;
    Some(f)
}

/// Approximate white point (0..1 per channel) for a colour temperature, via
/// Tanner Helland's well-known black-body approximation.
fn whitepoint(kelvin: u16) -> (f64, f64, f64) {
    let t = kelvin as f64 / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };
    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    let c = |v: f64| (v / 255.0).clamp(0.0, 1.0);
    (c(r), c(g), c(b))
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == ZwlrGammaControlManagerV1::interface().name {
                    state.manager = Some(registry.bind(name, version.min(1), qh, ()));
                    state.sync_controls();
                } else if interface == wl_output::WlOutput::interface().name {
                    let wl = registry.bind(name, version.min(4), qh, ());
                    state.outputs.push(Output {
                        reg_name: name,
                        wl,
                        control: None,
                        size: 0,
                        applied: None,
                        failed: false,
                    });
                    state.sync_controls();
                }
                let _ = state.conn.flush();
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(pos) = state.outputs.iter().position(|o| o.reg_name == name) {
                    let o = state.outputs.remove(pos);
                    if let Some(c) = o.control {
                        c.destroy();
                    }
                }
            }
            _ => {}
        }
    }
}

// We don't need wl_output or manager events; the proxies are all we use.
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrGammaControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrGammaControlManagerV1,
        _: <ZwlrGammaControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        reg_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(o) = state.outputs.iter_mut().find(|o| o.reg_name == *reg_name) {
                    o.size = size;
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                // Another client owns this output's gamma (wlsunset/gammastep), or
                // it's unsupported. Mark it failed so sync_controls stops re-creating
                // (and re-failing, re-logging) it until night-light is toggled off→on.
                if let Some(o) = state.outputs.iter_mut().find(|o| o.reg_name == *reg_name) {
                    o.control = None;
                    o.size = 0;
                    o.applied = None;
                    o.failed = true;
                }
                eprintln!("gnoblin-night-light: gamma control failed for one output");
            }
            _ => {}
        }
    }
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("gnoblin-night-light: {e}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), RuntimeError> {
    let conn = Connection::connect_to_env()
        .map_err(|e| runtime_error(format!("connect to Wayland: {e}")))?;
    let display = conn.display();
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let _registry = display.get_registry(&qh, ());

    let mut state = State {
        conn: conn.clone(),
        qh: qh.clone(),
        manager: None,
        outputs: Vec::new(),
        on: false,
        temp: nightlight::DEFAULT_TEMP,
    };

    // Bind the manager + outputs.
    queue
        .roundtrip(&mut state)
        .map_err(|e| runtime_error(format!("registry roundtrip: {e}")))?;
    if state.manager.is_none() {
        eprintln!("gnoblin-night-light: compositor doesn't advertise wlr-gamma-control; exiting");
        return Ok(());
    }

    loop {
        let on = nightlight::is_on();
        let temp = nightlight::temperature();
        if on != state.on || (on && temp != state.temp) {
            state.on = on;
            state.temp = temp;
            state.sync_controls();
            let _ = conn.flush();
        }
        // Pump events (gamma_size for new controls, hotplug, failures), then
        // apply to any output that's now ready.
        if queue.roundtrip(&mut state).is_err() {
            break;
        }
        state.apply_all();
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
    Ok(())
}

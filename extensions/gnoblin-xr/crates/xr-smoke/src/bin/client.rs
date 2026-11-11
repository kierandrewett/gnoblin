//! `xr-smoke-client`: a solid-colour Wayland window that prints what its seats' pointers do, for the XR smoke script.
//!
//! ```text
//! xr-smoke-client --app-id smoke-red --colour ff0000 [--size 400x300]
//! ```
//!
//! It maps one `xdg_toplevel` filled with one opaque colour, fills whatever size the compositor configures, and
//! binds the pointer of every `wl_seat` global, including one that appears after it started. Each event is one line
//! on stdout, which is line-buffered, so a script can wait for a line:
//!
//! ```text
//! seat 7 name=xr
//! configure 400x300
//! mapped app_id=smoke-red colour=ff0000 400x300
//! pointer-enter seat=xr x=200.0 y=150.0
//! pointer-motion seat=xr x=200.0 y=150.0
//! pointer-button seat=xr button=272 state=pressed
//! pointer-leave seat=xr
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// What the command line asked for.
struct Options {
    app_id: String,
    colour: [u8; 3],
    width: u32,
    height: u32,
}

fn parse() -> Result<Options, String> {
    let mut options = Options {
        app_id: "xr-smoke".to_owned(),
        colour: [0xff, 0x00, 0x00],
        width: 400,
        height: 300,
    };
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let value = args.next().ok_or_else(|| format!("{argument} needs a value"))?;
        match argument.as_str() {
            "--app-id" => options.app_id = value,
            "--colour" => {
                let bits = u32::from_str_radix(&value, 16).map_err(|_| format!("`{value}` is not RRGGBB"))?;
                options.colour = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
            }
            "--size" => {
                let (w, h) = value.split_once('x').ok_or_else(|| format!("`{value}` is not WxH"))?;
                options.width = w.parse().map_err(|_| format!("`{value}` is not WxH"))?;
                options.height = h.parse().map_err(|_| format!("`{value}` is not WxH"))?;
            }
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }
    Ok(options)
}

struct App {
    options: Options,
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    /// The size the compositor configured, or 0 for "your choice".
    configured: (u32, u32),
    /// The size of the buffer last attached.
    attached: Option<(u32, u32)>,
    /// Seat names by global name, as `wl_seat.name` announced them.
    seat_names: BTreeMap<u32, String>,
    /// The pointers bound, by seat global name, so a removed seat drops its pointer.
    pointers: BTreeMap<u32, wl_pointer::WlPointer>,
    mapped: bool,
    closed: bool,
}

impl App {
    fn seat_label(&self, global: u32) -> String {
        self.seat_names
            .get(&global)
            .cloned()
            .unwrap_or_else(|| format!("global{global}"))
    }

    /// Fills a buffer of the configured size with the colour and commits it.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let (width, height) = match self.configured {
            (0, _) | (_, 0) => (self.options.width, self.options.height),
            size => size,
        };
        let stride = width * 4;
        let size = u64::from(stride) * u64::from(height);
        let fd = match memfd_create("xr-smoke-client", MemfdFlags::CLOEXEC) {
            Ok(fd) => fd,
            Err(error) => {
                eprintln!("xr-smoke-client: memfd: {error}");
                self.closed = true;
                return;
            }
        };
        let file = File::from(fd);
        let [r, g, b] = self.options.colour;
        let row: Vec<u8> = (0..width).flat_map(|_| [b, g, r, 0xff]).collect();
        let mut bytes = Vec::with_capacity(size as usize);
        for _ in 0..height {
            bytes.extend_from_slice(&row);
        }
        if let Err(error) = file.write_all_at(&bytes, 0) {
            eprintln!("xr-smoke-client: write: {error}");
            self.closed = true;
            return;
        }
        let pool = self.shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Xrgb8888,
            qh,
            (),
        );
        pool.destroy();
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width as i32, height as i32);
        self.surface.commit();
        self.attached = Some((width, height));
        if !self.mapped {
            self.mapped = true;
            let [r, g, b] = self.options.colour;
            println!(
                "mapped app_id={} colour={r:02x}{g:02x}{b:02x} {width}x{height}",
                self.options.app_id
            );
        }
    }
}

fn main() -> std::process::ExitCode {
    let options = match parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("xr-smoke-client: {message}");
            eprintln!("usage: xr-smoke-client [--app-id ID] [--colour RRGGBB] [--size WxH]");
            return std::process::ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xr-smoke-client: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| format!("connecting: {error}"))?;
    let (globals, mut queue) =
        registry_queue_init::<App>(&connection).map_err(|error| format!("reading globals: {error}"))?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=4, ()).map_err(|e| e.to_string())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
    let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_app_id(options.app_id.clone());
    toplevel.set_title(options.app_id.clone());
    let mut app = App {
        options,
        shm,
        surface,
        configured: (0, 0),
        attached: None,
        seat_names: BTreeMap::new(),
        pointers: BTreeMap::new(),
        mapped: false,
        closed: false,
    };
    // The seats that were there before the first event, bound the way a later one is.
    let registry = globals.registry();
    let seats: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_seat")
            .map(|global| (global.name, global.version))
            .collect()
    });
    for (name, version) in seats {
        registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), &qh, name);
    }
    app.surface.commit();
    while !app.closed {
        queue
            .blocking_dispatch(&mut app)
            .map_err(|error| format!("dispatching: {error}"))?;
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        app: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => {
                registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), qh, name);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(seat) = app.seat_names.remove(&name) {
                    println!("seat-removed name={seat}");
                }
                app.pointers.remove(&name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, u32> for App {
    fn event(
        app: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        global: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_seat::Event::Name { name } => {
                println!("seat {global} name={name}");
                app.seat_names.insert(*global, name);
            }
            wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(capabilities),
            } if capabilities.contains(wl_seat::Capability::Pointer) && !app.pointers.contains_key(global) => {
                app.pointers.insert(*global, seat.get_pointer(qh, *global));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, u32> for App {
    fn event(
        app: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let seat = app.seat_label(*global);
        match event {
            wl_pointer::Event::Enter {
                surface_x, surface_y, ..
            } => println!("pointer-enter seat={seat} x={surface_x:.1} y={surface_y:.1}"),
            wl_pointer::Event::Leave { .. } => println!("pointer-leave seat={seat}"),
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => println!("pointer-motion seat={seat} x={surface_x:.1} y={surface_y:.1}"),
            wl_pointer::Event::Button { button, state, .. } => println!(
                "pointer-button seat={seat} button={button} state={}",
                match state {
                    WEnum::Value(wl_pointer::ButtonState::Pressed) => "pressed",
                    _ => "released",
                }
            ),
            wl_pointer::Event::Axis { axis, value, .. } => {
                println!("pointer-axis seat={seat} axis={axis:?} value={value:.1}");
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        app: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            let wanted = match app.configured {
                (0, _) | (_, 0) => (app.options.width, app.options.height),
                size => size,
            };
            if app.attached != Some(wanted) {
                app.draw(qh);
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        app: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                app.configured = (width.max(0) as u32, height.max(0) as u32);
                println!("configure {width}x{height}");
            }
            xdg_toplevel::Event::Close => {
                println!("closed");
                app.closed = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(App: ignore wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
delegate_noop!(App: ignore wl_surface::WlSurface);

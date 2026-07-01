//! gnoblin-menu - the dock app context menu as a resident layer-shell daemon.
//!
//! The dock sends per-open app state and geometry over D-Bus. The Slint
//! ContextMenu component and EGL/renderer pipeline stay warm in this process;
//! each Show maps a fresh content-sized wl_surface and rebinds the warm adapter.

use gnoblin_core::RuntimeError;
use gnoblin_runtime::{
    app_context_menu, run, run_daemon_with_runtime, BarApp, BarConfig, BarMargins, PopoutConfig,
    PopoutHandle, RuntimeControl,
};
use slint::platform::Key;
use slint::{ComponentHandle, SharedString};
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

slint::include_modules!(); // GnoblinMenu, MenuItem

const BUS_NAME: &str = "dev.gnoblin.Menu";
const BUS_PATH: &str = "/dev/gnoblin/Menu";
const SCRIM_NAMESPACE: &str = "gnoblin-menu-scrim";
const MENU_W: u32 = 220;
const EDGE_INSET: i32 = 8;
const DEFAULT_BOTTOM_MARGIN: i32 = 102;
const DOCK_GAP: i32 = 6;
const DEFAULT_SCREEN_W: i32 = 1280;
const DEFAULT_SCREEN_H: i32 = 800;

#[derive(Clone, Debug)]
struct MenuArgs {
    app_id: String,
    running: bool,
    pinned: bool,
    anchor_x: i32,
    screen_w: i32,
    screen_h: i32,
    bottom_margin: i32,
}

impl Default for MenuArgs {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            running: false,
            pinned: false,
            anchor_x: MENU_W as i32 / 2,
            screen_w: DEFAULT_SCREEN_W,
            screen_h: DEFAULT_SCREEN_H,
            bottom_margin: DEFAULT_BOTTOM_MARGIN,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct MenuCli {
    daemon: bool,
    params: Option<MenuArgs>,
}

impl MenuArgs {
    fn from_env() -> MenuCli {
        let mut args = Self::default();
        let mut daemon = false;
        let mut it = std::env::args().skip(1).peekable();

        while let Some(tok) = it.next() {
            let (key, inline) = match tok.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (tok, None),
            };
            let mut value = || {
                if inline.is_some() {
                    return inline.clone();
                }
                match it.peek() {
                    Some(next) if !next.starts_with("--") => it.next(),
                    _ => None,
                }
            };

            match key.as_str() {
                "--daemon" => daemon = true,
                "--app-id" => args.app_id = value().unwrap_or_default(),
                "--running" => args.running = value().as_deref().is_some_and(parse_bool),
                "--pinned" => args.pinned = value().as_deref().is_some_and(parse_bool),
                "--anchor-x" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.anchor_x = v;
                    }
                }
                "--screen-width" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.screen_w = v;
                    }
                }
                "--screen-height" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.screen_h = v;
                    }
                }
                "--bottom-margin" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.bottom_margin = v;
                    }
                }
                "--band-height" => {
                    if let Some(v) = value().and_then(|v: String| v.parse::<i32>().ok()) {
                        args.bottom_margin = v + DOCK_GAP;
                    }
                }
                _ => {
                    let _ = value();
                }
            }
        }

        MenuCli {
            daemon,
            params: if args.app_id.is_empty() {
                None
            } else {
                Some(args)
            },
        }
    }

    fn left_margin(&self) -> i32 {
        let raw = self.anchor_x - MENU_W as i32 / 2;
        let max_left = if self.screen_w > 0 {
            (self.screen_w - MENU_W as i32 - EDGE_INSET).max(EDGE_INSET)
        } else {
            raw.max(EDGE_INSET)
        };
        raw.clamp(EDGE_INSET, max_left)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn menu_height(items: &[MenuItem]) -> u32 {
    let mut total = 12;
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            total += 2;
        }
        total += if item.separator { 9 } else { 30 };
    }
    total.max(1)
}

fn menu_items(app_id: &str, running: bool, pinned: bool) -> Vec<MenuItem> {
    app_context_menu::build(app_id, running, pinned)
        .into_iter()
        .map(|it| MenuItem {
            id: it.id,
            label: it.label.into(),
            accelerator: Default::default(),
            separator: it.separator,
            submenu: it.submenu,
            enabled: it.enabled,
        })
        .collect()
}

fn menu_config(args: &MenuArgs) -> BarConfig {
    let items = menu_items(&args.app_id, args.running, args.pinned);
    BarConfig {
        namespace: "gnoblin-menu",
        anchor: Anchor::BOTTOM.union(Anchor::LEFT),
        layer: Layer::Overlay,
        width: MENU_W,
        height: menu_height(&items),
        margins: BarMargins {
            left: args.left_margin(),
            bottom: args.bottom_margin.max(0),
            ..BarMargins::default()
        },
        exclusive_zone: 0,
        full_height: false,
        input_passthrough: false,
        keyboard: true,
    }
}

fn scrim_config(screen_w: i32, screen_h: i32) -> PopoutConfig {
    PopoutConfig {
        namespace: SCRIM_NAMESPACE,
        anchor: Anchor::TOP.union(Anchor::LEFT),
        // Open before the menu's primary surface so same-layer stacking puts the
        // transparent input surface under the menu but above the rest of chrome.
        layer: Layer::Overlay,
        width: screen_w.max(1) as u32,
        height: screen_h.max(1) as u32,
        margins: BarMargins::default(),
        keyboard: true,
    }
}

fn close_delay() -> Duration {
    let motion = gnoblin_runtime::prefs::shell_motion();
    let millis = (motion.overlay_close_ms * motion.scale).max(0.0).ceil() as u64;
    Duration::from_millis(millis)
}

fn apply_theme(menu: &GnoblinMenu) {
    gnoblin_runtime::apply_shell_theme!(menu);
}

struct MenuBus {
    runtime: RuntimeControl,
    params: Arc<Mutex<MenuArgs>>,
}

#[zbus::interface(name = "dev.gnoblin.Menu")]
impl MenuBus {
    fn show(
        &self,
        app_id: String,
        running: bool,
        pinned: bool,
        anchor_x: i32,
        screen_width: i32,
        screen_height: i32,
        bottom_margin: i32,
    ) {
        if app_id.is_empty() {
            return;
        }
        let params = MenuArgs {
            app_id,
            running,
            pinned,
            anchor_x,
            screen_w: screen_width,
            screen_h: screen_height,
            bottom_margin,
        };
        if let Ok(mut locked) = self.params.lock() {
            *locked = params.clone();
        }
        self.runtime.hide_primary();
        self.runtime
            .open_popout(scrim_config(params.screen_w, params.screen_h));
        self.runtime.show_primary("show");
    }
}

#[zbus::proxy(
    interface = "dev.gnoblin.Menu",
    default_service = "dev.gnoblin.Menu",
    default_path = "/dev/gnoblin/Menu"
)]
trait MenuDaemon {
    fn show(
        &self,
        app_id: &str,
        running: bool,
        pinned: bool,
        anchor_x: i32,
        screen_width: i32,
        screen_height: i32,
        bottom_margin: i32,
    ) -> zbus::Result<()>;
}

fn own_menu_bus(
    runtime: RuntimeControl,
    params: Arc<Mutex<MenuArgs>>,
) -> zbus::Result<zbus::Connection> {
    zbus::block_on(async move {
        let builder = zbus::connection::Builder::session()?
            .serve_at(BUS_PATH, MenuBus { runtime, params })?
            .name(BUS_NAME)?
            .allow_name_replacements(false)
            .replace_existing_names(false);
        builder.build().await
    })
}

fn trigger_existing_daemon(args: &MenuArgs) -> zbus::Result<()> {
    let args = args.clone();
    zbus::block_on(async move {
        let conn = zbus::Connection::session().await?;
        let proxy = MenuDaemonProxy::new(&conn).await?;
        proxy
            .show(
                &args.app_id,
                args.running,
                args.pinned,
                args.anchor_x,
                args.screen_w,
                args.screen_h,
                args.bottom_margin,
            )
            .await
    })
}

struct MenuApp {
    menu: Option<GnoblinMenu>,
    scrim: Option<MenuScrimWindow>,
    shared_args: Arc<Mutex<MenuArgs>>,
    current: MenuArgs,
    current_app_id: Rc<RefCell<String>>,
    exit: Rc<Cell<bool>>,
    dismiss_requested: Rc<Cell<bool>>,
    runtime: Rc<RefCell<Option<RuntimeControl>>>,
    scrim_handle: Rc<Cell<Option<PopoutHandle>>>,
    resident: bool,
    pending_open: bool,
    hide_after: Option<Instant>,
    screen_w: u32,
    screen_h: u32,
}

impl MenuApp {
    fn new(shared_args: Arc<Mutex<MenuArgs>>, resident: bool) -> Self {
        let current = shared_args
            .lock()
            .map(|args| args.clone())
            .unwrap_or_default();
        Self {
            menu: None,
            scrim: None,
            shared_args,
            current_app_id: Rc::new(RefCell::new(current.app_id.clone())),
            current,
            exit: Rc::new(Cell::new(false)),
            dismiss_requested: Rc::new(Cell::new(false)),
            runtime: Rc::new(RefCell::new(None)),
            scrim_handle: Rc::new(Cell::new(None)),
            resident,
            pending_open: false,
            hide_after: None,
            screen_w: DEFAULT_SCREEN_W as u32,
            screen_h: DEFAULT_SCREEN_H as u32,
        }
    }

    fn latest_args(&self) -> MenuArgs {
        self.shared_args
            .lock()
            .map(|args| args.clone())
            .unwrap_or_else(|_| self.current.clone())
    }

    fn set_menu_open(&self, open: bool) {
        if let Some(menu) = &self.menu {
            menu.set_open(open);
        }
    }

    fn apply_args(&mut self, args: MenuArgs) {
        let items = menu_items(&args.app_id, args.running, args.pinned);
        *self.current_app_id.borrow_mut() = args.app_id.clone();
        self.current = args;
        if let Some(menu) = &self.menu {
            menu.set_items(Rc::new(slint::VecModel::from(items)).into());
        }
    }

    fn close_scrim(&mut self) {
        if let Some(handle) = self.scrim_handle.take() {
            if let Some(runtime) = self.runtime.borrow().as_ref().cloned() {
                runtime.close_popout(handle);
            }
        }
        self.scrim = None;
    }

    fn begin_show(&mut self) -> BarConfig {
        let args = self.latest_args();
        self.hide_after = None;
        self.dismiss_requested.set(false);
        self.pending_open = true;
        self.set_menu_open(false);
        self.apply_args(args);
        menu_config(&self.current)
    }

    fn dismiss(&mut self) {
        if self.hide_after.is_some() {
            return;
        }
        self.pending_open = false;
        self.dismiss_requested.set(false);
        self.set_menu_open(false);
        let delay = close_delay();
        if delay.is_zero() {
            self.finish_hide();
        } else {
            self.hide_after = Some(Instant::now() + delay);
        }
    }

    fn finish_hide(&mut self) {
        self.hide_after = None;
        if self.resident {
            if let Some(runtime) = self.runtime.borrow().as_ref().cloned() {
                runtime.hide_primary();
                return;
            }
        }
        self.exit.set(true);
    }
}

impl BarApp for MenuApp {
    fn set_runtime(&mut self, runtime: RuntimeControl) {
        *self.runtime.borrow_mut() = Some(runtime);
    }

    fn primary_config_for_mode(&mut self, _mode: &str, _config: BarConfig) -> BarConfig {
        self.begin_show()
    }

    fn primary_hidden(&mut self) {
        self.pending_open = false;
        self.hide_after = None;
        self.dismiss_requested.set(false);
        self.set_menu_open(false);
        self.close_scrim();
    }

    fn show(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        self.screen_w = screen_w;
        self.screen_h = screen_h;
        let menu = GnoblinMenu::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("GnoblinMenu::new: {e}")))?;
        apply_theme(&menu);
        gnoblin_runtime::apply_shell_motion_to_theme!(
            menu.global::<Theme>(),
            gnoblin_runtime::prefs::shell_motion()
        );
        menu.set_open(false);

        {
            let app_id = self.current_app_id.clone();
            let dismiss = self.dismiss_requested.clone();
            menu.on_item_activated(move |id| {
                let app_id = app_id.borrow().clone();
                let _ = app_context_menu::activate(&app_id, id);
                dismiss.set(true);
            });
        }
        {
            let dismiss = self.dismiss_requested.clone();
            menu.on_dismiss(move || dismiss.set(true));
        }

        menu.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("menu show: {e}")))?;
        self.menu = Some(menu);
        self.apply_args(self.current.clone());
        if !self.resident {
            self.set_menu_open(true);
        }
        Ok(())
    }

    fn show_popout(
        &mut self,
        handle: PopoutHandle,
        namespace: &'static str,
        _width: u32,
        _height: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        if namespace != SCRIM_NAMESPACE {
            return Err(gnoblin_core::runtime_error(format!(
                "unknown menu popout namespace: {namespace}"
            )));
        }
        let scrim = MenuScrimWindow::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("MenuScrimWindow::new: {e}")))?;
        let dismiss = self.dismiss_requested.clone();
        scrim.on_pressed(move || dismiss.set(true));
        scrim
            .show()
            .map_err(|e| gnoblin_core::runtime_error(format!("scrim.show: {e}")))?;
        self.scrim_handle.set(Some(handle));
        self.scrim = Some(scrim);
        Ok(())
    }

    fn popout_window(&self, handle: PopoutHandle) -> Option<&slint::Window> {
        if self.scrim_handle.get() == Some(handle) {
            return self.scrim.as_ref().map(|s| s.window());
        }
        None
    }

    fn popout_closed(&mut self, handle: PopoutHandle, namespace: &'static str) {
        if self.scrim_handle.get() == Some(handle) || namespace == SCRIM_NAMESPACE {
            self.scrim_handle.set(None);
            self.scrim = None;
        }
    }

    fn resized(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        self.screen_w = screen_w;
        self.screen_h = screen_h;
        if self.pending_open {
            self.pending_open = false;
            self.set_menu_open(true);
        }
    }

    fn tick(&mut self) -> bool {
        if self.dismiss_requested.replace(false) {
            self.dismiss();
            return true;
        }
        if self
            .hide_after
            .map(|deadline| Instant::now() >= deadline)
            .unwrap_or(false)
        {
            self.finish_hide();
            return true;
        }
        false
    }

    fn window(&self) -> Option<&slint::Window> {
        self.menu.as_ref().map(|m| m.window())
    }

    fn should_exit(&self) -> bool {
        self.exit.get()
    }

    fn key_pressed(&mut self, text: &SharedString) {
        if text.chars().next() == Some(char::from(Key::Escape)) {
            self.dismiss();
        }
    }
}

fn main() {
    let cli = MenuArgs::from_env();
    if cli.params.is_none() && !cli.daemon {
        eprintln!("gnoblin-menu: missing --app-id");
        return;
    }

    let initial_args = cli.params.clone();
    let shared_args = Arc::new(Mutex::new(initial_args.clone().unwrap_or_default()));
    let runtime = match RuntimeControl::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("gnoblin-menu: {e}");
            return;
        }
    };

    let bus_conn = match own_menu_bus(runtime.clone(), shared_args.clone()) {
        Ok(conn) => Some(conn),
        Err(zbus::Error::NameTaken) => {
            if let Some(args) = &initial_args {
                if let Err(e) = trigger_existing_daemon(args) {
                    eprintln!("gnoblin-menu: trigger failed: {e}");
                }
            }
            return;
        }
        Err(e) => {
            eprintln!("gnoblin-menu: D-Bus unavailable ({e}); running one-shot fallback");
            if cli.daemon {
                return;
            }
            None
        }
    };

    if bus_conn.is_some() {
        if let Some(args) = &initial_args {
            runtime.open_popout(scrim_config(args.screen_w, args.screen_h));
        }
        let base_config = shared_args
            .lock()
            .map(|args| menu_config(&args))
            .unwrap_or_else(|_| menu_config(&MenuArgs::default()));
        run_daemon_with_runtime(
            base_config,
            Box::new(MenuApp::new(shared_args, true)),
            runtime,
            if initial_args.is_some() {
                Some("show")
            } else {
                None
            },
        );
    } else if let Some(args) = initial_args {
        run(
            menu_config(&args),
            Box::new(MenuApp::new(shared_args, false)),
        );
    }
}

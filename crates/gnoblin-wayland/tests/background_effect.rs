//! `ext_background_effect_manager_v1` against a real client.
//!
//! The scenarios are the rules in `src/protocols/ext_background_effect_v1.rs`: the
//! capability the bind announces, a region that is staged until `wl_surface.commit`,
//! the unset that clears it, and a region that never reaches another surface.

mod owned;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_types::{Rect, WindowId};
use gnoblin_wayland::ObservedBlurRect;
use owned::{Connected, Handle, Session, TIMEOUT};
use wayland_client::protocol::{wl_buffer, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The `blur` bit of the `capability` enum the manager announces.
const BLUR: u32 = 1;

#[derive(Default)]
struct App {
    /// The flags of the last `capabilities` event, if one arrived.
    capabilities: Option<u32>,
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _manager: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.capabilities = Some(match flags {
                WEnum::Value(flags) => flags.bits(),
                WEnum::Unknown(bits) => bits,
            });
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        _state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_region::WlRegion);
wayland_client::delegate_noop!(App: ignore ExtBackgroundEffectSurfaceV1);

/// One connection with a mapped window and the background-effect manager bound.
struct Blurred {
    client: Connected<App>,
    surface: wl_surface::WlSurface,
    effect: ExtBackgroundEffectSurfaceV1,
}

impl Blurred {
    fn start(handle: &Handle) -> Self {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager: ExtBackgroundEffectManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("every client may describe its own surface");
        let wm_base: xdg_wm_base::XdgWmBase = client.globals.bind(&client.qh, 1..=5, ()).expect("xdg_wm_base");
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg.get_toplevel(&client.qh, ());
        toplevel.set_title("blurred".to_owned());
        surface.commit();
        client.roundtrip();
        let buffer = client.buffer(200, 150);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 200, 150);
        surface.commit();
        client.roundtrip();
        let effect = manager.get_background_effect(&surface, &client.qh, ());
        Self {
            client,
            surface,
            effect,
        }
    }

    /// A region made of the given rectangles, each `(x, y, width, height)`, added in
    /// order.
    fn region(&self, rects: &[(i32, i32, i32, i32)]) -> wl_region::WlRegion {
        let region = self.client.compositor.create_region(&self.client.qh, ());
        for &(x, y, width, height) in rects {
            region.add(x, y, width, height);
        }
        region
    }

    /// Stages a region on the surface; it applies on the next commit.
    fn stage(&mut self, region: Option<&wl_region::WlRegion>) {
        self.effect.set_blur_region(region);
        self.client.roundtrip();
    }

    /// Commits the surface, which applies whatever region was staged.
    fn commit(&mut self) {
        self.surface.commit();
        self.client.roundtrip();
    }
}

/// The rectangle a `(x, y, width, height)` tuple names.
fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
    Rect { x, y, width, height }
}

/// The committed region of a window, read through the control seam.
fn committed(handle: &Handle, window: WindowId) -> Option<Option<Vec<ObservedBlurRect>>> {
    handle.control.blur_region(window)
}

/// Waits until the committed region of `window` satisfies `predicate`.
fn wait_region(
    handle: &Handle,
    window: WindowId,
    what: &str,
    predicate: impl Fn(&Option<Vec<ObservedBlurRect>>) -> bool,
) -> Option<Vec<ObservedBlurRect>> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(region) = committed(handle, window)
            && predicate(&region)
        {
            return region;
        }
        assert!(Instant::now() < deadline, "the renderer never saw {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// The first window a test's clients map; identities are handed out in creation order.
fn first() -> WindowId {
    WindowId::new(1)
}

/// The second window a test's clients map.
fn second() -> WindowId {
    WindowId::new(2)
}

#[test]
fn the_manager_announces_the_blur_capability_when_it_is_bound() {
    let session = Session::start(None);
    session.run(|handle| {
        let client = Blurred::start(handle);
        assert_eq!(
            client.client.state.capabilities,
            Some(BLUR),
            "the bind announces blur and nothing else"
        );
    });
}

#[test]
fn a_region_applies_on_commit_and_reaches_the_renderer_in_order() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut client = Blurred::start(handle);
        assert_eq!(committed(handle, first()), Some(None), "no region before one is set");

        let region = client.region(&[(10, 10, 50, 40)]);
        client.stage(Some(&region));
        assert_eq!(
            committed(handle, first()),
            Some(None),
            "a staged region is not visible before commit"
        );

        client.commit();
        let applied = wait_region(handle, first(), "the committed region", |region| region.is_some());
        assert_eq!(
            applied,
            Some(vec![ObservedBlurRect::Add(rect(10, 10, 50, 40))]),
            "the renderer reads the rectangle the client sent"
        );
    });
}

#[test]
fn an_unset_clears_the_committed_region_on_commit() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut client = Blurred::start(handle);
        let region = client.region(&[(0, 0, 20, 20)]);
        client.stage(Some(&region));
        client.commit();
        wait_region(handle, first(), "the set region", |region| region.is_some());

        client.stage(None);
        assert!(
            committed(handle, first()).is_some_and(|region| region.is_some()),
            "an unset is staged too, so the old region holds until commit"
        );
        client.commit();
        wait_region(handle, first(), "the cleared region", |region| region.is_none());
    });
}

#[test]
fn subtracted_rectangles_keep_their_kind() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut client = Blurred::start(handle);
        // A `subtract` on the wire comes back as `Subtract` in the same position.
        let region = client.region(&[(0, 0, 100, 100)]);
        region.subtract(20, 20, 5, 5);
        client.stage(Some(&region));
        client.commit();
        let applied = wait_region(handle, first(), "the region with a cut", |region| region.is_some());
        assert_eq!(
            applied,
            Some(vec![
                ObservedBlurRect::Add(rect(0, 0, 100, 100)),
                ObservedBlurRect::Subtract(rect(20, 20, 5, 5)),
            ]),
        );
    });
}

#[test]
fn a_region_never_reaches_another_surface() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut first_client = Blurred::start(handle);
        let mut second_client = Blurred::start(handle);

        let first_region = first_client.region(&[(1, 1, 10, 10)]);
        first_client.stage(Some(&first_region));
        first_client.commit();
        wait_region(handle, first(), "the first client's region", |region| region.is_some());

        // The second window is still unblurred while the first has a region.
        assert_eq!(
            committed(handle, second()),
            Some(None),
            "a region set on one surface does not blur another"
        );

        let second_region = second_client.region(&[(2, 2, 30, 30)]);
        second_client.stage(Some(&second_region));
        second_client.commit();
        let second_applied = wait_region(handle, second(), "the second client's region", |region| {
            region.is_some()
        });
        assert_eq!(second_applied, Some(vec![ObservedBlurRect::Add(rect(2, 2, 30, 30))]));

        // Each window keeps its own rectangle once both are set.
        assert_eq!(
            committed(handle, first()),
            Some(Some(vec![ObservedBlurRect::Add(rect(1, 1, 10, 10))])),
            "the first window still reads its own region"
        );
    });
}

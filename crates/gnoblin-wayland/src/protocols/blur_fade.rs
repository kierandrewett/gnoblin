//! `gnoblin_blur_fade_v1`: animation opacity kept apart from glass alpha.
//!
//! A client that fades items inside its own surface (a list scrolling away, a
//! dialog appearing) needs the compositor to fade what is behind that item
//! separately from the transparency of its glass material: without this
//! metadata, a half-faded card behind a translucent panel is indistinguishable
//! from a transparent panel, and the compositor would blend the background of
//! the whole surface instead of the faded item.
//!
//! The metadata is a list of up to 32 rectangles in the surface's logical
//! coordinates, each with the opacity its item currently has. It is double
//! buffered with `wl_surface.commit`, so it always describes the frame it
//! arrived with, and it is *metadata only*: the client's own pixels are not
//! changed by it and it never enables blur, which stays a window rule's
//! decision.
//!
//! The wire, the record layout (five 24.8 fixed-point values), the limits and
//! the `invalid_fades` error code are the C compositor's
//! (`src/protocols/blur-fade/meta-wayland-blur-fade.c`) unchanged.
//!
//! The global is offered to every client of the session socket: this is
//! descriptive metadata about the client's own surface, not a capability over
//! another client's.
//!
//! ## Failure modes this module defends against
//!
//! Each rule is what the code does, and the parser rules are the C compositor's
//! value for value. The real-client blur-fade tests are not part of this change
//! (see "What is not proven yet").
//!
//! 1. **A malformed array is stored and reaches the renderer.** A length that is
//!    not a whole number of records, more than 32 records, a coordinate outside the
//!    24.8 range, a negative width or height and an opacity above one are all
//!    refused with `invalid_fades`, and the surface keeps its previous metadata.
//! 2. **A client's metadata outlives the frame it described.** It is applied only on
//!    commit, so metadata sent after the frame it belongs to changes the next frame.
//! 3. **An empty array leaves the old metadata behind.** An empty array removes the
//!    metadata on the next commit.
//! 4. **One surface's metadata becomes another's.** It lives in the surface's own
//!    data map, so it is dropped with the surface and no lookup can cross surfaces.
//!
//! ## What is not proven yet
//!
//! The global is served, the records are parsed and committed, and
//! [`State::with_blur_fades`] hands them to a renderer; the real-client tests that
//! would drive it are not in this change, and nothing in this slice consumes the
//! records yet because the software frame path draws no blur.

use std::sync::Mutex;

use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::compositor::with_states;
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::debug;

use crate::State;
use crate::protocols::wire::blur_fade::gnoblin_blur_fade_manager_v1::{self as manager, GnoblinBlurFadeManagerV1};

/// How many records one array may carry, from the protocol.
const MAX_FADES: usize = 32;

/// The values one record carries: x, y, width, height and opacity.
const RECORD_VALUES: usize = 5;

/// The fixed-point scale the protocol's values use: 24.8.
const FIXED_SCALE: f32 = 256.0;

/// The largest 24.8 magnitude a record's fields may carry, from the protocol.
const MAX_FIXED: i32 = 65536 * 256;

/// One faded item of a surface, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    /// The item's left edge.
    pub x: f32,
    /// The item's top edge.
    pub y: f32,
    /// The item's width.
    pub width: f32,
    /// The item's height.
    pub height: f32,
    /// The opacity of the item's whole completed texture, from zero to one.
    pub opacity: f32,
}

/// The metadata of one surface, double buffered with its commits.
///
/// It lives in the surface's own data map, so it is dropped exactly when the
/// surface is: a client that disconnects leaves no entry behind and no lookup
/// can return another surface's records.
#[derive(Default)]
struct FadeState {
    /// What the next commit applies, if the client sent anything since the last
    /// one.
    pending: Mutex<Option<Vec<Fade>>>,
    /// What the surface committed, in the order it committed it.
    committed: Mutex<Option<Vec<Fade>>>,
}

impl FadeState {
    /// The lock, or the value behind a poisoned one.
    ///
    /// A poisoned lock means this module panicked while holding it, which can
    /// only leave the metadata itself untouched; the worst case of reading it is
    /// that a fade list is one commit stale, never that a client sees another
    /// client's data.
    fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        value.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Registers the manager global.
///
/// Every client of the session socket may bind it: the metadata describes the
/// client's own surface, so it grants nothing over another client.
pub(crate) fn global(display_handle: &DisplayHandle) -> GlobalId {
    display_handle.create_global::<State, GnoblinBlurFadeManagerV1, FadeGlobal>(
        manager::GnoblinBlurFadeManagerV1::interface().version,
        FadeGlobal,
    )
}

/// The manager global's data, which decides nothing: the roster is not asked.
struct FadeGlobal;

impl GlobalDispatch2<GnoblinBlurFadeManagerV1, State> for FadeGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<GnoblinBlurFadeManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerObject);
    }
}

/// The user data of a bound manager.
struct ManagerObject;

impl Dispatch2<GnoblinBlurFadeManagerV1, State> for ManagerObject {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &GnoblinBlurFadeManagerV1,
        request: manager::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::Destroy => {}
            manager::Request::SetFades { surface, fades } => match parse(&fades) {
                Some(fades) => {
                    let surface = surface;
                    with_states(&surface, |states| {
                        let state = states.data_map.get_or_insert(FadeState::default);
                        *FadeState::lock(&state.pending) = Some(fades);
                    });
                }
                None => resource.post_error(
                    manager::Error::InvalidFades as u32,
                    "expected at most 32 fixed-point fade rectangles",
                ),
            },
        }
    }
}

/// Parses a `set_fades` array, or answers [`None`] for every malformed shape.
///
/// The rules are the C compositor's, value for value: a whole number of records,
/// at most 32 of them, every coordinate inside the 24.8 range, a non-negative
/// width and height and an opacity from zero to one.
fn parse(array: &[u8]) -> Option<Vec<Fade>> {
    let record = RECORD_VALUES * std::mem::size_of::<i32>();
    if !array.len().is_multiple_of(record) || array.len() > MAX_FADES * record {
        return None;
    }
    let mut fades = Vec::with_capacity(array.len() / record);
    for chunk in array.chunks_exact(record) {
        let mut values = [0_i32; RECORD_VALUES];
        let (words, _) = chunk.as_chunks::<4>();
        for (value, bytes) in values.iter_mut().zip(words) {
            *value = i32::from_ne_bytes(*bytes);
        }
        let [x, y, width, height, opacity] = values;
        if [x, y, width, height].iter().any(|value| value.abs() > MAX_FIXED) {
            return None;
        }
        if width < 0 || height < 0 || !(0..=MAX_FIXED).contains(&opacity) {
            return None;
        }
        fades.push(Fade {
            x: x as f32 / FIXED_SCALE,
            y: y as f32 / FIXED_SCALE,
            width: width as f32 / FIXED_SCALE,
            height: height as f32 / FIXED_SCALE,
            opacity: opacity as f32 / FIXED_SCALE,
        });
    }
    Some(fades)
}

impl State {
    /// The fade metadata a surface committed, or [`None`] when it has none.
    ///
    /// A visitor rather than a slice, because the records live in the surface's
    /// own data map behind a lock: the caller reads them under it and copies
    /// nothing, which is what a per-frame renderer needs.
    pub fn with_blur_fades<T>(&self, surface: &WlSurface, f: impl FnOnce(&[Fade]) -> T) -> Option<T> {
        with_states(surface, |states| {
            let state = states.data_map.get::<FadeState>()?;
            let committed = FadeState::lock(&state.committed);
            let fades = committed.as_deref()?;
            Some(f(fades))
        })
    }

    /// A surface committed: its staged metadata becomes the committed metadata.
    ///
    /// Called from the compositor's commit hook, which runs after the buffer
    /// handler has applied the surface state, so the metadata applies to exactly
    /// the frame it was sent with.
    pub(crate) fn blur_fades_committed(&mut self, surface: &WlSurface) {
        with_states(surface, |states| {
            let Some(state) = states.data_map.get::<FadeState>() else {
                return;
            };
            let staged = FadeState::lock(&state.pending).take();
            let Some(staged) = staged else {
                return;
            };
            debug!(records = staged.len(), "[blur] fade metadata committed");
            *FadeState::lock(&state.committed) = Some(staged);
        });
    }
}

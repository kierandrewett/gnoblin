//! XML protocol `wlr_screencopy_unstable_v1` (`<protocol name="wlr_screencopy_unstable_v1">`); interfaces served: `zwlr_screencopy_manager_v1`.
//!
//! `zwlr_screencopy_manager_v1` and the `zwlr_screencopy_frame_v1` objects it creates.
//!
//! The older wlroots screencopy protocol: a frame is created for an output or a region,
//! answered `failed` when it cannot be captured, and copies into the client's shm buffer
//! once. The pixel work it shares with `ext_image_copy_capture_v1` lives in
//! [`capture_shared`](super::capture_shared).
//!
//! # Refusals and failure modes
//!
//! 1. **No output, or capture not allowed.** A frame for an output the session does not
//!    drive, or any frame while the lock forbids capture, is answered `failed` at once.
//!    No pixels are copied and no previous frame is repeated.
//! 2. **A buffer that does not match the frame.** The copy is refused when the shm
//!    buffer's size or format differs from the one the frame announced.
//! 3. **A second copy on one frame.** A frame copies once; a second request is a protocol
//!    error for that client only.
//!
//! # Guarding tests
//!
//! `tests/capture.rs`: `a_screencopy_of_the_output_returns_the_window_pixels`,
//! `a_screencopy_with_overlay_cursor_draws_the_cursor_and_one_without_does_not`,
//! `a_capture_during_the_lock_is_refused`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::backend::allocator::Buffer as _;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::backend::ClientId as WaylandClientId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::shm;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_capture::{Format, PixelFormat, Target};
use gnoblin_lock::CaptureStream;
use gnoblin_types::{ClientId, OutputId, Point, Rect, Size, Tick};

use super::capture_shared::{ScreencopyPending, timestamp};
use crate::State;
use crate::access::{Access, AccessPolicy, Global};

use crate::log;

/// The `zwlr_screencopy_manager_v1` global's data.
pub(super) struct ZwlrScreencopyV1Global {
    /// The session's roster, which answers whether a connection may bind it.
    pub(super) access: Arc<Access>,
}

impl std::fmt::Debug for ZwlrScreencopyV1Global {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ZwlrScreencopyV1Global").finish_non_exhaustive()
    }
}

impl GlobalDispatch2<ZwlrScreencopyManagerV1, State> for ZwlrScreencopyV1Global {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ZwlrScreencopyV1ScreencopyManager);
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::Screencopy)
    }
}

/// The manager carries no state of its own: every frame it creates does.
#[derive(Debug)]
struct ZwlrScreencopyV1ScreencopyManager;

impl Dispatch2<ZwlrScreencopyManagerV1, State> for ZwlrScreencopyV1ScreencopyManager {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                create_screencopy_frame(state, client, frame, &output, None, overlay_cursor != 0, data_init);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => {
                let region = Rect::new(
                    x,
                    y,
                    u32::try_from(width).unwrap_or(0),
                    u32::try_from(height).unwrap_or(0),
                );
                create_screencopy_frame(
                    state,
                    client,
                    frame,
                    &output,
                    Some(region),
                    overlay_cursor != 0,
                    data_init,
                );
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

/// One `zwlr_screencopy_frame_v1` object.
#[derive(Debug)]
struct ZwlrScreencopyV1ScreencopyFrame {
    /// The connection that created it, so its session has an owner.
    owner: ClientId,
    /// The output it captures.
    output: OutputId,
    /// The region, in the output's physical pixels.
    region: Rect,
    /// The buffer geometry the client was told to allocate.
    format: Format,
    /// Whether the object has already copied, which the protocol forbids repeating.
    used: AtomicBool,
    /// Whether the client asked for the cursor to be composed into the capture.
    with_cursor: bool,
}

impl Dispatch2<ZwlrScreencopyFrameV1, State> for ZwlrScreencopyV1ScreencopyFrame {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => self.copy(state, frame, &buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => self.copy(state, frame, &buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: WaylandClientId, resource: &ZwlrScreencopyFrameV1) {
        state.capture.screencopy_frames.retain(|frame| frame != resource);
    }
}

impl ZwlrScreencopyV1ScreencopyFrame {
    /// Answers a copy request.
    ///
    /// One object is one copy: a second request is the protocol's `already_used` error,
    /// so a client cannot make the compositor write into a buffer it has already read.
    fn copy(&self, state: &mut State, resource: &ZwlrScreencopyFrameV1, buffer: &WlBuffer, with_damage: bool) {
        if self.used.swap(true, Ordering::Relaxed) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "this frame has already been copied",
            );
            return;
        }
        state.capture.screencopy_frames.retain(|frame| frame != resource);
        // Checked again at the copy boundary: a frame object created before the cover
        // went up must never expose the desktop after it did.
        if !state.capture_allowed() {
            log::debug!(area = "capture", output = %self.output, "a screencopy frame is refused during the lock");
            resource.failed();
            state.flush();
            return;
        }
        if !self.buffer_matches(buffer) && get_dmabuf(buffer).is_err() {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "the buffer does not match the advertised geometry",
            );
            return;
        }
        if let Ok(dmabuf) = get_dmabuf(buffer) {
            if !self.dmabuf_matches(state, dmabuf) {
                resource.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "the buffer does not match the advertised geometry or format",
                );
                return;
            }
            // The copy is made where the renderer is, which is not here: the loop makes it and answers.
            state.queue_screencopy(ScreencopyPending {
                resource: resource.clone(),
                owner: self.owner,
                output: self.output,
                region: self.region,
                format: self.format,
                with_cursor: self.with_cursor,
                with_damage,
                dmabuf: dmabuf.clone(),
            });
            return;
        }
        let capture = state.lock_capture(CaptureStream::Requested);
        let tick = state.capture.tick;
        let offset = Point::new(self.region.x, self.region.y);
        let copied = state.copy_into_buffer(
            self.owner,
            self.output,
            buffer,
            self.format,
            offset,
            self.with_cursor,
            capture,
            tick,
        );
        answer_copy(resource, with_damage, copied);
        state.flush();
    }

    /// Whether the client's buffer is the geometry the frame advertised.
    fn buffer_matches(&self, buffer: &WlBuffer) -> bool {
        let expected = (
            self.format.size().width,
            self.format.size().height,
            self.format.stride(),
        );
        shm::with_buffer_contents(buffer, |_pointer, _length, data| {
            (
                u32::try_from(data.width).unwrap_or(0),
                u32::try_from(data.height).unwrap_or(0),
                u32::try_from(data.stride).unwrap_or(0),
                data.format,
            ) == (expected.0, expected.1, expected.2, wl_shm::Format::Argb8888)
        })
        .unwrap_or(false)
    }

    /// Whether a GPU buffer is one the frame offered: exactly the advertised size, in a format and modifier the
    /// backend can render into, with a plane count its modifier allows.
    fn dmabuf_matches(&self, state: &State, dmabuf: &Dmabuf) -> bool {
        let Some(support) = state.screencopy_dmabuf_support(self.output) else {
            return false;
        };
        let size = dmabuf.size();
        let advertised = self.format.size();
        u32::try_from(size.w) == Ok(advertised.width)
            && u32::try_from(size.h) == Ok(advertised.height)
            && support.check(dmabuf).is_ok()
    }
}

/// Tells a screencopy client how its copy ended: the damage when it asked, the flags and the time the frame was
/// presented, or `failed` when nothing was copied.
pub(super) fn answer_copy(resource: &ZwlrScreencopyFrameV1, with_damage: bool, copied: Option<(Rect, Tick)>) {
    match copied {
        Some((copied, presented)) => {
            if with_damage {
                // One rectangle for the whole frame: the whole buffer was written,
                // so the copy that happened is the damage, not the scene's.
                resource.damage(0, 0, copied.width, copied.height);
            }
            resource.flags(zwlr_screencopy_frame_v1::Flags::empty());
            let (high, low, nanos) = timestamp(presented);
            resource.ready(high, low, nanos);
        }
        None => resource.failed(),
    }
}

/// The format a frame that cannot be captured is created with: one opaque pixel, so the
/// object exists and every copy of it fails.
fn placeholder() -> Format {
    Format::new(Size::new(1, 1), 4, PixelFormat::Argb8888).expect("a one-pixel format is valid")
}

/// Creates a screencopy frame for `output`, or answers it `failed` when it cannot be
/// captured.
fn create_screencopy_frame(
    state: &mut State,
    client: &Client,
    frame: New<ZwlrScreencopyFrameV1>,
    output: &WlOutput,
    region: Option<Rect>,
    with_cursor: bool,
    data_init: &mut DataInit<'_, State>,
) {
    let owner = state.capture.owner(&client.id());
    // Creating the frame needs the manager's data, which is not the client's; the
    // identity is resolved first so the borrow of the session's owner table ends here.
    let output = SmithayOutput::from_resource(output)
        .and_then(|output| state.output_id_of(&output))
        .filter(|output| state.output_rect(*output).is_some());
    // No usable buffer is advertised while the cover is up, and an output this session
    // does not drive has no pixels to offer: both answer `failed` before the client
    // allocates anything.
    let refused = output.is_none() || !state.capture_allowed();
    let bounds = output.and_then(|output| state.capture_bounds(Target::Output(output)));
    let region = match (bounds, region) {
        (Some(bounds), Some(region)) => region.intersection(bounds),
        (Some(bounds), None) => Some(bounds),
        (None, _) => None,
    };
    if refused || region.is_none() {
        let resource = data_init.init(
            frame,
            ZwlrScreencopyV1ScreencopyFrame {
                owner,
                output: output.unwrap_or_default(),
                region: Rect::EMPTY,
                format: placeholder(),
                with_cursor,
                used: AtomicBool::new(true),
            },
        );
        resource.failed();
        state.flush();
        return;
    }
    let (Some(output), Some(region)) = (output, region) else {
        return;
    };
    let Some(format) = Format::new(region.size(), region.width.saturating_mul(4), PixelFormat::Argb8888) else {
        let resource = data_init.init(
            frame,
            ZwlrScreencopyV1ScreencopyFrame {
                owner,
                output,
                region,
                format: placeholder(),
                with_cursor,
                used: AtomicBool::new(true),
            },
        );
        resource.failed();
        state.flush();
        return;
    };
    let resource = data_init.init(
        frame,
        ZwlrScreencopyV1ScreencopyFrame {
            owner,
            output,
            region,
            format,
            with_cursor,
            used: AtomicBool::new(false),
        },
    );
    resource.buffer(
        wl_shm::Format::Argb8888,
        format.size().width,
        format.size().height,
        format.stride(),
    );
    if resource.version() >= 3 {
        // A GPU buffer is offered only for an output the backend can write a capture into, and then in each format
        // it can render into: this protocol names the format and no modifier, and the check at the copy takes the
        // buffer's own.
        let codes: BTreeSet<u32> = state
            .screencopy_dmabuf_support(output)
            .map(|support| support.formats.iter().map(|format| format.code as u32).collect())
            .unwrap_or_default();
        for code in codes {
            resource.linux_dmabuf(code, format.size().width, format.size().height);
        }
        resource.buffer_done();
    }
    state.capture.screencopy_frames.push(resource);
    state.flush();
}

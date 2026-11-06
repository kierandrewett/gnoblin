//! One nested output's target: a software renderer, a shared-memory pool and the
//! surface the pixels are committed to.
//!
//! The presentation rules are the ones the wire has, and each is a value here so it
//! can be tested without a socket:
//!
//! - **A slot is only written when the parent has released it.** [`Ring`] states
//!   which slots are free, and a frame with no free slot is not presented: the parent
//!   is still reading what it has, and writing over it would tear the pixels a person
//!   is looking at.
//! - **The parent is asked for a frame only when it has finished the last one.** The
//!   target knows whether a frame callback is outstanding, so a compositor that
//!   renders faster than its parent cannot queue frames.
//! - **The frame is written whole and the damage is forwarded.** A slot is filled from
//!   the renderer in one pass and the frame's damage rectangles are stated to the
//!   parent as `damage_buffer`, so a parent that honours damage reads less than a whole
//!   frame. Writing the whole frame rather than only the damage is deliberate: a parent
//!   is free to ignore damage and draw the surface from the attached buffer, and half a
//!   frame in a buffer that was never fully filled is tearing for everyone who does.
//! - **The buffer matches the configure that asked for it.** Size and scale come from
//!   the parent, and [`plan`] decides the physical buffer size, the `wl_surface`
//!   buffer scale and whether a viewport destination is needed to state the logical
//!   size a fractional scale produces.
//!
//! # Failure modes
//!
//! 1. **A frame is written into a buffer the parent still owns.** A taken slot is not
//!    free until its release arrives, and a frame with no free slot presents nothing:
//!    `ring_tests::a_slot_in_flight_is_not_taken_twice` and
//!    `ring_tests::a_full_ring_presents_nothing`.
//! 2. **A fractional scale produces a buffer the parent draws at the wrong size.** The
//!    buffer plan states the physical size and either the buffer scale or the viewport
//!    destination: `tests::an_integral_scale_needs_no_viewport`,
//!    `tests::a_fractional_scale_is_drawn_through_the_viewport` and
//!    `tests::a_parent_without_a_viewport_falls_back_to_a_whole_scale`.
//! 3. **A parent that cannot scale gets buffers at the wrong size.** A `wl_surface`
//!    older than version 3 has no buffer scale, so the plan draws at one physical
//!    pixel per logical pixel: `tests::a_surface_without_a_buffer_scale_draws_at_one_to_one`.
//! 4. **A resize leaves the renderer and the pool disagreeing.** A configure creates
//!    the new pool before it touches the renderer, and a renderer that cannot resize
//!    destroys the pool it was going to use, so the target is never half resized.
//! 5. **A release from a replaced pool frees a slot of the new one.** Every pool has a
//!    generation and a release carries it, so `NestedTarget::release` ignores a
//!    generation that is not the current one.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use gnoblin_log::{debug_in, warn_in};
use gnoblin_types::{Rect, Scale, Size};
use wayland_client::protocol::wl_buffer;
use wayland_client::protocol::{wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use super::NestedError;
use super::handler::{Handler, OutputKey, SlotKey};
use crate::Present;
use crate::render::{Image, OutputFrame, RenderError, Software};
use crate::{Output, Target};

/// How many buffers one nested output cycles through.
///
/// Two is the smallest number that lets the parent still be reading one while the
/// session draws the next; a third would only matter for a parent that holds buffers
/// far longer than a frame.
const SLOTS: usize = 2;

/// The alignment of one slot inside the pool.
///
/// `wl_shm` does not require it, but every compositor that maps a pool does so in
/// whole pages, and an aligned slot keeps a buffer's first row on its own page.
const SLOT_ALIGNMENT: usize = 4096;

/// The largest buffer this backend will create, in pixels on one axis.
///
/// A parent can state any configure size, and creating a pool for a nonsense one would
/// be a multi-gigabyte allocation before anything is drawn. It is deliberately far
/// above any monitor and well below anything that would exhaust a session's memory.
const MAX_EXTENT: u32 = 16384;

/// A counter for the pool files this process creates, so two sessions in one process
/// never pick the same name.
static POOL_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Which pool slots are free.
///
/// A slot is taken while a frame in it is on the wire and released when the parent says
/// it is done with it.
#[derive(Debug)]
pub(crate) struct Ring {
    /// Whether each slot is free to draw into, indexed by slot.
    free: Vec<bool>,
}

impl Ring {
    /// A ring of `slots` free slots.
    pub fn new(slots: usize) -> Self {
        Self {
            free: vec![true; slots],
        }
    }

    /// Takes a free slot.
    pub fn take(&mut self) -> Option<usize> {
        let index = self.free.iter().position(|free| *free)?;
        if let Some(entry) = self.free.get_mut(index) {
            *entry = false;
        }
        Some(index)
    }

    /// Returns a slot the parent has finished with.
    ///
    /// A slot outside the ring is ignored: a release from a pool that a resize replaced
    /// names slots the current ring never had.
    pub fn release(&mut self, slot: u8) {
        if let Some(entry) = self.free.get_mut(usize::from(slot)) {
            *entry = true;
        }
    }

    /// How many slots are free.
    #[cfg(test)]
    pub fn available(&self) -> usize {
        self.free.iter().filter(|free| **free).count()
    }

    /// Whether no slot is free.
    #[cfg(test)]
    pub fn is_full(&self) -> bool {
        self.available() == 0
    }
}

/// What a logical size and a compositing scale become on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BufferPlan {
    /// The buffer's size in physical pixels.
    pub physical: Size,
    /// The `wl_surface.set_buffer_scale` value, or [`None`] for a surface too old to
    /// have one.
    pub buffer_scale: Option<i32>,
    /// The viewport destination, which states the logical size a fractional scale
    /// produces, or [`None`] when the buffer scale already says it.
    pub destination: Option<Size>,
}

/// How a logical size and a compositing scale become a buffer.
///
/// `viewport` is whether the parent has a `wp_viewporter`, and `buffer_scale` whether
/// its `wl_surface` is new enough to state one.
pub(crate) fn plan(size: Size, scale: Scale, viewport: bool, buffer_scale: bool) -> BufferPlan {
    if !buffer_scale {
        // Without `set_buffer_scale` a surface's logical size *is* its buffer's, so the
        // only buffer it can accept is one at one physical pixel per logical one.
        return BufferPlan {
            physical: size,
            buffer_scale: None,
            destination: None,
        };
    }
    if scale.denominator() == 1 {
        let steps = scale.numerator();
        return BufferPlan {
            physical: Size::new(size.width.saturating_mul(steps), size.height.saturating_mul(steps)),
            buffer_scale: Some(i32::try_from(steps).unwrap_or(i32::MAX)),
            destination: None,
        };
    }
    if viewport {
        // A fractional scale is drawn by handing the parent a buffer of the physical
        // size and telling it, through the viewport, which logical size it stands for.
        return BufferPlan {
            physical: physical_size(size, scale),
            buffer_scale: Some(1),
            destination: Some(size),
        };
    }
    // No viewport: a fractional scale cannot be stated, so the nearest whole one is
    // used. It is drawn slightly wrong for a person looking closely, and it is still a
    // correct buffer the parent accepts.
    let steps = rounded_scale(scale);
    BufferPlan {
        physical: Size::new(size.width.saturating_mul(steps), size.height.saturating_mul(steps)),
        buffer_scale: Some(i32::try_from(steps).unwrap_or(i32::MAX)),
        destination: None,
    }
}

/// The physical extent of a logical one at a scale, rounded half up.
fn physical_size(size: Size, scale: Scale) -> Size {
    Size::new(physical_extent(size.width, scale), physical_extent(size.height, scale))
}

/// One axis of [`physical_size`].
pub(crate) fn physical_extent(logical: u32, scale: Scale) -> u32 {
    let numerator = u64::from(logical) * u64::from(scale.numerator());
    let divisor = u64::from(scale.denominator());
    let rounded = (numerator + divisor / 2) / divisor;
    u32::try_from(rounded).unwrap_or(u32::MAX).max(1)
}

/// A fractional scale as the nearest whole number of physical pixels per logical one.
fn rounded_scale(scale: Scale) -> u32 {
    let numerator = u64::from(scale.numerator());
    let divisor = u64::from(scale.denominator());
    let rounded = (numerator + divisor / 2) / divisor;
    u32::try_from(rounded).unwrap_or(1).max(1)
}

/// A shared-memory pool and the buffers a target cycles through.
struct Pool {
    /// The file the buffers live in, kept for writing pixels into.
    file: File,
    /// The pool object, kept so it can be destroyed when a resize replaces it.
    pool: wl_shm_pool::WlShmPool,
    /// The buffer objects, one per slot, in slot order.
    buffers: Vec<wl_buffer::WlBuffer>,
    /// Which pool this is, so a release for an older one is identifiable.
    generation: u32,
    /// Bytes from one row of pixels to the next.
    stride: usize,
    /// The width of each buffer, in physical pixels.
    width: u32,
    /// The height of each buffer, in physical pixels.
    height: u32,
    /// Bytes from one slot's start to the next, padded to the pool's slot alignment.
    slot_bytes: usize,
}

impl Pool {
    /// A pool of [`SLOTS`] buffers of `physical` pixels.
    fn new(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<Handler>,
        index: usize,
        generation: u32,
        physical: Size,
        format: wl_shm::Format,
    ) -> Result<Self, NestedError> {
        let width = i32::try_from(physical.width).map_err(|_| NestedError::NoFormat)?;
        let height = i32::try_from(physical.height).map_err(|_| NestedError::NoFormat)?;
        let stride = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or(NestedError::NoFormat)?;
        let rows = usize::try_from(height).map_err(|_| NestedError::NoFormat)?;
        let slot_bytes = align_up(stride.checked_mul(rows).ok_or(NestedError::NoFormat)?);
        let length = slot_bytes.checked_mul(SLOTS).ok_or(NestedError::NoFormat)?;
        let length = i32::try_from(length).map_err(|_| NestedError::NoFormat)?;
        let file = anonymous_file(usize::try_from(length).unwrap_or(0)).map_err(NestedError::Buffer)?;
        let pool = shm.create_pool(file.as_fd(), length, qh, ());
        let mut buffers = Vec::with_capacity(SLOTS);
        for slot in 0..SLOTS {
            buffers.push(pool.create_buffer(
                i32::try_from(slot * slot_bytes).map_err(|_| NestedError::NoFormat)?,
                width,
                height,
                i32::try_from(stride).map_err(|_| NestedError::NoFormat)?,
                format,
                qh,
                SlotKey {
                    index,
                    generation,
                    slot: u8::try_from(slot).unwrap_or(0),
                },
            ));
        }
        Ok(Self {
            file,
            pool,
            buffers,
            generation,
            stride,
            width: physical.width,
            height: physical.height,
            slot_bytes,
        })
    }

    /// Where a slot starts in the file.
    fn offset(&self, slot: usize) -> usize {
        slot * self.slot_bytes
    }

    /// Tells the parent this pool is no longer needed.
    ///
    /// The buffers created from it stay valid until the parent releases them, which is
    /// what `wl_shm_pool.destroy` states, so a frame in flight is not torn down.
    fn destroy(self) {
        self.pool.destroy();
    }
}

/// Whether the parent's last frame is still on the wire.
///
/// | State      | Event                 | Result                                    |
/// |------------|-----------------------|-------------------------------------------|
/// | `Idle`     | present a frame       | sent, becomes `Awaiting`                  |
/// | `Awaiting` | present a frame       | refused (`AlreadyAwaiting`), nothing sent |
/// | `Awaiting` | parent frame callback | becomes `Idle`                            |
/// | `Idle`     | parent frame callback | stays `Idle` (a stray callback)           |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameWait {
    /// No frame is waiting for the parent, so the next present may send one.
    Idle,
    /// A frame is on the wire and the parent has not called back yet.
    Awaiting,
}

/// The buffer scale the parent was last told. A fresh or resized target has told it
/// nothing, so the first present states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferScaleState {
    /// Nothing has been stated to the parent since the surface was made or resized.
    Unstated,
    /// The parent was told this buffer scale.
    Stated(i32),
}

/// One nested output: the renderer, the pool and the surface.
pub struct NestedTarget {
    /// The output this target presents. Its mode is physical, so its logical size is
    /// the size the parent configured.
    output: Output,
    /// The software renderer that draws the output's pixels.
    software: Software,
    /// The parent's surface the output is presented on.
    surface: wl_surface::WlSurface,
    /// The viewport that scales the buffer to the surface's logical size, if the parent has one.
    viewport: Option<WpViewport>,
    /// The parent's shared-memory global, used to create pools.
    shm: wl_shm::WlShm,
    /// The queue handle, used to create pools and buffers.
    qh: QueueHandle<Handler>,
    /// The record index, which is what the frame callback and the buffers carry.
    index: usize,
    /// The shared-memory pool the slots live in.
    pool: Pool,
    /// Which slots are free to draw into.
    ring: Ring,
    /// Whether a frame is on the wire and its callback has not arrived.
    frame: FrameWait,
    /// The damage that has not reached the parent yet, in the output's logical space.
    damage: Vec<Rect>,
    /// How many frames the target has presented.
    presents: u64,
    /// The shared-memory pixel format of the buffers.
    format: wl_shm::Format,
    /// The buffer scale last stated to the parent, so a steady frame repeats nothing.
    buffer_scale: BufferScaleState,
    /// One row of packed slot pixels, so a frame with damage allocates nothing.
    scratch: Vec<u8>,
}

impl std::fmt::Debug for NestedTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NestedTarget")
            .field("output", &self.output)
            .field("physical", &Size::new(self.pool.width, self.pool.height))
            .field("frame", &self.frame)
            .field("presents", &self.presents)
            .finish_non_exhaustive()
    }
}

impl NestedTarget {
    /// A target that presents `output` on `surface`.
    ///
    /// # Errors
    ///
    /// [`NestedError::NoFormat`] for a size this backend will not allocate,
    /// [`NestedError::Buffer`] when the pool's file cannot be created, and
    /// [`NestedError::Render`] when the renderer cannot be created.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        qh: &QueueHandle<Handler>,
        shm: &wl_shm::WlShm,
        format: wl_shm::Format,
        surface: wl_surface::WlSurface,
        viewport: Option<WpViewport>,
        index: usize,
        output: Output,
    ) -> Result<Self, NestedError> {
        let planned = plan(
            output.logical_size(),
            output.scale,
            viewport.is_some(),
            surface.version() >= 3,
        );
        if planned.physical.width > MAX_EXTENT || planned.physical.height > MAX_EXTENT {
            return Err(NestedError::NoFormat);
        }
        let software = Software::new(planned.physical).map_err(NestedError::Render)?;
        let pool = Pool::new(shm, qh, index, 1, planned.physical, format)?;
        let mut target = Self {
            output,
            software,
            surface,
            viewport,
            shm: shm.clone(),
            qh: qh.clone(),
            index,
            pool,
            ring: Ring::new(SLOTS),
            frame: FrameWait::Idle,
            damage: Vec::new(),
            presents: 0,
            format,
            buffer_scale: planned
                .buffer_scale
                .map_or(BufferScaleState::Unstated, BufferScaleState::Stated),
            scratch: Vec::new(),
        };
        target.damage_whole();
        Ok(target)
    }

    /// The output this target presents.
    #[must_use]
    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Whether a frame is on the wire and the parent has not finished it.
    #[must_use]
    pub fn awaiting_frame(&self) -> bool {
        self.frame == FrameWait::Awaiting
    }

    /// How many frames this target presented.
    #[must_use]
    pub fn presents(&self) -> u64 {
        self.presents
    }

    /// The damage the next present will forward, in the output's logical space.
    #[must_use]
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    /// The pixels of the last frame drawn, read back from the renderer.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot copy the buffer out.
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        self.software.capture(with_cursor)
    }

    /// Applies a new configure: the mode the output is in and the scale it composes at.
    ///
    /// The buffer is replaced, so the whole output is damaged: the pixels the parent
    /// holds are the old size and cannot be reused.
    ///
    /// # Errors
    ///
    /// [`NestedError::NoFormat`] for a mode with no pixels or a size this backend will
    /// not allocate, [`NestedError::Buffer`] when the new pool cannot be created, and
    /// [`NestedError::Render`] when the renderer cannot resize. The old pool and buffer
    /// are kept on failure, so a parent that states an impossible size does not leave
    /// the output blank.
    pub(crate) fn resize(&mut self, output: Output) -> Result<(), NestedError> {
        if output.mode.width == 0 || output.mode.height == 0 {
            return Err(NestedError::NoFormat);
        }
        let planned = plan(
            output.logical_size(),
            output.scale,
            self.viewport.is_some(),
            self.surface.version() >= 3,
        );
        if planned.physical.width > MAX_EXTENT || planned.physical.height > MAX_EXTENT {
            return Err(NestedError::NoFormat);
        }
        // The pool is created first and the renderer resized second: whichever step
        // fails, the target is left exactly as it was.
        let pool = Pool::new(
            &self.shm,
            &self.qh,
            self.index,
            self.pool.generation.wrapping_add(1),
            planned.physical,
            self.format,
        )?;
        if let Err(error) = self.software.resize(planned.physical) {
            pool.destroy();
            return Err(NestedError::Render(error));
        }
        let previous = std::mem::replace(&mut self.pool, pool);
        previous.destroy();
        self.output = output;
        self.ring = Ring::new(SLOTS);
        // The buffer scale is a property of the surface rather than of a buffer, and it
        // may have changed with the configure, so the next present states it again.
        self.buffer_scale = BufferScaleState::Unstated;
        self.damage_whole();
        Ok(())
    }

    /// The parent released one of this target's buffers.
    pub(crate) fn release(&mut self, generation: u32, slot: u8) {
        if generation == self.pool.generation {
            self.ring.release(slot);
        }
    }

    /// The parent finished the frame this target presented.
    pub(crate) fn frame_done(&mut self) {
        self.frame = self.frame.callback();
    }

    /// Damages the whole output.
    fn damage_whole(&mut self) {
        let size = self.output.logical_size();
        self.damage.clear();
        self.damage.push(Rect::new(0, 0, size.width, size.height));
    }

    /// Writes the whole of `image` into `slot`.
    ///
    /// One `pwrite` of the whole slot rather than a loop over the damage: the readback
    /// has already copied the framebuffer once, a slot that is only partly filled is a
    /// frame a parent may draw with holes in it, and one system call beats six hundred.
    fn write_slot(&mut self, slot: usize, image: &Image) -> io::Result<()> {
        let Self {
            pool, scratch, format, ..
        } = self;
        let rows = usize::try_from(pool.height).unwrap_or(0);
        let length = pool.stride.saturating_mul(rows);
        if scratch.len() < length {
            scratch.resize(length, 0);
        }
        for row in 0..rows {
            let Some(source) = image
                .data()
                .get(row * image.stride()..row * image.stride() + pool.stride)
            else {
                // The readback is shorter than the buffer it was given: the frame is not
                // written rather than half written.
                return Err(io::Error::other("the readback is shorter than its buffer"));
            };
            let destination = &mut scratch[row * pool.stride..(row + 1) * pool.stride];
            let (pixels, remainder) = source.as_chunks::<4>();
            let (packed_rows, _) = destination.as_chunks_mut::<4>();
            debug_assert!(remainder.is_empty(), "a row is four bytes per pixel");
            for (pixel, out) in pixels.iter().zip(packed_rows.iter_mut()) {
                *out = packed(pixel, *format);
            }
        }
        pool.file.write_all_at(&scratch[..length], pool.offset(slot) as u64)
    }
}

impl Target for NestedTarget {
    fn size(&self) -> Size {
        self.output.logical_size()
    }

    fn scale(&self) -> Scale {
        self.output.scale
    }

    fn draw(&mut self, frame: &OutputFrame) -> Result<(), RenderError> {
        self.software.draw(frame, self.output.scale)
    }

    fn present(&mut self, damage: &[Rect]) -> Present {
        let bounds = Rect::new(0, 0, self.size().width, self.size().height);
        // Damage accumulates until a frame actually reaches the parent: a frame that is
        // skipped would otherwise leave the parent showing what it replaced.
        self.damage
            .extend(damage.iter().filter_map(|rect| rect.intersection(bounds)));
        if self.frame.send().is_err() {
            // Named refusal: the parent has not answered the last frame, so this one is
            // not sent and nothing is written.
            return Present::Nothing;
        }
        let Some(slot) = self.ring.take() else {
            // Every buffer is still attached; the parent is behind.
            debug_in!("backend", "[nested] every buffer is in flight, so this frame waits");
            return Present::Nothing;
        };
        let released = u8::try_from(slot).unwrap_or(0);
        let image = match self.software.capture(true) {
            Ok(image) => image,
            Err(error) => {
                warn_in!("backend", ?error, "[nested] the frame could not be read back");
                self.ring.release(released);
                return Present::Nothing;
            }
        };
        if let Err(error) = self.write_slot(slot, &image) {
            warn_in!(
                "backend",
                ?error,
                "[nested] the frame could not be written into its buffer"
            );
            self.ring.release(released);
            return Present::Nothing;
        }
        let Some(buffer) = self.pool.buffers.get(slot).cloned() else {
            self.ring.release(released);
            return Present::Nothing;
        };
        if self.damage.is_empty() {
            // The host said something changed without saying where, so everything is
            // repainted rather than a buffer committed with no damage at all.
            self.damage.push(bounds);
        }
        self.surface.attach(Some(&buffer), 0, 0);
        let planned = plan(
            self.size(),
            self.output.scale,
            self.viewport.is_some(),
            self.surface.version() >= 3,
        );
        if let Some(buffer_scale) = planned.buffer_scale
            && self.buffer_scale != BufferScaleState::Stated(buffer_scale)
        {
            self.surface.set_buffer_scale(buffer_scale);
            self.buffer_scale = BufferScaleState::Stated(buffer_scale);
        }
        if let (Some(destination), Some(viewport)) = (planned.destination, self.viewport.as_ref()) {
            viewport.set_destination(
                i32::try_from(destination.width).unwrap_or(i32::MAX),
                i32::try_from(destination.height).unwrap_or(i32::MAX),
            );
        }
        for rect in &self.damage {
            let rect = scaled(*rect, self.output.scale);
            let width = i32::try_from(rect.width).unwrap_or(i32::MAX);
            let height = i32::try_from(rect.height).unwrap_or(i32::MAX);
            if self.surface.version() >= 4 {
                self.surface.damage_buffer(rect.x, rect.y, width, height);
            } else {
                self.surface.damage(rect.x, rect.y, width, height);
            }
        }
        // The callback is what asks the parent for the next frame; losing the handle
        // does not lose the event, because the object stays registered until the parent
        // answers it.
        let _callback = self.surface.frame(&self.qh, OutputKey(self.index));
        self.surface.commit();
        self.damage.clear();
        self.frame = FrameWait::Awaiting;
        self.presents = self.presents.saturating_add(1);
        Present::Rendered
    }
}

/// One logical rectangle in the buffer's pixels.
fn scaled(rect: Rect, scale: Scale) -> Rect {
    let x = physical_extent_signed(i64::from(rect.x), scale);
    let y = physical_extent_signed(i64::from(rect.y), scale);
    let right = physical_extent_signed(rect.right(), scale);
    let bottom = physical_extent_signed(rect.bottom(), scale);
    Rect::new(
        x,
        y,
        u32::try_from((right - x).max(1)).unwrap_or(1),
        u32::try_from((bottom - y).max(1)).unwrap_or(1),
    )
}

/// One axis of [`scaled`].
fn physical_extent_signed(value: i64, scale: Scale) -> i32 {
    let numerator = i128::from(value) * i128::from(scale.numerator());
    let divisor = i128::from(scale.denominator());
    let rounded = (numerator + divisor / 2) / divisor;
    i32::try_from(rounded).unwrap_or(if rounded < 0 { i32::MIN } else { i32::MAX })
}

/// A pixel of the RGBA8 readback in a `wl_shm` format's byte order.
///
/// The readback is RGBA8 in memory order; a `wl_shm` format is a native-endian 32-bit
/// value, so the bytes a buffer holds depend on the machine.
fn packed(pixel: &[u8], format: wl_shm::Format) -> [u8; 4] {
    let alpha = if matches!(format, wl_shm::Format::Argb8888) {
        pixel.get(3).copied().unwrap_or(0xFF)
    } else {
        // `Xrgb8888` states the alpha byte is unused; an opaque value is what a
        // compositor that does read it should see.
        0xFF
    };
    let red = pixel.first().copied().unwrap_or(0);
    let green = pixel.get(1).copied().unwrap_or(0);
    let blue = pixel.get(2).copied().unwrap_or(0);
    #[cfg(target_endian = "little")]
    {
        // 0xAARRGGBB stored little-endian is B, G, R, A.
        [blue, green, red, alpha]
    }
    #[cfg(target_endian = "big")]
    {
        // 0xAARRGGBB stored big-endian is A, R, G, B.
        [alpha, red, green, blue]
    }
}

/// Rounds `bytes` up to [`SLOT_ALIGNMENT`].
fn align_up(bytes: usize) -> usize {
    let remainder = bytes % SLOT_ALIGNMENT;
    if remainder == 0 {
        bytes
    } else {
        bytes.saturating_add(SLOT_ALIGNMENT - remainder)
    }
}

/// A shared-memory file no other process can find.
///
/// It is created in the runtime directory under a name only this process chooses,
/// opened exclusively, and unlinked before the first pixel is written: the descriptor
/// is the only way to reach it afterwards, and the kernel reclaims it when the session
/// exits. `memfd_create` would skip the path entirely, but it is a raw syscall and this
/// crate forbids unsafe code.
fn anonymous_file(length: usize) -> io::Result<File> {
    let path = runtime_directory().join(format!(
        "gnoblin-nested-{}-{}.shm",
        std::process::id(),
        POOL_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    // The file is reachable only through the descriptor from here on. A failure to
    // unlink is not fatal: the session still has its buffer, and the worst case is a
    // stale path in the runtime directory that the next counter avoids.
    if let Err(error) = std::fs::remove_file(&path) {
        debug_in!("backend", ?error, path = %path.display(), "[nested] the buffer file was not unlinked");
    }
    file.set_len(u64::try_from(length).unwrap_or(u64::MAX))?;
    Ok(file)
}

/// Where a session's socket and buffers live.
fn runtime_directory() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// A frame send was refused because the parent has not answered the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlreadyAwaiting;

impl FrameWait {
    /// Sends a frame: `Idle` becomes `Awaiting`, `Awaiting` refuses with [`AlreadyAwaiting`].
    pub(crate) fn send(self) -> Result<Self, AlreadyAwaiting> {
        match self {
            Self::Idle => Ok(Self::Awaiting),
            Self::Awaiting => Err(AlreadyAwaiting),
        }
    }

    /// The parent's frame callback: `Awaiting` becomes `Idle`; a stray callback stays `Idle`.
    pub(crate) fn callback(self) -> Self {
        Self::Idle
    }
}

#[cfg(test)]
mod frame_wait_tests {
    use super::{AlreadyAwaiting, FrameWait};

    #[test]
    fn an_idle_target_sends_a_frame_and_awaits_it() {
        assert_eq!(FrameWait::Idle.send(), Ok(FrameWait::Awaiting));
    }

    #[test]
    fn an_awaiting_target_refuses_a_second_frame() {
        assert_eq!(FrameWait::Awaiting.send(), Err(AlreadyAwaiting));
    }

    #[test]
    fn the_callback_frees_an_awaiting_target() {
        assert_eq!(FrameWait::Awaiting.callback(), FrameWait::Idle);
    }

    #[test]
    fn a_stray_callback_leaves_an_idle_target_idle() {
        assert_eq!(FrameWait::Idle.callback(), FrameWait::Idle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGICAL: Size = Size::new(800, 600);

    #[test]
    fn an_integral_scale_needs_no_viewport() {
        let planned = plan(LOGICAL, Scale::integral(2), true, true);
        assert_eq!(planned.physical, Size::new(1600, 1200));
        assert_eq!(planned.buffer_scale, Some(2));
        assert_eq!(
            planned.destination, None,
            "the buffer scale already states the logical size"
        );
    }

    #[test]
    fn a_fractional_scale_is_drawn_through_the_viewport() {
        let planned = plan(LOGICAL, Scale::new(3, 2), true, true);
        assert_eq!(planned.physical, Size::new(1200, 900));
        assert_eq!(
            planned.buffer_scale,
            Some(1),
            "a fractional scale cannot be a buffer scale"
        );
        assert_eq!(planned.destination, Some(LOGICAL));
    }

    #[test]
    fn a_parent_without_a_viewport_falls_back_to_a_whole_scale() {
        let planned = plan(LOGICAL, Scale::new(3, 2), false, true);
        assert_eq!(planned.physical, Size::new(1600, 1200), "three halves rounded to two");
        assert_eq!(planned.buffer_scale, Some(2));
        assert_eq!(planned.destination, None);
    }

    #[test]
    fn a_surface_without_a_buffer_scale_draws_at_one_to_one() {
        let planned = plan(LOGICAL, Scale::new(3, 2), true, false);
        assert_eq!(planned.physical, LOGICAL);
        assert_eq!(planned.buffer_scale, None);
        assert_eq!(planned.destination, None);
    }

    #[test]
    fn a_scale_of_one_is_a_buffer_of_the_logical_size() {
        let planned = plan(LOGICAL, Scale::IDENTITY, false, true);
        assert_eq!(planned.physical, LOGICAL);
        assert_eq!(planned.buffer_scale, Some(1));
    }

    #[test]
    fn a_rounded_logical_size_comes_back_as_the_same_logical_size() {
        // The output's mode is physical, so the logical size is derived from it again:
        // a scale that does not divide evenly must not drift by a pixel.
        for width in 800..=810 {
            let logical = Size::new(width, 600);
            let planned = plan(logical, Scale::new(3, 2), true, true);
            assert_eq!(
                Scale::new(3, 2).logical_size(planned.physical),
                logical,
                "{width} logical pixels at three halves"
            );
        }
    }

    #[test]
    fn damage_is_stated_in_the_buffer_pixels_of_a_fractional_scale() {
        assert_eq!(
            scaled(Rect::new(0, 0, 100, 50), Scale::new(3, 2)),
            Rect::new(0, 0, 150, 75)
        );
        assert_eq!(
            scaled(Rect::new(10, 20, 100, 50), Scale::integral(2)),
            Rect::new(20, 40, 200, 100)
        );
    }

    #[test]
    fn a_pixel_is_packed_in_the_byte_order_of_its_format() {
        // Opaque orange: R 255, G 128, B 64, A 255.
        let pixel = [255, 128, 64, 255];
        let argb = packed(&pixel, wl_shm::Format::Argb8888);
        let xrgb = packed(&pixel, wl_shm::Format::Xrgb8888);
        assert_eq!(argb[3], 255, "Argb8888 keeps the frame's alpha");
        assert_eq!(xrgb[3], 255, "Xrgb8888 states an opaque alpha byte");
        #[cfg(target_endian = "little")]
        {
            assert_eq!(argb, [64, 128, 255, 255], "B, G, R, A on a little-endian machine");
        }
    }

    #[test]
    fn a_slot_is_aligned_to_a_page() {
        assert_eq!(align_up(4096), 4096);
        assert_eq!(align_up(4097), 8192);
        assert_eq!(align_up(0), 0);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::Ring;

    #[test]
    fn a_slot_in_flight_is_not_taken_twice() {
        let mut ring = Ring::new(2);
        assert_eq!(ring.take(), Some(0));
        assert_eq!(ring.take(), Some(1), "the next frame goes to the other buffer");
        assert_eq!(ring.take(), None, "and a third has nowhere to go");
    }

    #[test]
    fn a_released_slot_is_taken_again() {
        let mut ring = Ring::new(2);
        let slot = ring.take().expect("a free slot");
        ring.release(u8::try_from(slot).unwrap_or(0));
        assert_eq!(ring.available(), 2);
        assert_eq!(ring.take(), Some(0), "the released buffer is the next one used");
    }

    #[test]
    fn a_release_outside_the_ring_is_ignored() {
        let mut ring = Ring::new(2);
        let _ = ring.take();
        ring.release(9);
        assert_eq!(
            ring.available(),
            1,
            "a release for a slot this ring never had frees nothing"
        );
    }

    #[test]
    fn a_full_ring_presents_nothing() {
        let mut ring = Ring::new(2);
        assert!(!ring.is_full());
        let _ = ring.take();
        let _ = ring.take();
        assert!(ring.is_full());
        assert_eq!(ring.take(), None);
    }
}

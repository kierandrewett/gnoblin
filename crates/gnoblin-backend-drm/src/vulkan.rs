//! The Vulkan half of the scan-out path: what one output owns when `compositor.renderer = vulkan`.
//!
//! The output composes into the renderer's persistent target and the kernel scans out a GBM buffer, so every flip
//! copies the target over the buffer it is about to show. The copy is the LINEAR path of the Vulkan renderer: the
//! GBM buffer is allocated with the LINEAR modifier, mapped, and written under the kernel's dmabuf sync bracket
//! ([`AnyRenderer::copy_image_into_dmabuf`]). It has finished when it returns, which is why a flip carries no
//! `IN_FENCE_FD`: there is no GPU write in flight to wait for.
//!
//! The target is persistent and the buffers alternate, so a buffer shows what the target held two frames ago. Each
//! slot therefore keeps the damage of every frame it has not been written for ([`SlotDamage`]); the copy writes that
//! union and the same rectangles go to the plane as `FB_DAMAGE_CLIPS`.
//!
//! # Failure modes first
//!
//! 1. **A slot is shown stale.** The frame before it damaged pixels the slot was not written for: a slot's missed
//!    damage is the union of every frame since its own last write (`a_slot_is_written_for_what_the_other_slot_drew`).
//! 2. **A buffer nobody wrote is shown.** A new slot, a resized target and a rebuilt renderer all start as
//!    everything missed (`a_new_slot_misses_everything`).
//! 3. **A refused copy loses the damage.** The missed rectangles are cleared only after every copy succeeded, so the
//!    retry writes them again (`a_slot_that_was_not_written_keeps_what_it_missed`).
//! 4. **The renderer repaints more than the frame says.** A magnifier or a colour matrix reads and rewrites the whole
//!    output, so a frame with one, and the frame after it, misses everything (`a_filter_widens_the_damage_for_two_frames`).
//! 5. **A lost device is drawn on again.** Once a call returns [`RenderError::Lost`] the output is lost and draws
//!    nothing until it is rebuilt ([`VulkanScan::rebuild`]); the GBM buffers survive, so the screen keeps the last
//!    frame ([`Recovery`] paces the rebuilds: `the_rebuilds_back_off_and_settle`).
//! 6. **A capture shows a secure window, or leaves the output showing the redaction.** The frame last drawn is kept
//!    when it has a secure layer; a capture draws it redacted, reads, and draws it whole again
//!    ([`VulkanScan::with_redaction`]).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gnoblin_backend::DmabufCopy;
use gnoblin_backend::render::renderer::{AnyRenderer, GpuShare, RendererKind};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{ColourMatrix, OutputFrame, SecureTreatment};
use gnoblin_render_frame::geometry::physical_rect;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{Point, Rect, Scale, Size};
use smithay::backend::allocator::dmabuf::Dmabuf;

use crate::log;

/// The most rectangles a slot remembers; past it they are one bounding rectangle, so a slot that sat out a long run of
/// scattered damage does not grow a list.
const MOST_RECTS: usize = 32;

/// What one slot has not been written for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Missed {
    /// The whole buffer: nothing was ever written, or the target was replaced.
    Everything,
    /// These rectangles, in the buffer's pixels.
    Rects(Vec<Rect>),
}

/// The damage each scan-out slot has missed: what to write into it before it is shown, and what to tell the plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SlotDamage {
    /// The buffer's extent, which damage is clipped to.
    bounds: Rect,
    slots: Vec<Missed>,
}

impl SlotDamage {
    /// `slots` slots of a buffer of `shown` pixels, none of them written yet.
    pub(crate) fn new(slots: usize, shown: Size) -> Self {
        Self {
            bounds: Rect::new(0, 0, shown.width, shown.height),
            slots: vec![Missed::Everything; slots],
        }
    }

    /// Every slot missed `damage`, which a frame drew into the target. Pixels outside the buffer are dropped.
    pub(crate) fn add(&mut self, damage: &[Rect]) {
        let clipped: Vec<Rect> = damage
            .iter()
            .filter_map(|rect| rect.intersection(self.bounds))
            .collect();
        if clipped.is_empty() {
            return;
        }
        for slot in &mut self.slots {
            let Missed::Rects(held) = slot else { continue };
            held.extend_from_slice(&clipped);
            if held.len() > MOST_RECTS {
                let bound = held.iter().copied().reduce(Rect::union);
                *held = bound.into_iter().collect();
            }
        }
    }

    /// Every slot missed everything: the target was replaced, or the renderer was rebuilt and holds no pixels.
    pub(crate) fn add_everything(&mut self) {
        self.slots.fill(Missed::Everything);
    }

    /// What `slot` must be written with before it is shown. A slot that does not exist has none.
    pub(crate) fn missed(&self, slot: usize) -> Vec<Rect> {
        match self.slots.get(slot) {
            Some(Missed::Everything) => vec![self.bounds],
            Some(Missed::Rects(rects)) => rects.clone(),
            None => Vec::new(),
        }
    }

    /// `slot` was written with everything it had missed.
    pub(crate) fn written(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = Missed::Rects(Vec::new());
        }
    }
}

/// Whether `frame` makes the renderer repaint the whole output: a magnifier or a colour matrix reads and rewrites every
/// pixel, so the pixels it does not damage change too.
#[must_use]
fn filters(frame: &OutputFrame) -> bool {
    let magnified = frame.view().is_some_and(|view| view.zoom != 1.0);
    let tinted = frame
        .colour_matrix()
        .is_some_and(|matrix| matrix.rows != ColourMatrix::IDENTITY.rows);
    magnified || tinted
}

/// The damage of `frame` in the scan-out buffer's pixels.
///
/// The frame's rectangles are logical; the target is `composed` pixels across and the buffer `shown`, which is the
/// same unless the output is supersampled, and then a rectangle covers every shown pixel it touches.
/// `widened` is whether the renderer repaints the whole output for this frame ([`filters`], now or the frame
/// before).
#[must_use]
pub(crate) fn frame_damage(frame: &OutputFrame, scale: Scale, composed: Size, shown: Size, widened: bool) -> Vec<Rect> {
    let target = Rect::new(0, 0, composed.width, composed.height);
    if widened {
        return vec![Rect::new(0, 0, shown.width, shown.height)];
    }
    frame
        .damage()
        .iter()
        .filter_map(|rect| {
            let physical = physical_rect(*rect, scale);
            Rect::new(
                physical.loc.x,
                physical.loc.y,
                physical.size.w.max(0).unsigned_abs(),
                physical.size.h.max(0).unsigned_abs(),
            )
            .intersection(target)
        })
        .map(|rect| shown_rect(rect, composed, shown))
        .collect()
}

/// `rect`, a rectangle of the target, as the rectangle of the buffer that covers every pixel it averages into.
#[must_use]
fn shown_rect(rect: Rect, composed: Size, shown: Size) -> Rect {
    if composed == shown {
        return rect;
    }
    let along = |start: i32, length: u32, from: u32, to: u32| -> (i32, u32) {
        let first = u64::from(start.max(0).unsigned_abs()) * u64::from(to) / u64::from(from.max(1));
        let end = (u64::from(start.max(0).unsigned_abs()) + u64::from(length)) * u64::from(to);
        let last = end.div_ceil(u64::from(from.max(1)));
        (
            i32::try_from(first).unwrap_or(i32::MAX),
            u32::try_from(last.saturating_sub(first)).unwrap_or(u32::MAX),
        )
    };
    let (x, width) = along(rect.x, rect.width, composed.width, shown.width);
    let (y, height) = along(rect.y, rect.height, composed.height, shown.height);
    Rect::new(x, y, width, height)
}

/// How long to wait before the next rebuild of a lost renderer.
///
/// A rebuild right away, one after 100 ms and one after a second ride out a device that comes back; a device that keeps
/// going away within ten minutes is retried every five seconds, which is what an explicit renderer choice gets (the
/// session never swaps it for another renderer, and the log says each failure).
#[derive(Clone, Debug, Default)]
pub(crate) struct Recovery {
    failures: VecDeque<Instant>,
}

impl Recovery {
    /// How far back a failure still counts.
    const WINDOW: Duration = Duration::from_secs(600);
    /// The waits after the first, second and third failure in the window.
    const WAITS: [Duration; 3] = [Duration::ZERO, Duration::from_millis(100), Duration::from_secs(1)];
    /// The wait after that.
    const STEADY: Duration = Duration::from_secs(5);

    /// Records a loss, or a rebuild that failed, at `now` and says how long to wait before the next rebuild.
    pub(crate) fn failed(&mut self, now: Instant) -> Duration {
        self.failures.push_back(now);
        while self
            .failures
            .front()
            .is_some_and(|first| now.saturating_duration_since(*first) > Self::WINDOW)
        {
            self.failures.pop_front();
        }
        Self::WAITS
            .get(self.failures.len().saturating_sub(1))
            .copied()
            .unwrap_or(Self::STEADY)
    }
}

/// An output's Vulkan renderer, and what the scan-out copy needs to know about its target.
pub(crate) struct VulkanScan {
    renderer: AnyRenderer,
    damage: SlotDamage,
    /// The frame last drawn, kept only when it has a secure layer, which a capture draws redacted.
    display: Option<OutputFrame>,
    /// Whether the frame last drawn was repainted whole by a magnifier or a colour matrix.
    filtered: bool,
    /// Whether a call returned [`RenderError::Lost`]: nothing is drawn until the renderer is rebuilt.
    lost: bool,
}

impl VulkanScan {
    /// A renderer with a target of `composed` pixels on the device `share` holds, feeding `slots` buffers of `shown`
    /// pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the machine has no Vulkan device or the renderer cannot be made on it.
    pub(crate) fn new(share: &mut GpuShare, composed: Size, shown: Size, slots: usize) -> Result<Self, RenderError> {
        let mut renderer = AnyRenderer::new_sharing(RendererKind::Vulkan, composed, share)?;
        if let Some(vulkan) = renderer.vulkan_mut() {
            let device = vulkan.device();
            log::info!(area = "render", device = %device.name, kind = ?device.kind, "the Vulkan renderer is ready");
        }
        Ok(Self {
            renderer,
            damage: SlotDamage::new(slots, shown),
            display: None,
            filtered: false,
            lost: false,
        })
    }

    /// Whether the device was lost, so the output draws nothing until [`VulkanScan::rebuild`].
    pub(crate) fn is_lost(&self) -> bool {
        self.lost
    }

    /// The renderer, for what a client asks of the GPU.
    pub(crate) fn renderer(&mut self) -> &mut AnyRenderer {
        &mut self.renderer
    }

    /// The bytes the renderer holds for this output.
    pub(crate) fn footprint(&self) -> gnoblin_render_frame::footprint::Footprint {
        self.renderer.footprint()
    }

    /// Notes the renderer's answer: a lost device is permanent for this renderer.
    fn note<T>(&mut self, answer: Result<T, RenderError>) -> Result<T, RenderError> {
        if answer.as_ref().is_err_and(|error| *error == RenderError::Lost) {
            self.lost = true;
        }
        answer
    }

    /// Replaces the renderer with one on a new device after a loss, and every slot misses everything: the new target
    /// holds no pixels, and the host draws the output whole.
    ///
    /// # Errors
    ///
    /// As [`VulkanScan::new`]; the lost renderer is kept.
    pub(crate) fn rebuild(&mut self, share: &mut GpuShare, composed: Size) -> Result<(), RenderError> {
        self.renderer = AnyRenderer::new_sharing(RendererKind::Vulkan, composed, share)?;
        self.damage.add_everything();
        self.display = None;
        self.filtered = false;
        self.lost = false;
        Ok(())
    }

    /// Gives the output a target of `composed` pixels, which holds no pixels until the host draws the output whole.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`]; the target it had is kept.
    pub(crate) fn resize(&mut self, composed: Size) -> Result<(), RenderError> {
        let resized = self.renderer.resize(composed);
        self.note(resized)?;
        self.damage.add_everything();
        self.filtered = false;
        Ok(())
    }

    /// Composes `frame` into the target and notes the damage every slot now misses.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`].
    pub(crate) fn compose(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        composed: Size,
        shown: Size,
    ) -> Result<(), RenderError> {
        if self.lost {
            return Err(RenderError::Lost);
        }
        let drawn = self.renderer.draw(frame, scale);
        self.note(drawn)?;
        let widened = filters(frame) || self.filtered;
        self.filtered = filters(frame);
        self.damage.add(&frame_damage(frame, scale, composed, shown, widened));
        self.display = frame.has_secure().then(|| frame.clone());
        Ok(())
    }

    /// Writes into `dmabuf`, the buffer of `slot`, every pixel the slot missed, and returns them for the plane.
    ///
    /// `shown` is the buffer's size; an output composed on a larger target is averaged down to it first.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`], or [`RenderError::Unsupported`] for a buffer it cannot write. Nothing is marked
    /// written, so the next call writes the same rectangles.
    pub(crate) fn write_slot(
        &mut self,
        slot: usize,
        dmabuf: &Dmabuf,
        composed: Size,
        shown: Size,
    ) -> Result<Vec<Rect>, RenderError> {
        let missed = self.damage.missed(slot);
        if missed.is_empty() {
            return Ok(missed);
        }
        let image = if composed == shown {
            self.renderer.capture(true)
        } else {
            self.renderer.downsampled(shown)
        };
        let image = self.note(image)?;
        for rect in &missed {
            let copy = DmabufCopy {
                region: *rect,
                from: Point::new(0, 0),
            };
            self.renderer.copy_image_into_dmabuf(dmabuf, &image, copy)?;
        }
        self.damage.written(slot);
        Ok(missed)
    }

    /// The target read back, with the cursor composed in when `with_cursor`, and each secure layer of the frame last
    /// drawn replaced as `treatment` says.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`].
    pub(crate) fn capture(
        &mut self,
        with_cursor: bool,
        bounds: Rect,
        scale: Scale,
        treatment: SecureTreatment,
    ) -> Result<Image, RenderError> {
        self.with_redaction(bounds, scale, treatment, with_cursor, |renderer, cursor| {
            renderer.capture(cursor)
        })
    }

    /// The frame last drawn, as the display shows it at `shown` pixels, with each secure layer replaced.
    ///
    /// # Errors
    ///
    /// The renderer's [`RenderError`].
    pub(crate) fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        copy: DmabufCopy,
        with_cursor: bool,
        bounds: Rect,
        scale: Scale,
        treatment: SecureTreatment,
    ) -> Result<(), RenderError> {
        self.with_redaction(bounds, scale, treatment, with_cursor, |renderer, cursor| {
            renderer.copy_frame_into_dmabuf(dmabuf, copy, cursor)
        })
    }

    /// Reads the target through `read` with the secure layers of the frame last drawn replaced by `treatment`, and
    /// draws that frame whole again afterwards, so the next flip shows the real layers even when the read failed.
    /// The scan-out slots are not touched: the target is what the real frame left it after the second draw.
    fn with_redaction<T>(
        &mut self,
        bounds: Rect,
        scale: Scale,
        treatment: SecureTreatment,
        with_cursor: bool,
        read: impl FnOnce(&mut AnyRenderer, bool) -> Result<T, RenderError>,
    ) -> Result<T, RenderError> {
        if self.lost {
            return Err(RenderError::Lost);
        }
        let Some((replaced, whole)) = self
            .display
            .as_ref()
            .map(|display| (display.redacted(treatment, bounds), display.fully_damaged(bounds)))
        else {
            let read = read(&mut self.renderer, with_cursor);
            return self.note(read);
        };
        let taken = self
            .renderer
            .draw(&replaced, scale)
            .and_then(|()| read(&mut self.renderer, with_cursor));
        let restored = self.renderer.draw(&whole, scale);
        let taken = self.note(taken);
        let restored = self.note(restored);
        if let (Err(error), true) = (&restored, taken.is_ok()) {
            log::warn!(area = "render", %error, "the frame could not be drawn again after a capture");
        }
        let taken = taken?;
        restored?;
        Ok(taken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render_frame::frame::View;

    fn size(width: u32, height: u32) -> Size {
        Size::new(width, height)
    }

    #[test]
    fn a_new_slot_misses_everything() {
        let damage = SlotDamage::new(2, size(100, 50));
        for slot in 0..2 {
            assert_eq!(
                damage.missed(slot),
                [Rect::new(0, 0, 100, 50)],
                "slot {slot} was never written"
            );
        }
        assert!(damage.missed(2).is_empty(), "a slot that does not exist misses nothing");
    }

    #[test]
    fn a_slot_is_written_for_what_the_other_slot_drew() {
        let mut damage = SlotDamage::new(2, size(100, 50));
        // The first two frames are the first writes of both slots, which miss everything.
        damage.add(&[Rect::new(0, 0, 10, 10)]);
        damage.written(0);
        damage.add(&[Rect::new(20, 0, 10, 10)]);
        damage.written(1);
        // Slot 0 was written before the second frame, so it missed that one; slot 1 missed nothing yet.
        assert_eq!(damage.missed(0), [Rect::new(20, 0, 10, 10)]);
        assert!(damage.missed(1).is_empty());
        // The third frame lands in slot 0: the second frame's damage and its own.
        damage.add(&[Rect::new(40, 20, 5, 5)]);
        assert_eq!(damage.missed(0), [Rect::new(20, 0, 10, 10), Rect::new(40, 20, 5, 5)]);
        damage.written(0);
        // Slot 1 missed that frame too, and only that one.
        assert_eq!(damage.missed(1), [Rect::new(40, 20, 5, 5)]);
    }

    #[test]
    fn a_slot_that_was_not_written_keeps_what_it_missed() {
        let mut damage = SlotDamage::new(2, size(100, 50));
        damage.written(0);
        damage.add(&[Rect::new(1, 1, 3, 3)]);
        let first = damage.missed(0);
        // The copy failed: nothing was marked written, and the retry sees the same rectangles.
        assert_eq!(damage.missed(0), first);
        damage.add(&[Rect::new(1, 1, 3, 3)]);
        assert!(damage.missed(0).contains(&Rect::new(1, 1, 3, 3)));
    }

    #[test]
    fn damage_outside_the_buffer_is_dropped_and_scattered_damage_is_bounded() {
        let mut damage = SlotDamage::new(1, size(100, 50));
        damage.written(0);
        damage.add(&[Rect::new(200, 200, 10, 10)]);
        assert!(
            damage.missed(0).is_empty(),
            "a rectangle outside the buffer misses nothing"
        );
        damage.add(&[Rect::new(90, 40, 40, 40)]);
        assert_eq!(
            damage.missed(0),
            [Rect::new(90, 40, 10, 10)],
            "a rectangle is clipped to the buffer"
        );
        damage.written(0);
        for step in 0..=MOST_RECTS as i32 {
            damage.add(&[Rect::new(step, 0, 1, 1)]);
        }
        let missed = damage.missed(0);
        assert_eq!(missed.len(), 1, "past the limit the slot keeps one bounding rectangle");
        assert_eq!(missed[0], Rect::new(0, 0, MOST_RECTS as u32 + 1, 1));
    }

    #[test]
    fn everything_is_missed_again_after_a_new_target() {
        let mut damage = SlotDamage::new(2, size(100, 50));
        damage.written(0);
        damage.written(1);
        damage.add_everything();
        assert_eq!(damage.missed(0), [Rect::new(0, 0, 100, 50)]);
        assert_eq!(damage.missed(1), [Rect::new(0, 0, 100, 50)]);
    }

    fn damaged(rect: Rect) -> OutputFrame {
        let mut frame = OutputFrame::new();
        frame.push_damage(rect);
        frame
    }

    #[test]
    fn a_frame_damages_the_buffer_by_its_scale() {
        let frame = damaged(Rect::new(10, 5, 20, 10));
        let whole = size(400, 200);
        assert_eq!(
            frame_damage(&frame, Scale::integral(2), whole, whole, false),
            [Rect::new(20, 10, 40, 20)],
            "logical rectangles are scaled to the target's pixels"
        );
        assert_eq!(
            frame_damage(
                &damaged(Rect::new(390, 190, 50, 50)),
                Scale::integral(1),
                whole,
                whole,
                false
            ),
            [Rect::new(390, 190, 10, 10)],
            "what lies past the target is clipped"
        );
    }

    #[test]
    fn a_supersampled_target_damages_every_shown_pixel_it_touches() {
        let composed = size(400, 200);
        let shown = size(200, 100);
        // The target pixels 3..=6 across average into the shown pixels 1..=3.
        let frame = damaged(Rect::new(3, 3, 4, 1));
        assert_eq!(
            frame_damage(&frame, Scale::integral(1), composed, shown, false),
            [Rect::new(1, 1, 3, 1)]
        );
    }

    #[test]
    fn a_filter_widens_the_damage_for_two_frames() {
        let mut frame = damaged(Rect::new(0, 0, 2, 2));
        assert!(!filters(&frame));
        frame.set_view(View::new(2.0, 10.0, 10.0));
        assert!(filters(&frame), "a magnifier repaints the whole output");
        let whole = size(100, 50);
        assert_eq!(
            frame_damage(&frame, Scale::integral(1), whole, whole, true),
            [Rect::new(0, 0, 100, 50)]
        );
        let mut tinted = damaged(Rect::new(0, 0, 2, 2));
        tinted.set_colour_matrix(Some(ColourMatrix::IDENTITY));
        assert!(!filters(&tinted), "an identity matrix changes nothing");
    }

    #[test]
    fn the_rebuilds_back_off_and_settle() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        assert_eq!(
            recovery.failed(start),
            Duration::ZERO,
            "the first loss is rebuilt at once"
        );
        assert_eq!(recovery.failed(start), Duration::from_millis(100));
        assert_eq!(recovery.failed(start), Duration::from_secs(1));
        assert_eq!(
            recovery.failed(start),
            Duration::from_secs(5),
            "then every five seconds"
        );
        assert_eq!(recovery.failed(start), Duration::from_secs(5));
        // Ten quiet minutes forget the earlier failures.
        let later = start + Duration::from_secs(601);
        assert_eq!(
            recovery.failed(later),
            Duration::ZERO,
            "a device lost again much later is a first loss"
        );
    }

    #[test]
    fn a_matrix_that_changes_a_colour_widens_the_damage() {
        let mut rows = ColourMatrix::IDENTITY.rows;
        rows[0][0] = 0.5;
        let mut frame = damaged(Rect::new(0, 0, 2, 2));
        frame.set_colour_matrix(ColourMatrix::new(rows));
        assert!(filters(&frame), "a matrix that dims red repaints the whole output");
    }

    /// A renderer for a `edge` x `edge` output, or `None` on a machine with no Vulkan device, which skips loudly.
    fn scan(edge: u32) -> Option<VulkanScan> {
        let size = size(edge, edge);
        match VulkanScan::new(&mut GpuShare::default(), size, size, 2) {
            Ok(scan) => Some(scan),
            Err(RenderError::Unavailable) => {
                eprintln!("SKIPPED: no Vulkan device on this machine");
                None
            }
            Err(error) => panic!("the Vulkan renderer failed to start: {error}"),
        }
    }

    #[test]
    fn a_vulkan_output_reads_its_frame_back_and_a_capture_redacts_a_secure_layer() {
        use gnoblin_render_frame::frame::{Layer, Solid};
        use gnoblin_types::Color;
        let Some(mut scan) = scan(16) else { return };
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 16, 16));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 4, 4),
            Color::from_rgba8(255, 0, 0, 255),
        )));
        let mut secret = Solid::new(Rect::new(8, 8, 4, 4), Color::from_rgba8(0, 255, 0, 255));
        secret.secure = true;
        frame.push_layer(Layer::Solid(secret));
        scan.compose(&frame, Scale::integral(1), size(16, 16), size(16, 16))
            .expect("a frame composes");
        assert_eq!(
            scan.damage.missed(0),
            [Rect::new(0, 0, 16, 16)],
            "a slot nobody wrote misses the whole frame"
        );

        let bounds = Rect::new(0, 0, 16, 16);
        let shown = scan
            .capture(false, bounds, Scale::integral(1), SecureTreatment::Omit)
            .expect("the frame reads back");
        assert_eq!(shown.size(), size(16, 16));
        assert_eq!(
            shown.pixel(Point::new(1, 1)),
            Some([255, 0, 0, 255]),
            "the red square is in the capture"
        );
        assert_ne!(
            shown.pixel(Point::new(9, 9)),
            Some([0, 255, 0, 255]),
            "a secure layer is not in a capture"
        );
        let real = scan.renderer().capture(true).expect("the target reads back");
        assert_eq!(
            real.pixel(Point::new(9, 9)),
            Some([0, 255, 0, 255]),
            "the output is drawn whole again after a capture, so the next flip shows the real layer"
        );
    }
}

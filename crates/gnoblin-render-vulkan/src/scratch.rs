//! The images a frame draws effects through, kept between frames.
//!
//! A blur sweeps over images the size of the layer's region, and the layer is drawn into others (its group, its picture).
//! They have the sizes of the layers, so a steady session asks for the same ones every frame: the pool keeps an image
//! after its frame and hands it to the next request of the same size, and a steady frame makes none. Every frame begins with
//! a barrier from all earlier work, so an image a frame in flight drew may be drawn into again by the next one.
//!
//! An image no frame asked for in [`MAX_IDLE_FRAMES`] frames goes to the retire queue, with the last timeline value the output
//! submitted, and is destroyed when that frame has finished: a window that was resized or closed gives its images back, and a
//! frame in flight never loses one.
//!
//! Sizes are exact. A blur reads past the edge of its image and relies on the sampler to repeat the edge, which an image
//! larger than it needs would not do.

use std::rc::Rc;

use ash::vk;
use gnoblin_types::Size;

use super::context::VkContext;
use super::frame::RenderTarget;
use super::sync::RetireQueue;

/// How many frames an image may sit unused before the pool lets it go.
const MAX_IDLE_FRAMES: u32 = 120;

/// An image waiting for a request.
struct Idle {
    target: RenderTarget,
    /// The frames it has waited.
    idle: u32,
}

/// The images of past frames, by size.
#[derive(Default)]
pub(super) struct ScratchPool {
    idle: Vec<Idle>,
    /// The images made because none of the right size was waiting.
    made: u64,
}

impl ScratchPool {
    /// An image of exactly `size`: one a past frame gave back, or a new one.
    ///
    /// # Errors
    ///
    /// The driver's result when a new image cannot be made.
    pub(super) fn lease(
        &mut self,
        context: &Rc<VkContext>,
        pass: vk::RenderPass,
        size: Size,
    ) -> Result<RenderTarget, vk::Result> {
        let found = self
            .idle
            .iter()
            .position(|held| held.target.image.width() == size.width && held.target.image.height() == size.height);
        match found {
            Some(at) => Ok(self.idle.swap_remove(at).target),
            None => {
                let target = RenderTarget::scratch(context, pass, size)?;
                self.made += 1;
                Ok(target)
            }
        }
    }

    /// Takes an image back at the end of its frame.
    pub(super) fn give_back(&mut self, target: RenderTarget) {
        self.idle.push(Idle { target, idle: 0 });
    }

    /// Ages the images nobody asked for this frame, and hands the ones that waited too long to `retire`, to go when the frame
    /// with timeline value `submitted` (the newest that may use them) has finished.
    pub(super) fn end_frame(&mut self, retire: &mut RetireQueue, submitted: u64) {
        for held in &mut self.idle {
            held.idle += 1;
        }
        let (keep, expired): (Vec<Idle>, Vec<Idle>) = std::mem::take(&mut self.idle)
            .into_iter()
            .partition(|held| held.idle <= MAX_IDLE_FRAMES);
        self.idle = keep;
        for held in expired {
            retire.push(submitted, held.target);
        }
    }

    /// The bytes the pool holds: nominal, width times height times four.
    pub(super) fn bytes(&self) -> u64 {
        self.idle
            .iter()
            .map(|held| u64::from(held.target.image.width()) * u64::from(held.target.image.height()) * 4)
            .fold(0, u64::saturating_add)
    }

    /// How many images the pool holds.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.idle.len()
    }

    /// How many images the pool has made over its life, for a test that says a steady frame makes none.
    #[cfg(test)]
    pub(super) fn made(&self) -> u64 {
        self.made
    }
}

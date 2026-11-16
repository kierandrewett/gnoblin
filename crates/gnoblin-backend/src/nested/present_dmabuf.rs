//! Presenting a frame through the host's `zwp_linux_dmabuf_v1`, so a GPU session does not read every frame back.
//!
//! The shared-memory path draws the frame, reads it back with `glReadPixels` (or its Vulkan equivalent), converts it and
//! writes it into a `wl_shm` slot. When the host can import GPU buffers and the session draws with GLES, the frame is
//! instead drawn into a buffer allocated on the host's device and the host is handed that buffer: no pixel crosses the
//! bus to the CPU. The two paths share everything else (pacing, damage, scale, release), and a failure of the GPU one is
//! a frame through the other, never a lost frame.
//!
//! # Who does what
//!
//! - [`DmabufPresenter`] allocates the buffers and fills one with the frame the renderer last drew. [`GbmPresenter`]
//!   is the production one: GBM on the host's render node, and the renderer's own copy into a dmabuf
//!   ([`AnyRenderer::copy_frame_into_dmabuf`], the code a screen capture into a client's GPU buffer runs). A test
//!   installs another one ([`crate::nested::Nested::with_dmabuf_presenter`]) because a machine with no GPU cannot
//!   allocate.
//! - [`GpuSlots`] is the target's half: the buffers of one size, which of them the host holds, and the creation of the
//!   `wl_buffer`s. A buffer is created with `zwp_linux_buffer_params_v1.create`, not `create_immed`: a host that refuses
//!   an import answers `failed`, where the immediate form is a protocol error that ends the connection and so the
//!   session.
//!
//! # Failure modes first
//!
//! 1. **The host refuses a buffer.** `failed` retires the path for this output and every frame goes through shared
//!    memory from then on: `tests::a_refused_buffer_retires_the_gpu_path`, and from the host's side
//!    `a_host_that_refuses_the_buffers_gets_the_frames_through_shared_memory` in `tests/nested_dmabuf.rs`.
//! 2. **A frame is drawn into a buffer the host still holds.** A taken slot is free again only when the host releases
//!    its `wl_buffer`: `tests::a_slot_in_flight_is_not_taken_twice`.
//! 3. **A frame waits for buffers that never arrive.** Until every slot's `created` has come, frames go through shared
//!    memory: `tests::frames_use_shared_memory_until_every_buffer_exists`.
//! 4. **A resize presents the old size.** The buffers are the output's size, so a resize drops them and builds new ones
//!    (`NestedTarget::resize`), and a release of an old one is ignored by its generation:
//!    `tests::a_release_of_a_replaced_generation_frees_nothing`.
//! 5. **A frame composed larger than the display goes to the host unaveraged.** A supersampled output presents through
//!    shared memory, where the frame is averaged down; the GPU path is for a frame that is the size of the buffer:
//!    `NestedTarget::dmabuf_usable`.
//! 6. **The machine has no GPU the host's device is on.** Allocation fails and the path is retired before the first frame:
//!    `a_host_whose_device_cannot_be_allocated_on_gets_shared_memory` in `tests/nested_dmabuf.rs`, which is the case on a
//!    machine with no render node.

use std::os::fd::OwnedFd;

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer as _, Format, Fourcc};
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::utils::DeviceFd;

use gnoblin_render_frame::error::RenderError;
use gnoblin_types::{Point, Rect, Size};

use crate::DmabufCopy;
use crate::render::renderer::AnyRenderer;

/// Why a buffer could not be allocated or filled.
#[derive(Debug)]
pub enum PresentRefusal {
    /// The device the host's renderer is on cannot be opened or allocated on here.
    NoDevice(String),
    /// The allocator refused a buffer of this size and format.
    Allocation(String),
    /// The renderer cannot draw into a dmabuf (only GLES does), or refused this one.
    Fill(RenderError),
}

impl std::fmt::Display for PresentRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice(why) => write!(formatter, "no device to allocate on: {why}"),
            Self::Allocation(why) => write!(formatter, "a buffer could not be allocated: {why}"),
            Self::Fill(error) => write!(formatter, "the frame could not be drawn into the buffer: {error}"),
        }
    }
}

impl std::error::Error for PresentRefusal {}

/// What allocates the buffers the frames are presented in, and fills them.
pub trait DmabufPresenter: Send {
    /// Whether this presenter can fill a buffer from `renderer`'s frames at all, so the path is not tried for a renderer
    /// that cannot take part.
    fn can_fill(&self, renderer: &mut AnyRenderer) -> bool;

    /// Allocates a buffer of `size` in one of `formats` (in the host's order of preference), on the device the host's
    /// renderer is on when it named one.
    ///
    /// # Errors
    ///
    /// [`PresentRefusal`] naming why there is no buffer.
    fn allocate(&mut self, device: Option<u64>, size: Size, formats: &[Format]) -> Result<Dmabuf, PresentRefusal>;

    /// Draws the frame `renderer` last drew into `buffer`, the cursor included.
    ///
    /// # Errors
    ///
    /// [`PresentRefusal`] naming why the buffer was not written.
    fn fill(&mut self, renderer: &mut AnyRenderer, buffer: &Dmabuf) -> Result<(), PresentRefusal>;
}

/// The production presenter: GBM on the host's render node, and the GLES renderer's own copy.
#[derive(Default)]
pub struct GbmPresenter {
    /// The allocator and the device it is for.
    allocator: Option<(u64, GbmAllocator<DrmDeviceFd>)>,
}

impl GbmPresenter {
    /// The GBM allocator for `device`, opened once.
    fn allocator_for(&mut self, device: u64) -> Result<&mut GbmAllocator<DrmDeviceFd>, PresentRefusal> {
        if self.allocator.as_ref().is_none_or(|(known, _)| *known != device) {
            let node = DrmNode::from_dev_id(device)
                .map_err(|error| PresentRefusal::NoDevice(format!("no DRM node numbered {device:#x}: {error}")))?;
            let path = node
                .dev_path()
                .ok_or_else(|| PresentRefusal::NoDevice(format!("DRM node {device:#x} has no device path")))?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|error| PresentRefusal::NoDevice(format!("{} cannot be opened: {error}", path.display())))?;
            let gbm = GbmDevice::new(DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file))))
                .map_err(|error| PresentRefusal::NoDevice(format!("no GBM device on {}: {error}", path.display())))?;
            self.allocator = Some((device, GbmAllocator::new(gbm, GbmBufferFlags::RENDERING)));
        }
        match self.allocator.as_mut() {
            Some((_, allocator)) => Ok(allocator),
            None => Err(PresentRefusal::NoDevice("no allocator".to_owned())),
        }
    }
}

impl DmabufPresenter for GbmPresenter {
    fn can_fill(&self, renderer: &mut AnyRenderer) -> bool {
        renderer.dmabuf_support().is_some()
    }

    fn allocate(&mut self, device: Option<u64>, size: Size, formats: &[Format]) -> Result<Dmabuf, PresentRefusal> {
        // A host that names no device (version 3) is allocated on with the device of the renderer's own context.
        let device = device.ok_or_else(|| PresentRefusal::NoDevice("the host names no device".to_owned()))?;
        let allocator = self.allocator_for(device)?;
        for code in codes(formats) {
            let modifiers: Vec<_> = formats
                .iter()
                .filter(|format| format.code == code)
                .map(|format| format.modifier)
                .collect();
            match allocator.create_buffer(size.width, size.height, code, &modifiers) {
                Ok(buffer) => {
                    return buffer
                        .export()
                        .map_err(|error| PresentRefusal::Allocation(error.to_string()));
                }
                Err(error) => {
                    crate::log::debug!(area = "nested", ?code, %error, "a format could not be allocated");
                }
            }
        }
        Err(PresentRefusal::Allocation(
            "the device allocates none of the host's formats".to_owned(),
        ))
    }

    fn fill(&mut self, renderer: &mut AnyRenderer, buffer: &Dmabuf) -> Result<(), PresentRefusal> {
        let size = buffer.size();
        let region = Rect::new(0, 0, size.w.unsigned_abs(), size.h.unsigned_abs());
        renderer
            .copy_frame_into_dmabuf(
                buffer,
                DmabufCopy {
                    region,
                    from: Point::ZERO,
                },
                true,
            )
            .map_err(PresentRefusal::Fill)
    }
}

/// The formats' codes in the host's order, without repeats.
pub(crate) fn codes(formats: &[Format]) -> Vec<Fourcc> {
    let mut seen = Vec::new();
    for format in formats {
        if !seen.contains(&format.code) {
            seen.push(format.code);
        }
    }
    seen
}

/// The formats a frame can be presented in: those of `offered` that are colour formats the session draws, in the
/// order the host prefers, with an alpha format before an opaque one (the shared-memory path states alpha too). Narrowed
/// to `renderer` when it names the pairs it can draw into.
pub(crate) fn presentable(offered: &[Format], renderer: Option<&[Format]>) -> Vec<Format> {
    let mut wanted: Vec<Format> = offered
        .iter()
        .copied()
        .filter(|format| matches!(format.code, Fourcc::Argb8888 | Fourcc::Xrgb8888))
        .filter(|format| renderer.is_none_or(|drawable| drawable.contains(format)))
        .collect();
    wanted.sort_by_key(|format| format.code != Fourcc::Argb8888);
    wanted
}

/// Which of a target's buffers can be drawn into.
///
/// A buffer is `Pending` from the moment its params were sent until the host answers, `Free` once the host created it
/// and again after it released it, and `Held` while the host has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SlotState {
    /// Sent to the host, which has not answered.
    Pending,
    /// The host has it, and the session may draw into it.
    Free,
    /// Attached to the surface, and the host has not released it.
    Held,
}

/// The states of one generation of buffers.
#[derive(Debug)]
pub(crate) struct GpuSlots {
    /// The generation the buffers belong to, so a release of an older one is recognisable.
    pub generation: u32,
    states: Vec<SlotState>,
    /// Whether the host refused one: the path is over for this output.
    refused: bool,
}

impl GpuSlots {
    /// `count` buffers, all pending.
    pub fn new(generation: u32, count: usize) -> Self {
        Self {
            generation,
            states: vec![SlotState::Pending; count],
            refused: false,
        }
    }

    /// The host created buffer `slot`.
    pub fn created(&mut self, slot: u8) {
        if let Some(state) = self.states.get_mut(usize::from(slot))
            && *state == SlotState::Pending
        {
            *state = SlotState::Free;
        }
    }

    /// The host refused a buffer.
    pub fn failed(&mut self) {
        self.refused = true;
    }

    /// Whether every buffer exists, so frames can use them.
    pub fn is_ready(&self) -> bool {
        !self.refused && self.states.iter().all(|state| *state != SlotState::Pending)
    }

    /// Takes a free buffer, if the generation is ready.
    pub fn take(&mut self) -> Option<usize> {
        if !self.is_ready() {
            return None;
        }
        let index = self.states.iter().position(|state| *state == SlotState::Free)?;
        self.states[index] = SlotState::Held;
        Some(index)
    }

    /// Gives a taken buffer back without the host having held it, for a frame that was not presented.
    pub fn give_back(&mut self, slot: usize) {
        if let Some(state) = self.states.get_mut(slot)
            && *state == SlotState::Held
        {
            *state = SlotState::Free;
        }
    }

    /// The host released buffer `slot` of `generation`.
    pub fn release(&mut self, generation: u32, slot: u8) {
        if generation != self.generation {
            return;
        }
        if let Some(state) = self.states.get_mut(usize::from(slot))
            && *state == SlotState::Held
        {
            *state = SlotState::Free;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::allocator::Modifier;

    fn format(code: Fourcc, modifier: Modifier) -> Format {
        Format { code, modifier }
    }

    #[test]
    fn frames_use_shared_memory_until_every_buffer_exists() {
        let mut slots = GpuSlots::new(1, 2);
        assert_eq!(
            slots.take(),
            None,
            "nothing is drawn into a buffer the host has not created"
        );
        slots.created(0);
        assert_eq!(slots.take(), None, "one of two is not every one");
        slots.created(1);
        assert_eq!(slots.take(), Some(0));
    }

    #[test]
    fn a_slot_in_flight_is_not_taken_twice() {
        let mut slots = GpuSlots::new(1, 2);
        slots.created(0);
        slots.created(1);
        assert_eq!(slots.take(), Some(0));
        assert_eq!(slots.take(), Some(1));
        assert_eq!(slots.take(), None, "both are with the host");
        slots.release(1, 0);
        assert_eq!(slots.take(), Some(0));
    }

    #[test]
    fn a_release_of_a_replaced_generation_frees_nothing() {
        let mut slots = GpuSlots::new(2, 1);
        slots.created(0);
        assert_eq!(slots.take(), Some(0));
        slots.release(1, 0);
        assert_eq!(slots.take(), None, "the release was for buffers that were replaced");
        slots.release(2, 0);
        assert_eq!(slots.take(), Some(0));
    }

    #[test]
    fn a_refused_buffer_retires_the_gpu_path() {
        let mut slots = GpuSlots::new(1, 2);
        slots.created(0);
        slots.failed();
        slots.created(1);
        assert!(!slots.is_ready(), "a generation with a refused buffer is never ready");
        assert_eq!(slots.take(), None, "no frame goes to buffers the host refused");
    }

    #[test]
    fn a_frame_not_presented_gives_its_buffer_back() {
        let mut slots = GpuSlots::new(1, 1);
        slots.created(0);
        let taken = slots.take().expect("a buffer");
        slots.give_back(taken);
        assert_eq!(slots.take(), Some(0));
    }

    #[test]
    fn an_alpha_format_is_preferred_to_an_opaque_one_and_the_host_order_is_kept_within_a_format() {
        let offered = [
            format(Fourcc::Xrgb8888, Modifier::Linear),
            format(Fourcc::Nv12, Modifier::Linear),
            format(Fourcc::Argb8888, Modifier::Invalid),
            format(Fourcc::Argb8888, Modifier::Linear),
        ];
        assert_eq!(
            presentable(&offered, None),
            vec![
                format(Fourcc::Argb8888, Modifier::Invalid),
                format(Fourcc::Argb8888, Modifier::Linear),
                format(Fourcc::Xrgb8888, Modifier::Linear),
            ]
        );
    }

    #[test]
    fn the_renderer_narrows_the_pairs() {
        let offered = [
            format(Fourcc::Argb8888, Modifier::Invalid),
            format(Fourcc::Argb8888, Modifier::Linear),
        ];
        let drawable = [format(Fourcc::Argb8888, Modifier::Linear)];
        assert_eq!(
            presentable(&offered, Some(&drawable)),
            vec![format(Fourcc::Argb8888, Modifier::Linear)]
        );
    }

    #[test]
    fn a_device_that_does_not_exist_is_a_refusal_and_not_a_panic() {
        let mut presenter = GbmPresenter::default();
        let refused = presenter.allocate(
            Some(u64::MAX),
            Size::new(4, 4),
            &[format(Fourcc::Argb8888, Modifier::Linear)],
        );
        assert!(matches!(refused, Err(PresentRefusal::NoDevice(_))), "{refused:?}");
        let refused = presenter.allocate(None, Size::new(4, 4), &[format(Fourcc::Argb8888, Modifier::Linear)]);
        assert!(matches!(refused, Err(PresentRefusal::NoDevice(_))), "{refused:?}");
    }
}

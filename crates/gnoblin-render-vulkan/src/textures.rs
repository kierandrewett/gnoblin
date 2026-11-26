#![allow(unsafe_code)]
//! The textures of client buffers: kept between frames, shared by every output on a device, and uploaded only where they
//! changed.
//!
//! A surface that has a [`ContentKey`] keeps one image in the cache under the key's surface. What a frame has to do for it
//! follows from the key and what the cache holds:
//!
//! | The cache holds                                  | The frame                                                      |
//! | ------------------------------------------------ | -------------------------------------------------------------- |
//! | the same commit                                  | nothing: the texture is drawn as it is                         |
//! | the commit before, at the same size, and damage  | uploads the damaged rectangles, and only those                 |
//! | anything else of the same size                   | uploads the whole buffer into the image it has                 |
//! | another size or another format, or nothing       | makes a new image, uploads the whole buffer, retires the old   |
//!
//! A layer with no key (the overlay and the cursor, a solid layer's one pixel for its shader, a source nobody tracks) gets an
//! image for the frame alone, uploaded whole and retired with the frame.
//!
//! An image is destroyed through the retire queue, with the value of the last frame that used it, so a frame in flight never
//! loses a texture it samples. A texture no frame has drawn for [`IDLE_FRAMES`] frames is given back: its surface is gone.
//!
//! # Failure modes
//!
//! 1. **A texture shows an older picture than the surface has.** The cache records the commit of what it holds, and a commit
//!    it did not see in order is uploaded whole: `tests::a_commit_that_was_not_drawn_is_uploaded_whole`.
//! 2. **A partial upload leaves the damage's overlap written twice, or reads past the buffer.** The rectangles are clipped to
//!    the buffer and made disjoint first (Vulkan forbids overlapping copy regions): `tests::damage_rectangles_are_disjoint`.
//! 3. **A frame that failed leaves the cache believing it uploaded.** The textures a frame touched are dropped from the cache
//!    when it fails before it is submitted, so the next frame uploads them whole.
//! 4. **A destroyed buffer is read again.** A key at the cached commit needs no read at all, so a buffer the client freed
//!    mid-frame is drawn from its texture: `tests::a_destroyed_buffer_still_draws_its_last_picture`.

use std::collections::HashMap;
use std::rc::Rc;

use ash::vk;
use gnoblin_render_frame::frame::ContentKey;
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;
use gnoblin_types::Rect;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;

use super::context::{SHM_FORMAT, TARGET_FORMAT, VkContext};
use super::dmabuf;
use super::memory::{RenderCounters, VkImage};
use super::pipeline::{LongSet, Pipelines};
use super::plan::TextureSlot;
use super::sync::{InFlightFrame, RetireQueue};
use crate::log;

/// The alignment of a texture's bytes inside the staging buffer. A buffer-to-image copy needs a multiple of the texel size.
pub(super) const STAGING_ALIGN: usize = 16;

/// How many frames a texture may go undrawn before it is given back.
pub(super) const IDLE_FRAMES: u64 = 120;

/// More damage rectangles than this, or damage that covers half the buffer, is uploaded as the whole buffer.
const MAX_DAMAGE_RECTS: usize = 32;

/// What a texture is made from.
pub(super) enum TextureSource<'a> {
    /// A `wl_shm` buffer, read from the client's pool.
    Shm(&'a WlBuffer),
    /// A bitmap of premultiplied `r, g, b, a` pixels: a snapshot, the overlay, the cursor.
    Bitmap(&'a Image),
    /// One premultiplied pixel: a solid layer's colour, which its shader pass reads as the layer's own.
    Pixel([u8; 4]),
    /// A client's linear `zwp_linux_dmabuf_v1` buffer, mapped and read (`dmabuf.rs`).
    Dmabuf(&'a Dmabuf),
}

/// How a source is laid out as a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextureShape {
    width: u32,
    height: u32,
    format: vk::Format,
    /// Whether the fourth byte is padding, read as one.
    opaque: bool,
}

impl TextureSource<'_> {
    /// The texture the source makes, from its metadata alone.
    fn shape(&self) -> Result<TextureShape, String> {
        Ok(match self {
            Self::Shm(buffer) => {
                let shape = shm::shape(buffer).map_err(|error| error.to_string())?;
                TextureShape {
                    width: shape.width,
                    height: shape.height,
                    format: SHM_FORMAT,
                    opaque: shape.opaque,
                }
            }
            Self::Bitmap(image) => {
                let size = image.size();
                TextureShape {
                    width: size.width,
                    height: size.height,
                    format: TARGET_FORMAT,
                    opaque: false,
                }
            }
            Self::Pixel(_) => TextureShape {
                width: 1,
                height: 1,
                format: TARGET_FORMAT,
                opaque: false,
            },
            Self::Dmabuf(dmabuf) => {
                let linear = dmabuf::linear(dmabuf).map_err(|error| error.to_string())?;
                TextureShape {
                    width: linear.width,
                    height: linear.height,
                    format: linear.layout.format,
                    opaque: linear.layout.opaque,
                }
            }
        })
    }

    /// Whether the buffer is read bottom row first, which makes its damage rectangles (in buffer rows) the wrong ones.
    fn inverted(&self) -> bool {
        matches!(self, Self::Dmabuf(dmabuf) if dmabuf.y_inverted())
    }

    /// Copies `region` of the source, rows packed, into `out`, which is `region`'s area in bytes. Returns false when it
    /// cannot: a buffer that cannot be read.
    fn fill(&self, region: Rect, out: &mut [u8]) -> bool {
        match self {
            Self::Shm(buffer) => match shm::read_region(buffer, region, out) {
                Ok(_) => true,
                Err(error) => {
                    log::warn!(area = "render", %error, "a client buffer could not be read; the layer is skipped");
                    false
                }
            },
            Self::Bitmap(image) => {
                let row = region.width as usize * 4;
                let x = region.x as usize * 4;
                for (line, out) in out.chunks_exact_mut(row).enumerate() {
                    let start = (region.y as usize + line) * image.stride() + x;
                    let Some(pixels) = image.data().get(start..start + row) else {
                        return false;
                    };
                    out.copy_from_slice(pixels);
                }
                true
            }
            Self::Pixel(pixel) => {
                out.copy_from_slice(pixel);
                true
            }
            Self::Dmabuf(dmabuf) => {
                let read = dmabuf::linear(dmabuf).and_then(|shape| dmabuf::read_region(dmabuf, &shape, region, out));
                match read {
                    Ok(()) => true,
                    Err(error) => {
                        log::warn!(area = "render", %error, "a client's dmabuf could not be read; the layer is skipped");
                        false
                    }
                }
            }
        }
    }
}

/// A texture a frame draws: where it goes, what it is made of and which surface it is, when it has one.
pub(super) struct Wanted<'a> {
    pub slot: TextureSlot,
    pub source: TextureSource<'a>,
    pub key: Option<&'a ContentKey>,
}

/// What a texture takes from the buffer this frame.
enum TextureDecision {
    /// The texture is as the surface is.
    Current,
    /// The image the cache holds takes these rectangles.
    Update(Vec<Rect>),
    /// The image the cache holds takes the whole buffer.
    Whole,
    /// A new image takes the whole buffer; an entry of the surface that is not of this shape goes.
    Replace,
    /// An image for this frame alone takes the whole buffer.
    Transient,
}

/// A copy from the staging buffer into an image, recorded at the start of the frame.
pub(super) struct Copy {
    pub image: vk::Image,
    /// Whether the image has no contents yet, so it is transitioned from `UNDEFINED`, and not from being sampled.
    pub fresh: bool,
    pub regions: Vec<vk::BufferImageCopy>,
}

/// What [`TextureCache::prepare`] made of a frame's textures.
#[derive(Default)]
pub(super) struct Prepared {
    /// The descriptor set that binds each texture.
    pub sets: Vec<(TextureSlot, vk::DescriptorSet)>,
    /// The size of each texture, which is what the plan lays the layers out by.
    pub sizes: Vec<(TextureSlot, (u32, u32))>,
    /// The uploads to record.
    pub copies: Vec<Copy>,
    /// The images made for this frame alone, retired with it.
    pub transient: Vec<VkImage>,
    /// The surfaces whose textures this frame touched, dropped from the cache if the frame fails.
    pub keys: Vec<u64>,
}

/// What the cache needs of the frame being recorded.
pub(super) struct CacheFrame<'a> {
    pub context: &'a Rc<VkContext>,
    pub pipelines: &'a Pipelines,
    pub slot: &'a mut InFlightFrame,
    pub retire: &'a mut RetireQueue,
    /// The timeline value the frame will signal: the last use of every texture it touches.
    pub value: u64,
}

/// A texture the cache holds.
struct CachedTexture {
    image: VkImage,
    set: LongSet,
    opaque: bool,
    /// The commit of the surface the image holds.
    commit: u64,
    used_frame: u64,
    used_value: u64,
}

impl CachedTexture {
    fn matches(&self, shape: &TextureShape) -> bool {
        self.image.width() == shape.width
            && self.image.height() == shape.height
            && self.image.format() == shape.format
            && self.opaque == shape.opaque
    }
}

/// The textures of a device, by the surface they show.
#[derive(Default)]
pub(super) struct TextureCache {
    entries: HashMap<u64, CachedTexture>,
    /// The number of frames prepared: the clock the idle textures are given back by.
    frame: u64,
}

impl TextureCache {
    /// How many textures the cache holds.
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The bytes the cached images nominally hold.
    pub(super) fn bytes(&self) -> u64 {
        self.entries
            .values()
            .map(|entry| entry.image.bytes())
            .fold(0, u64::saturating_add)
    }

    /// Gives back every texture, through the retire queue.
    pub(super) fn clear(&mut self, retire: &mut RetireQueue) {
        for (_, entry) in self.entries.drain() {
            retire.push(entry.used_value, entry);
        }
    }

    /// Drops the textures of `surfaces`, which a frame that failed may have left holding what it never uploaded.
    pub(super) fn invalidate(&mut self, surfaces: &[u64], retire: &mut RetireQueue) {
        for surface in surfaces {
            if let Some(entry) = self.entries.remove(surface) {
                retire.push(entry.used_value, entry);
            }
        }
    }

    /// Gives back the textures no frame has drawn for [`IDLE_FRAMES`] frames.
    fn evict(&mut self, retire: &mut RetireQueue) {
        let clock = self.frame;
        let idle: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.used_frame + IDLE_FRAMES < clock)
            .map(|(surface, _)| *surface)
            .collect();
        self.invalidate(&idle, retire);
    }

    /// What a texture takes from its buffer, or `None` when its layer cannot be drawn.
    fn decide(&self, wanted: &Wanted<'_>) -> Option<(TextureDecision, TextureShape)> {
        let entry = wanted.key.and_then(|key| self.entries.get(&key.surface));
        if let (Some(key), Some(entry)) = (wanted.key, entry)
            && entry.commit == key.commit
        {
            let shape = TextureShape {
                width: entry.image.width(),
                height: entry.image.height(),
                format: entry.image.format(),
                opaque: entry.opaque,
            };
            return Some((TextureDecision::Current, shape));
        }
        let shape = match wanted.source.shape() {
            Ok(shape) => shape,
            Err(error) => {
                log::warn!(area = "render", %error, "a client buffer could not be read; the layer is skipped");
                return None;
            }
        };
        let Some(key) = wanted.key else {
            return Some((TextureDecision::Transient, shape));
        };
        let Some(entry) = entry else {
            return Some((TextureDecision::Replace, shape));
        };
        if !entry.matches(&shape) {
            return Some((TextureDecision::Replace, shape));
        }
        if key.commit == entry.commit + 1
            && !wanted.source.inverted()
            && let Some(damage) = &key.damage
        {
            let rects = disjoint(damage, shape.width, shape.height);
            if rects.is_empty() {
                // A commit that changed no pixel: the image is current, and the cache learns the commit.
                return Some((TextureDecision::Current, shape));
            }
            let area: u64 = rects
                .iter()
                .map(|rect| u64::from(rect.width) * u64::from(rect.height))
                .sum();
            if rects.len() <= MAX_DAMAGE_RECTS && area * 2 < u64::from(shape.width) * u64::from(shape.height) {
                return Some((TextureDecision::Update(rects), shape));
            }
        }
        Some((TextureDecision::Whole, shape))
    }

    /// Makes every texture of a frame ready: finds or makes its image, copies what changed into the frame's staging buffer
    /// and says which copies the frame records.
    ///
    /// # Errors
    ///
    /// The driver's result, when the staging buffer cannot grow.
    pub(super) fn prepare(
        &mut self,
        wanted: Vec<Wanted<'_>>,
        frame: &mut CacheFrame<'_>,
    ) -> Result<Prepared, vk::Result> {
        self.frame += 1;
        self.evict(frame.retire);
        let decisions: Vec<(Wanted<'_>, TextureDecision, TextureShape)> = wanted
            .into_iter()
            .filter_map(|wanted| {
                let (decision, shape) = self.decide(&wanted)?;
                Some((wanted, decision, shape))
            })
            .collect();
        let staging: usize = decisions
            .iter()
            .map(|(_, decision, shape)| match decision {
                TextureDecision::Current => 0,
                TextureDecision::Update(rects) => rects
                    .iter()
                    .map(|rect| (rect.width as usize * rect.height as usize * 4).next_multiple_of(STAGING_ALIGN))
                    .sum(),
                TextureDecision::Whole | TextureDecision::Replace | TextureDecision::Transient => {
                    (shape.width as usize * shape.height as usize * 4).next_multiple_of(STAGING_ALIGN)
                }
            })
            .sum();
        InFlightFrame::ensure(
            frame.context,
            &mut frame.slot.staging,
            staging,
            vk::BufferUsageFlags::TRANSFER_SRC,
        )?;
        let mut prepared = Prepared::default();
        let mut offset = 0;
        for (wanted, decision, shape) in decisions {
            let used = self.apply(&wanted, &decision, &shape, offset, frame, &mut prepared);
            offset += used;
        }
        Ok(prepared)
    }

    /// Carries out one texture's decision. Returns the staging bytes it took.
    fn apply(
        &mut self,
        wanted: &Wanted<'_>,
        decision: &TextureDecision,
        shape: &TextureShape,
        offset: usize,
        frame: &mut CacheFrame<'_>,
        prepared: &mut Prepared,
    ) -> usize {
        let size = (shape.width, shape.height);
        let whole = Rect::new(0, 0, shape.width, shape.height);
        match decision {
            TextureDecision::Current => {
                let (Some(key), Some(entry)) = (
                    wanted.key,
                    wanted.key.and_then(|key| self.entries.get_mut(&key.surface)),
                ) else {
                    return 0;
                };
                entry.commit = key.commit;
                self.touch(key.surface, wanted.slot, frame, prepared);
                0
            }
            TextureDecision::Update(rects) => {
                let Some(key) = wanted.key else { return 0 };
                let mut used = 0;
                let mut regions = Vec::with_capacity(rects.len());
                for rect in rects {
                    let bytes = rect.width as usize * rect.height as usize * 4;
                    let Some(out) = frame.slot.staging.as_mut_slice(offset + used, bytes) else {
                        return used;
                    };
                    if !wanted.source.fill(*rect, out) {
                        return used;
                    }
                    regions.push(region(rect, offset + used));
                    used += bytes.next_multiple_of(STAGING_ALIGN);
                }
                let Some(entry) = self.entries.get_mut(&key.surface) else {
                    return used;
                };
                entry.commit = key.commit;
                prepared.copies.push(Copy {
                    image: entry.image.raw(),
                    fresh: false,
                    regions,
                });
                RenderCounters::add(&frame.context.memory.counters.uploaded, used as u64);
                RenderCounters::add(&frame.context.memory.counters.uploads, 1);
                self.touch(key.surface, wanted.slot, frame, prepared);
                used
            }
            TextureDecision::Whole => {
                let Some(key) = wanted.key else { return 0 };
                let bytes = shape.width as usize * shape.height as usize * 4;
                let Some(out) = frame.slot.staging.as_mut_slice(offset, bytes) else {
                    return 0;
                };
                if !wanted.source.fill(whole, out) {
                    return 0;
                }
                let Some(entry) = self.entries.get_mut(&key.surface) else {
                    return 0;
                };
                entry.commit = key.commit;
                prepared.copies.push(Copy {
                    image: entry.image.raw(),
                    fresh: false,
                    regions: vec![region(&whole, offset)],
                });
                RenderCounters::add(&frame.context.memory.counters.uploaded, bytes as u64);
                RenderCounters::add(&frame.context.memory.counters.uploads, 1);
                self.touch(key.surface, wanted.slot, frame, prepared);
                bytes.next_multiple_of(STAGING_ALIGN)
            }
            TextureDecision::Replace | TextureDecision::Transient => {
                let bytes = shape.width as usize * shape.height as usize * 4;
                let Some(out) = frame.slot.staging.as_mut_slice(offset, bytes) else {
                    return 0;
                };
                if !wanted.source.fill(whole, out) {
                    return 0;
                }
                let image = match VkImage::new(
                    frame.context,
                    shape.width,
                    shape.height,
                    shape.format,
                    vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                    shape.opaque,
                ) {
                    Ok(image) => image,
                    Err(result) => {
                        log::warn!(
                            area = "render",
                            ?result,
                            slot = ?wanted.slot,
                            "a texture could not be made; the layer is skipped"
                        );
                        // A texture the surface had is older than its buffer now, and is not drawn.
                        if let Some(key) = wanted.key {
                            self.invalidate(&[key.surface], frame.retire);
                        }
                        return 0;
                    }
                };
                let copy = Copy {
                    image: image.raw(),
                    fresh: true,
                    regions: vec![region(&whole, offset)],
                };
                let sampler = if wanted.slot.nearest() {
                    frame.pipelines.nearest
                } else {
                    frame.pipelines.linear
                };
                match (decision, wanted.key) {
                    (TextureDecision::Replace, Some(key)) => {
                        let set = match frame.pipelines.long_sets.allocate(frame.pipelines.texture_layout) {
                            Ok(set) => set,
                            Err(result) => {
                                log::warn!(
                                    area = "render",
                                    ?result,
                                    "a texture's descriptor set could not be made; the layer is skipped"
                                );
                                return 0;
                            }
                        };
                        write_texture_set(frame.context, set.set, image.view(), sampler);
                        if let Some(old) = self.entries.remove(&key.surface) {
                            frame.retire.push(old.used_value, old);
                        }
                        self.entries.insert(
                            key.surface,
                            CachedTexture {
                                image,
                                set,
                                opaque: shape.opaque,
                                commit: key.commit,
                                used_frame: self.frame,
                                used_value: frame.value,
                            },
                        );
                        self.touch(key.surface, wanted.slot, frame, prepared);
                    }
                    _ => {
                        let set = match frame.slot.arena.allocate(frame.pipelines.texture_layout) {
                            Ok(set) => set,
                            Err(result) => {
                                log::warn!(
                                    area = "render",
                                    ?result,
                                    "a texture's descriptor set could not be made; the layer is skipped"
                                );
                                return 0;
                            }
                        };
                        write_texture_set(frame.context, set, image.view(), sampler);
                        prepared.sets.push((wanted.slot, set));
                        prepared.sizes.push((wanted.slot, size));
                        prepared.transient.push(image);
                    }
                }
                prepared.copies.push(copy);
                RenderCounters::add(&frame.context.memory.counters.uploaded, bytes as u64);
                RenderCounters::add(&frame.context.memory.counters.uploads, 1);
                bytes.next_multiple_of(STAGING_ALIGN)
            }
        }
    }

    /// Records that this frame draws the texture of `surface`.
    fn touch(&mut self, surface: u64, slot: TextureSlot, frame: &CacheFrame<'_>, prepared: &mut Prepared) {
        let Some(entry) = self.entries.get_mut(&surface) else {
            return;
        };
        entry.used_frame = self.frame;
        entry.used_value = frame.value;
        prepared.sets.push((slot, entry.set.set));
        prepared.sizes.push((slot, (entry.image.width(), entry.image.height())));
        prepared.keys.push(surface);
    }
}

/// Points a texture's descriptor set at an image view and a sampler.
pub(super) fn write_texture_set(
    context: &VkContext,
    set: vk::DescriptorSet,
    view: vk::ImageView,
    sampler: vk::Sampler,
) {
    let image = [vk::DescriptorImageInfo::default()
        .image_view(view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let sampler = [vk::DescriptorImageInfo::default().sampler(sampler)];
    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&image),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&sampler),
    ];
    // SAFETY: the set, view and sampler are live and the layout matches the set's.
    unsafe { context.device.update_descriptor_sets(&writes, &[]) };
}

/// The colour layers of the first mip level.
pub(super) fn colour_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// The copy of `rect` of a texture from packed rows at `offset` in the staging buffer.
fn region(rect: &Rect, offset: usize) -> vk::BufferImageCopy {
    vk::BufferImageCopy::default()
        .buffer_offset(offset as u64)
        .image_subresource(colour_layers())
        .image_offset(vk::Offset3D {
            x: rect.x,
            y: rect.y,
            z: 0,
        })
        .image_extent(vk::Extent3D {
            width: rect.width,
            height: rect.height,
            depth: 1,
        })
}

/// `rects` clipped to a buffer of `width` by `height` and made disjoint, which Vulkan asks of the regions of one copy.
///
/// What is left of a rectangle after the ones before it are taken away is split into at most four, so the result covers
/// exactly the union of the input.
pub(super) fn disjoint(rects: &[Rect], width: u32, height: u32) -> Vec<Rect> {
    let bounds = Rect::new(0, 0, width, height);
    let mut out: Vec<Rect> = Vec::new();
    for rect in rects {
        let Some(clipped) = rect.intersection(bounds) else {
            continue;
        };
        let mut pieces = vec![clipped];
        for taken in &out {
            pieces = pieces.into_iter().flat_map(|piece| subtract(piece, *taken)).collect();
            if pieces.is_empty() {
                break;
            }
        }
        out.extend(pieces);
    }
    out
}

/// What is left of `rect` when `hole` is taken out: up to four rectangles, none of which overlaps the hole.
fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
    let Some(inside) = rect.intersection(hole) else {
        return vec![rect];
    };
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
    let (inside_left, inside_top) = (i64::from(inside.x), i64::from(inside.y));
    let (inside_right, inside_bottom) = (
        inside_left + i64::from(inside.width),
        inside_top + i64::from(inside.height),
    );
    let make = |x0: i64, y0: i64, x1: i64, y1: i64| {
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
    };
    [
        make(left, top, right, inside_top),
        make(left, inside_bottom, right, bottom),
        make(left, inside_top, inside_left, inside_bottom),
        make(inside_right, inside_top, right, inside_bottom),
    ]
    .into_iter()
    .flatten()
    .collect()
}

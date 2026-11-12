//! A GPU the tests do not have: what the compositor asks of a renderer about client buffers, answered from files.
//!
//! The Agent37 build instances have no DRM device (`/dev/dri`, `/dev/udmabuf` and `/dev/dma_heap` are absent in their
//! gVisor sandbox), so no process there can allocate a dmabuf, and no driver can import one or render into one. This
//! stand-in is the part of the compositor a GPU is, and only that part: it names the device and the formats a renderer
//! would, accepts the buffers a client sends, and when the compositor asks it to write a frame or a picture into a
//! client's buffer it writes the bytes into the buffer's plane. The plane is an ordinary file the test created and
//! still holds, so the test reads the result back through the file, which is the same thing a client reading its
//! dmabuf would see.
//!
//! **Everything that runs on top of it is real**: the Wayland protocol, the capture sessions, the lock, the
//! secure-window masking, the View pictures, the queue the loop answers GPU captures from. **What it is not**: a
//! GL blit. That a frame is copied into a GPU buffer the right way up, with the right crop, with the cursor in or out,
//! is proved by `gnoblin-render-gles`'s own tests on a texture of the renderer's and by `tests/dmabuf_capture_gpu.rs`
//! on a real device; neither can run on a machine with no GPU, and this file does not pretend to.
//!
//! The pixel layout it writes is linear `argb8888` / `xrgb8888`, which is the layout of the planes the tests make.

#![allow(dead_code)]

use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_backend::{DmabufCopy, DmabufCopyError, DmabufImportError, DmabufProvider, DmabufSupport};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{Point, Rect, Size};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Format, Fourcc, Modifier};

/// The render node number the stand-in reports: major 226 (DRM), minor 128.
pub const DEVICE: u64 = (226 << 8) | 128;

/// The pairs a client may share.
#[must_use]
pub fn importable() -> Vec<Format> {
    vec![
        Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Invalid,
        },
    ]
}

/// The pairs a capture may be written into, unless a test says otherwise: `argb8888` and `xrgb8888`, linear.
#[must_use]
pub fn renderable() -> Vec<Format> {
    importable()
        .into_iter()
        .filter(|format| format.modifier == Modifier::Linear)
        .collect()
}

/// One copy the compositor made into a client's buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The pixels written, in the buffer's.
    pub region: Rect,
    /// Where the region started in the source.
    pub from: Point,
    /// The buffer's size.
    pub buffer: Size,
    /// The buffer's format.
    pub format: Fourcc,
    /// The size of the picture the copy read, which is the frame of an output or a View's picture.
    pub source: Size,
}

/// What the tests set and read of the stand-in while the session runs.
#[derive(Debug)]
struct Shared {
    /// The pairs a capture may be written into, or `None` for a GPU that cannot render into a client's buffer.
    capture: Option<Vec<Format>>,
    /// Whether the stand-in refuses every write, as a driver that fails the copy would.
    refuse_writes: bool,
    /// Every copy it made.
    written: Vec<Written>,
    /// How many buffers it read back.
    reads: usize,
    /// How many buffers it imported.
    imports: usize,
}

/// The handle a test keeps while the session owns the stand-in.
#[derive(Clone, Debug)]
pub struct Controls(Arc<Mutex<Shared>>);

fn locked(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Controls {
    /// The copies the compositor made into client buffers so far.
    #[must_use]
    pub fn written(&self) -> Vec<Written> {
        locked(&self.0).written.clone()
    }

    /// How many client buffers the compositor read back, for a View.
    #[must_use]
    pub fn reads(&self) -> usize {
        locked(&self.0).reads
    }

    /// How many client buffers it imported.
    #[must_use]
    pub fn imports(&self) -> usize {
        locked(&self.0).imports
    }

    /// Makes the GPU refuse every write, or stop refusing.
    pub fn refuse_writes(&self, refuse: bool) {
        locked(&self.0).refuse_writes = refuse;
    }
}

/// The stand-in a session is built with.
pub struct Mock {
    shared: Arc<Mutex<Shared>>,
}

impl Mock {
    /// A GPU that imports client buffers and renders captures into `capture`, with the handle the test keeps.
    #[must_use]
    pub fn new(capture: Option<Vec<Format>>) -> (Self, Controls) {
        let shared = Arc::new(Mutex::new(Shared {
            capture,
            refuse_writes: false,
            written: Vec::new(),
            reads: 0,
            imports: 0,
        }));
        (
            Self {
                shared: Arc::clone(&shared),
            },
            Controls(shared),
        )
    }

    /// A GPU that renders captures into the layouts a test makes.
    #[must_use]
    pub fn capable() -> (Self, Controls) {
        Self::new(Some(renderable()))
    }

    /// A GPU that imports client buffers and cannot render into one.
    #[must_use]
    pub fn import_only() -> (Self, Controls) {
        Self::new(None)
    }
}

/// The plane file of a dmabuf of one plane, its offset and its stride.
fn plane(dmabuf: &Dmabuf) -> Result<(std::fs::File, u64, usize), DmabufCopyError> {
    let fd: OwnedFd = dmabuf
        .handles()
        .next()
        .ok_or(DmabufCopyError::Unsupported)?
        .try_clone_to_owned()
        .map_err(|_| DmabufCopyError::Refused(RenderError::Failed))?;
    let offset = dmabuf.offsets().next().ok_or(DmabufCopyError::Unsupported)?;
    let stride = dmabuf.strides().next().ok_or(DmabufCopyError::Unsupported)?;
    Ok((std::fs::File::from(fd), u64::from(offset), stride as usize))
}

/// The buffer's size, as unsigned numbers.
fn size_of(dmabuf: &Dmabuf) -> Size {
    let size = dmabuf.size();
    Size::new(size.w.unsigned_abs(), size.h.unsigned_abs())
}

impl DmabufProvider for Mock {
    fn support(&mut self) -> Option<DmabufSupport> {
        Some(DmabufSupport::new(Some(DEVICE), importable()))
    }

    fn import(&mut self, _dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        locked(&self.shared).imports += 1;
        Ok(())
    }

    fn capture_support(&mut self) -> Option<DmabufSupport> {
        locked(&self.shared)
            .capture
            .clone()
            .map(|formats| DmabufSupport::new(Some(DEVICE), formats))
    }

    fn write(&mut self, dmabuf: &Dmabuf, picture: &Image, copy: DmabufCopy) -> Result<(), DmabufCopyError> {
        if locked(&self.shared).refuse_writes {
            return Err(DmabufCopyError::Refused(RenderError::Failed));
        }
        let buffer = size_of(dmabuf);
        let (file, offset, stride) = plane(dmabuf)?;
        // The same meaning the renderers give `region` and `from`: buffer pixel (x, y) of the region is the source's
        // pixel (from.x + x, from.y + y), clipped to both.
        let written = copy
            .region
            .intersection(Rect::from_size(buffer))
            .ok_or(DmabufCopyError::NoFrame)?;
        let wanted = Rect::new(
            copy.from.x + written.x,
            copy.from.y + written.y,
            written.width,
            written.height,
        );
        let source = wanted
            .intersection(Rect::from_size(picture.size()))
            .ok_or(DmabufCopyError::NoFrame)?;
        let opaque = dmabuf.format().code == Fourcc::Xrgb8888;
        for row in 0..source.height as i32 {
            let mut bytes = Vec::with_capacity(source.width as usize * 4);
            for column in 0..source.width as i32 {
                let [red, green, blue, alpha] = picture
                    .colour_at(source.x + column, source.y + row)
                    .ok_or(DmabufCopyError::NoFrame)?;
                // `argb8888` is B, G, R, A in memory.
                bytes.extend_from_slice(&[blue, green, red, if opaque { 255 } else { alpha }]);
            }
            let y = (source.y - copy.from.y + row) as u64;
            let x = (source.x - copy.from.x) as u64;
            file.write_all_at(&bytes, offset + y * stride as u64 + x * 4)
                .map_err(|_| DmabufCopyError::Refused(RenderError::Failed))?;
        }
        locked(&self.shared).written.push(Written {
            region: copy.region,
            from: copy.from,
            buffer,
            format: dmabuf.format().code,
            source: picture.size(),
        });
        Ok(())
    }

    fn read(&mut self, dmabuf: &Dmabuf) -> Result<Image, DmabufCopyError> {
        locked(&self.shared).reads += 1;
        let size = size_of(dmabuf);
        let (file, offset, stride) = plane(dmabuf)?;
        let opaque = dmabuf.format().code == Fourcc::Xrgb8888;
        let mut rgba = Vec::with_capacity(size.width as usize * size.height as usize * 4);
        let mut row = vec![0u8; size.width as usize * 4];
        for y in 0..u64::from(size.height) {
            file.read_exact_at(&mut row, offset + y * stride as u64)
                .map_err(|_| DmabufCopyError::Refused(RenderError::Failed))?;
            for pixel in row.as_chunks::<4>().0 {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], if opaque { 255 } else { pixel[3] }]);
            }
        }
        Image::new(size, size.width as usize * 4, rgba).ok_or(DmabufCopyError::Refused(RenderError::Failed))
    }
}

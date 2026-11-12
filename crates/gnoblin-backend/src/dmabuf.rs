//! What a backend tells the compositor about client GPU buffers (`zwp_linux_dmabuf_v1`).
//!
//! The protocol is a promise to clients: "buffers in these formats, allocated on this device,
//! will be drawn". Only the backend knows whether the promise can be kept, because only it owns
//! the renderer that imports the buffers. So the compositor asks twice:
//!
//! 1. At start, [`Backend::dmabuf`](crate::Backend::dmabuf): *can* you import, and in which formats
//!    and on which device? `None` means the global is never created, and a client falls back to
//!    shared memory. The software renderer and a session with no outputs answer `None`.
//! 2. For every buffer a client sends, [`Backend::import_dmabuf`](crate::Backend::import_dmabuf):
//!    can *this* one be imported? The answer decides whether the client's `create` succeeds or
//!    fails, so a buffer the GPU cannot read is refused up front and is never a black window.
//!
//! # Failure modes
//!
//! 1. **A software session advertises a protocol it cannot draw.** The software renderer reports
//!    nothing: `tests::the_software_renderer_offers_no_dmabuf` in `headless.rs`.
//! 2. **A format the display cannot sample is offered.** The list is the renderer's own: see
//!    `gnoblin_render_gles::Gles::dmabuf_formats`.
//! 3. **A buffer in an unadvertised format or modifier is imported.** [`DmabufSupport::offers`] is
//!    the check every import runs first, because the protocol's own validation knows the format
//!    code and not the modifier.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Modifier};

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Point, Rect};

/// What a backend can import: the format and modifier pairs, and the device they live on.
#[derive(Clone, Debug)]
pub struct DmabufSupport {
    /// The `dev_t` of the render node the compositor draws with. With a device, clients get
    /// version 4 feedback (`main_device` and a format table); without one (a software
    /// rasteriser, a surfaceless context) the compositor serves version 3, which names formats and
    /// no device.
    pub main_device: Option<u64>,
    /// The pairs a client may allocate in.
    pub formats: FormatSet,
}

impl DmabufSupport {
    /// Support for `formats` on `main_device`.
    #[must_use]
    pub fn new(main_device: Option<u64>, formats: impl IntoIterator<Item = Format>) -> Self {
        Self {
            main_device,
            formats: formats.into_iter().collect(),
        }
    }

    /// Whether `format` and `modifier` were offered to clients. A buffer outside the set is never
    /// imported, whatever the driver would say.
    #[must_use]
    pub fn offers(&self, format: Format) -> bool {
        self.formats.contains(&format)
    }

    /// Whether the buffer's format and modifier were offered.
    #[must_use]
    pub fn offers_buffer(&self, dmabuf: &Dmabuf) -> bool {
        use smithay::backend::allocator::Buffer as _;
        self.offers(dmabuf.format())
    }

    /// Whether a buffer's plane count is possible for its modifier.
    ///
    /// A linear or implicit-modifier buffer of a packed colour format is one plane; extra planes
    /// belong to compression or tiling metadata a vendor modifier names, and only the driver can
    /// judge those. Checking the case the compositor can decide spares the driver a malformed
    /// buffer and gives the client a clear failure.
    #[must_use]
    pub fn plane_count_fits(dmabuf: &Dmabuf) -> bool {
        use smithay::backend::allocator::Buffer as _;
        let plain = matches!(dmabuf.format().modifier, Modifier::Linear | Modifier::Invalid);
        !plain || dmabuf.num_planes() == 1
    }

    /// The checks that need no driver, in the order a backend runs them before it asks the driver.
    ///
    /// # Errors
    ///
    /// [`DmabufImportError::NotOffered`] for a pair outside the offer and
    /// [`DmabufImportError::BadPlanes`] for a plane count the modifier cannot have.
    pub fn check(&self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        if !self.offers_buffer(dmabuf) {
            return Err(DmabufImportError::NotOffered);
        }
        if !Self::plane_count_fits(dmabuf) {
            return Err(DmabufImportError::BadPlanes);
        }
        Ok(())
    }
}

/// Why a buffer was not imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmabufImportError {
    /// The backend serves no dmabuf import at all.
    Unsupported,
    /// The format and modifier were not among those offered.
    NotOffered,
    /// The buffer's planes cannot be right for its modifier.
    BadPlanes,
    /// The driver refused the buffer.
    Refused(RenderError),
}

impl std::fmt::Display for DmabufImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("the backend imports no dmabuf"),
            Self::NotOffered => formatter.write_str("the buffer's format and modifier were not offered"),
            Self::BadPlanes => formatter.write_str("the buffer's plane count does not fit its modifier"),
            Self::Refused(error) => write!(formatter, "the driver refused the buffer: {error}"),
        }
    }
}

impl std::error::Error for DmabufImportError {}

/// Something that answers the dmabuf questions for a backend.
///
/// The GLES renderers answer them for real. The trait is public so a session without a GPU can be
/// handed another device's answers, which is how the protocol is tested where no device exists:
/// the protocol layer is the same code, and only the importer and the copy are a stand-in.
///
/// The capture methods default to "cannot", so a provider that only imports (the first half of this
/// trait) serves clients' buffers and offers no capture into one.
pub trait DmabufProvider: Send {
    /// What this provider can import, or `None` when it cannot import at all.
    fn support(&mut self) -> Option<DmabufSupport>;

    /// Imports `dmabuf` the way a draw would, so a later draw finds the import done.
    ///
    /// # Errors
    ///
    /// [`DmabufImportError::Refused`] when the driver refuses the buffer.
    fn import(&mut self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError>;

    /// What a capture may be written into, or `None` when this provider cannot write into a client's buffer. The
    /// formats are the ones it can render into, and the device is the one they live on.
    fn capture_support(&mut self) -> Option<DmabufSupport> {
        None
    }

    /// Writes `picture` into `dmabuf` as `copy` says, the way a GPU copy would. The frame an output last drew is
    /// handed over as a picture here, because a stand-in has no frame of its own.
    ///
    /// # Errors
    ///
    /// [`DmabufCopyError`] naming why nothing was written.
    fn write(&mut self, _dmabuf: &Dmabuf, _picture: &Image, _copy: DmabufCopy) -> Result<(), DmabufCopyError> {
        Err(DmabufCopyError::Unsupported)
    }

    /// Reads `dmabuf` back as a picture, top row first.
    ///
    /// # Errors
    ///
    /// [`DmabufCopyError`] naming why nothing was read.
    fn read(&mut self, _dmabuf: &Dmabuf) -> Result<Image, DmabufCopyError> {
        Err(DmabufCopyError::Unsupported)
    }
}

/// Where the pixels of a capture into a client's buffer come from.
#[derive(Clone, Copy, Debug)]
pub enum DmabufSource<'a> {
    /// The frame an output last drew.
    Frame {
        /// The output.
        output: OutputId,
        /// Whether the cursor is in the copy. Without it, what lay under the cursor is.
        with_cursor: bool,
    },
    /// A picture the compositor holds on the CPU, such as a View's.
    Picture(&'a Image),
}

/// Which pixels of the source go where in the client's buffer.
///
/// Buffer pixel `(x, y)` of `region` is read from source pixel `(from.x + x, from.y + y)`. A region that does not fit
/// the buffer or the source writes the part that does, which is the meaning `gnoblin_render_frame::shm::copy_into`
/// gives the same two numbers for shared memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmabufCopy {
    /// The pixels written, in the buffer's.
    pub region: Rect,
    /// Where the region starts in the source.
    pub from: Point,
}

/// Why nothing was copied into, or read from, a client's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmabufCopyError {
    /// The backend cannot copy into, or read, a client's GPU buffer at all.
    Unsupported,
    /// The buffer's format and modifier are not among those a capture offered.
    NotOffered,
    /// The buffer's plane count does not fit its modifier.
    BadPlanes,
    /// There is no frame to copy: the output has not drawn one, or is gone.
    NoFrame,
    /// The driver refused the copy.
    Refused(RenderError),
}

impl From<DmabufImportError> for DmabufCopyError {
    fn from(error: DmabufImportError) -> Self {
        match error {
            DmabufImportError::Unsupported => Self::Unsupported,
            DmabufImportError::NotOffered => Self::NotOffered,
            DmabufImportError::BadPlanes => Self::BadPlanes,
            DmabufImportError::Refused(error) => Self::from(error),
        }
    }
}

impl From<RenderError> for DmabufCopyError {
    fn from(error: RenderError) -> Self {
        match error {
            RenderError::Unsupported => Self::Unsupported,
            other => Self::Refused(other),
        }
    }
}

impl std::fmt::Display for DmabufCopyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("the backend copies into no client GPU buffer"),
            Self::NotOffered => formatter.write_str("the buffer's format and modifier were not offered for capture"),
            Self::BadPlanes => formatter.write_str("the buffer's plane count does not fit its modifier"),
            Self::NoFrame => formatter.write_str("there is no frame to copy"),
            Self::Refused(error) => write!(formatter, "the driver refused the copy: {error}"),
        }
    }
}

impl std::error::Error for DmabufCopyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::allocator::Fourcc;

    fn support() -> DmabufSupport {
        DmabufSupport::new(
            Some(0xe280),
            [
                Format {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::Linear,
                },
                Format {
                    code: Fourcc::Xrgb8888,
                    modifier: Modifier::Invalid,
                },
            ],
        )
    }

    #[test]
    fn a_pair_that_was_not_offered_is_not_offered() {
        let support = support();
        assert!(support.offers(Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear
        }));
        // The same code with another modifier is a different promise.
        assert!(!support.offers(Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Invalid
        }));
        assert!(!support.offers(Format {
            code: Fourcc::Abgr8888,
            modifier: Modifier::Linear
        }));
    }

    #[test]
    fn the_device_is_kept_as_given() {
        assert_eq!(support().main_device, Some(0xe280));
        assert_eq!(DmabufSupport::new(None, []).main_device, None);
    }
}

//! The kernel's answer on a client's dmabuf planes, asked before any renderer maps one.
//!
//! A client names each plane of its buffer with a file descriptor. Smithay's protocol check reads the size with `lseek`,
//! which a pipe or socket refuses (the check then passes) and which a regular file or memfd answers truthfully only until
//! the client resizes it. A renderer that maps such a descriptor, or lets a driver map it, dies of SIGSEGV or SIGBUS on
//! the first read past the end: Mesa's llvmpipe on a display with no render node, and the Vulkan renderer's CPU copy of a
//! linear plane, both read the client's memory in the compositor's own process.
//!
//! [`unmappable`] is the one question every renderer asks first: is each plane a dma-buf, which the kernel sizes once for
//! the buffer's life, and does it hold the rows the buffer says it has.
//!
//! # Failure modes
//!
//! 1. **A plane that is no dma-buf is mapped.** The descriptor's filesystem is the pseudo filesystem of dma-bufs and no
//!    other: `tests/dmabuf_attack.rs` in `gnoblin-wayland` shares a regular file, one a thread keeps resizing and a pipe.
//! 2. **A dma-buf too small for its rows is read.** The plane's size is compared with `offset + stride * rows`.

use std::os::fd::BorrowedFd;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Modifier};

/// `DMA_BUF_MAGIC` of `linux/magic.h`: the `f_type` of the pseudo filesystem every dma-buf lives on, which no other
/// file does.
const DMA_BUF_MAGIC: i128 = 0x444d_4142;

/// Why a plane of a client's buffer is not memory the driver may map.
#[derive(Debug, PartialEq, Eq)]
pub enum PlaneFault {
    /// The kernel cannot say what the descriptor is.
    Unreadable,
    /// The descriptor is not a dma-buf: a regular file, a pipe, `/dev/null`.
    NotDmabuf,
    /// The dma-buf is smaller than the rows the buffer claims.
    TooSmall {
        /// The bytes the plane needs.
        need: u64,
        /// The bytes the dma-buf has.
        have: u64,
    },
}

impl std::fmt::Display for PlaneFault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("the kernel cannot describe the descriptor"),
            Self::NotDmabuf => formatter.write_str("the descriptor is not a dma-buf"),
            Self::TooSmall { need, have } => {
                write!(formatter, "the dma-buf holds {have} bytes and the plane needs {need}")
            }
        }
    }
}

/// The first reason a plane of `dmabuf` cannot be mapped, or `None` when every plane is a dma-buf at least as large as
/// the rows the buffer says it has.
///
/// A plane needs one row (`offset + stride`). The first plane of a linear or implicit-modifier buffer needs all
/// `height` of them: a vendor modifier lays its planes out in tiles and compression metadata that only the driver
/// can size, and a driver that is given a real dma-buf judges those itself. A dma-buf cannot be resized, so the answer
/// holds for the buffer's life; a regular file or memfd could be truncated under the driver, which is why none is let
/// through.
pub fn unmappable(dmabuf: &Dmabuf) -> Option<(usize, PlaneFault)> {
    let height = u64::try_from(dmabuf.size().h).unwrap_or(0).max(1);
    let whole = matches!(dmabuf.format().modifier, Modifier::Linear | Modifier::Invalid);
    let planes = dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides());
    for (index, ((fd, offset), stride)) in planes.enumerate() {
        let rows = if index == 0 && whole { height } else { 1 };
        let need = u64::from(offset).saturating_add(u64::from(stride).saturating_mul(rows));
        if let Err(fault) = plane_holds(fd, need) {
            return Some((index, fault));
        }
    }
    None
}

/// Whether `fd` is a dma-buf of at least `need` bytes.
fn plane_holds(fd: BorrowedFd<'_>, need: u64) -> Result<(), PlaneFault> {
    let kind = rustix::fs::fstatfs(fd).map_err(|_| PlaneFault::Unreadable)?.f_type;
    if i128::from(kind) != DMA_BUF_MAGIC {
        return Err(PlaneFault::NotDmabuf);
    }
    let have = rustix::fs::fstat(fd).map_err(|_| PlaneFault::Unreadable)?.st_size;
    let have = u64::try_from(have).unwrap_or(0);
    if have < need {
        return Err(PlaneFault::TooSmall { need, have });
    }
    Ok(())
}

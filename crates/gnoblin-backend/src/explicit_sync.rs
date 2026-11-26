//! What a backend tells the compositor about explicit synchronisation (`wp_linux_drm_syncobj_manager_v1`).
//!
//! With implicit sync the kernel orders a client's GPU writes before the compositor's reads through fences on the dmabuf,
//! and nothing says when the compositor is done reading. Explicit sync replaces both with timeline points of a DRM
//! synchronisation object: the client names a point that signals when its buffer is ready (acquire) and a point the
//! compositor signals when it has finished reading it (release). Two things in that promise belong to the backend,
//! because only it owns the renderer and the GPU:
//!
//! 1. **Whether it can be kept.** A release point must not signal while the GPU may still read the buffer. A renderer that
//!    cannot say when its reads are over must not offer the protocol, because a client that reuses a buffer on a signalled
//!    release writes into a picture still being drawn. [`Backend::explicit_sync`](crate::Backend::explicit_sync) answers
//!    with the DRM device timelines are imported on, or `None` for a backend whose renderer does not report retirement.
//! 2. **When the reads are over.** [`Backend::buffers_retired`](crate::Backend::buffers_retired) reports how many frames
//!    the renderer has submitted and how many the GPU has finished, which is what the compositor holds a client's release
//!    point against.
//!
//! The acquire side needs nothing of the backend: the compositor holds a commit back until its point signals (the
//! kernel wakes an eventfd), so the renderer is never handed a buffer that is not ready.
//!
//! # Failure modes
//!
//! 1. **The protocol is offered where it cannot work.** A device with no DRM node (a software Vulkan driver) or whose
//!    kernel lacks `DRM_IOCTL_SYNCOBJ_EVENTFD` makes [`ExplicitSync::on_node`] refuse, so no global is created and a
//!    client keeps implicit sync: `a_session_on_a_software_vulkan_device_does_not_serve_syncobj` in
//!    `gnoblin-wayland/tests/syncobj.rs`.
//! 2. **A release point signals under a draw.** Only a renderer that reports [`Retirement`] is offered: Vulkan does, GLES
//!    and the software renderer do not and are never asked.
//! 3. **A rebuilt device leaves releases waiting for ever.** A device made anew counts from zero; the compositor reads a
//!    count that went backwards as every earlier frame being over.

use std::io;
use std::path::PathBuf;

use smithay::backend::drm::{DrmDeviceFd, DrmNode, NodeType};
use smithay::utils::DeviceFd;
use smithay::wayland::drm_syncobj::supports_syncobj_eventfd;

/// A DRM device the compositor imports a client's timelines on, and waits and signals them through.
#[derive(Clone, Debug)]
pub struct ExplicitSync {
    device: DrmDeviceFd,
}

/// Why a device cannot serve explicit sync.
#[derive(Debug)]
pub enum ExplicitSyncRefusal {
    /// The renderer names no DRM node: a software driver, or a context with no device.
    NoNode,
    /// The number names no DRM node on this machine.
    UnknownNode(u64, String),
    /// The node has no render node to open, or it could not be opened.
    Open(PathBuf, io::Error),
    /// The kernel does not know `DRM_IOCTL_SYNCOBJ_EVENTFD`, so a commit cannot wait for an acquire point without
    /// stalling the loop.
    NoEventfd(PathBuf),
}

impl std::fmt::Display for ExplicitSyncRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoNode => formatter.write_str("the renderer's device has no DRM node"),
            Self::UnknownNode(node, why) => write!(formatter, "no DRM node numbered {node:#x}: {why}"),
            Self::Open(path, error) => write!(formatter, "{} cannot be opened: {error}", path.display()),
            Self::NoEventfd(path) => write!(
                formatter,
                "{} does not support syncobj eventfds (Linux 6.6 and later)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ExplicitSyncRefusal {}

impl ExplicitSync {
    /// The render node numbered `node` (its `dev_t`), when the kernel can wait on a timeline point through an eventfd.
    ///
    /// # Errors
    ///
    /// [`ExplicitSyncRefusal`] naming why this device cannot serve the protocol.
    pub fn on_node(node: Option<u64>) -> Result<Self, ExplicitSyncRefusal> {
        let number = node.ok_or(ExplicitSyncRefusal::NoNode)?;
        let found = DrmNode::from_dev_id(number)
            .map_err(|error| ExplicitSyncRefusal::UnknownNode(number, error.to_string()))?;
        let path = found
            .dev_path_with_type(NodeType::Render)
            .or_else(|| found.dev_path())
            .ok_or_else(|| ExplicitSyncRefusal::UnknownNode(number, "it has no device path".to_owned()))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| ExplicitSyncRefusal::Open(path.clone(), error))?;
        let device = DrmDeviceFd::new(DeviceFd::from(std::os::fd::OwnedFd::from(file)));
        if !supports_syncobj_eventfd(&device) {
            return Err(ExplicitSyncRefusal::NoEventfd(path));
        }
        Ok(Self { device })
    }

    /// `device`, taken as capable without asking the kernel.
    ///
    /// For a backend that has already made sure, and for a stand-in that has no GPU: the protocol's error paths that
    /// need no working timeline (a second object for a surface, a timeline that cannot be imported, a buffer with no
    /// points) are exercised against a device that cannot import one.
    #[must_use]
    pub const fn unchecked(device: DrmDeviceFd) -> Self {
        Self { device }
    }

    /// The device, for the compositor to import timelines on.
    #[must_use]
    pub fn into_device(self) -> DrmDeviceFd {
        self.device
    }
}

/// How far the GPU has got through the frames a backend submitted.
///
/// Both counts only grow while the device lives, and a frame is counted when it is submitted, whatever it drew. A client's
/// buffer that any frame up to `submitted` may have read is safe to release once `retired` reaches that count. Everything
/// zero, the default, is a backend with nothing outstanding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retirement {
    /// The number of the last submission.
    pub submitted: u64,
    /// The number of the last submission the GPU has finished, and every one before it.
    pub retired: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_with_no_node_is_refused() {
        assert!(matches!(ExplicitSync::on_node(None), Err(ExplicitSyncRefusal::NoNode)));
    }

    #[test]
    fn a_number_that_is_no_drm_node_is_refused() {
        // Major 1 is the memory devices (`/dev/null`), never DRM (major 226).
        assert!(matches!(
            ExplicitSync::on_node(Some(1 << 8)),
            Err(ExplicitSyncRefusal::UnknownNode(..))
        ));
    }
}

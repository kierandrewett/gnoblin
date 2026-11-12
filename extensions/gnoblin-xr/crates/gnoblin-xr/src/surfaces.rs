//! One swapchain per View: made when the View first has a picture, filled when the picture changes, remade when its
//! size changes, and destroyed when the View goes.
//!
//! The pictures come from the captures (`xr-capture`), the swapchains from the runtime, and the copy from the
//! [`Presenter`]. Each frame the bridge calls [`Surfaces::sync`] with the Views the plan draws, and then asks
//! [`Surfaces::swapchain`] for the ready swapchain of each, which `xr-layers` turns into layers.
//!
//! A swapchain is only handed out once a picture has been copied into it. A picture is copied again only when its
//! serial differs from the one last copied, so a still desktop costs no copies.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use xr_capture::ShmFormat;
use xr_capture::views::FrameSource;
use xr_capture::wayland::Frame;
use xr_gpu::{GpuError, Pixels, Presenter, SourceFormat, SwapchainFormat, Target, choose_format};
use xr_layers::{SwapchainId, SwapchainRef};
use xr_session::runtime::{Capabilities, Runtime, RuntimeError};

use crate::dump::Dump;

/// Why a View could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {
    /// The session has no graphics binding, so there are no swapchains. This is the same for every View.
    NoGraphics,
    /// The runtime lists none of the swapchain formats the extension can fill.
    NoFormat,
    /// The runtime refused a swapchain call.
    Runtime(RuntimeError),
    /// The GPU refused the copy.
    Gpu(GpuError),
}

/// A failure to show a View, or `None` for one that concerns every View.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub view: Option<u64>,
    pub kind: FaultKind,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(view) = self.view {
            write!(f, "view {view}: ")?;
        }
        match &self.kind {
            FaultKind::NoGraphics => f.write_str(
                "the OpenXR session has no graphics binding, so no View can be shown: the runtime must offer \
                 XR_KHR_vulkan_enable2",
            ),
            FaultKind::NoFormat => f.write_str(
                "the runtime offers neither R8G8B8A8_SRGB nor B8G8R8A8_SRGB swapchains, so no View can be shown",
            ),
            FaultKind::Runtime(error) => write!(f, "{error}"),
            FaultKind::Gpu(error) => write!(f, "{error}"),
        }
    }
}

/// The swapchain of one View.
struct Surface {
    id: SwapchainId,
    width: u32,
    height: u32,
    /// The raw image handles, by image index.
    images: Vec<u64>,
    /// The serial of the picture last copied in. `None` until the first copy succeeds.
    shown: Option<u64>,
    /// Set when the capture went away, so the next picture is copied whatever its serial: a new capture counts
    /// its serials from 1 again.
    recaptured: bool,
}

/// The swapchains of the Views the plan draws.
#[derive(Default)]
pub struct Surfaces {
    surfaces: BTreeMap<u64, Surface>,
    /// Where each uploaded picture is also written, for tests (`GNOBLIN_XR_DUMP_DIR`).
    dump: Option<Dump>,
}

fn source_format(format: ShmFormat) -> SourceFormat {
    match format {
        ShmFormat::Argb8888 => SourceFormat::Argb8888,
        ShmFormat::Xrgb8888 => SourceFormat::Xrgb8888,
    }
}

impl Surfaces {
    pub fn new(dump: Option<Dump>) -> Self {
        Self {
            surfaces: BTreeMap::new(),
            dump,
        }
    }

    /// The ready swapchain of `view`: one with a picture in it.
    pub fn swapchain(&self, view: u64) -> Option<SwapchainRef> {
        let surface = self.surfaces.get(&view)?;
        surface.shown.map(|_| SwapchainRef {
            id: surface.id,
            width: surface.width,
            height: surface.height,
        })
    }

    /// Brings the swapchains to the Views in `wanted`: destroys the swapchains of Views no longer wanted, makes or
    /// remakes the swapchain of each wanted View that has a picture, and copies each picture that changed. Returns
    /// what failed. A failure leaves the View without a ready swapchain for this frame and is tried again next time: a
    /// swapchain whose runtime calls fail is destroyed and made again.
    pub fn sync(
        &mut self,
        runtime: &mut impl Runtime,
        presenter: &mut impl Presenter,
        capabilities: &Capabilities,
        wanted: &BTreeSet<u64>,
        frames: &impl FrameSource,
    ) -> Vec<Fault> {
        let mut faults = Vec::new();
        if !capabilities.swapchains {
            // A session with no graphics binding shows nothing, whatever the Views hold.
            if !wanted.is_empty() {
                faults.push(Fault {
                    view: None,
                    kind: FaultKind::NoGraphics,
                });
            }
            return faults;
        }

        let gone: Vec<u64> = self
            .surfaces
            .keys()
            .filter(|view| !wanted.contains(view))
            .copied()
            .collect();
        for view in gone {
            self.destroy(runtime, view, &mut faults);
        }

        let Some(format) = choose_format(&capabilities.swapchain_formats) else {
            if !wanted.is_empty() {
                faults.push(Fault {
                    view: None,
                    kind: FaultKind::NoFormat,
                });
            }
            return faults;
        };

        for &view in wanted {
            let Some(frame) = frames.latest(view) else {
                // The capture has gone or not begun. The swapchain keeps the last picture.
                if let Some(surface) = self.surfaces.get_mut(&view) {
                    surface.recaptured = true;
                }
                continue;
            };
            if frame.width == 0 || frame.height == 0 {
                continue;
            }
            if self
                .surfaces
                .get(&view)
                .is_some_and(|surface| (surface.width, surface.height) != (frame.width, frame.height))
            {
                self.destroy(runtime, view, &mut faults);
            }
            let surface = match self.surfaces.entry(view) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => match create(runtime, frame, format) {
                    Ok(surface) => entry.insert(surface),
                    Err(error) => {
                        faults.push(Fault {
                            view: Some(view),
                            kind: FaultKind::Runtime(error),
                        });
                        continue;
                    }
                },
            };
            if surface.shown == Some(frame.serial) && !surface.recaptured {
                continue;
            }
            match upload(runtime, presenter, surface, frame, format) {
                Ok(()) => {
                    surface.shown = Some(frame.serial);
                    surface.recaptured = false;
                    if let Some(dump) = &mut self.dump {
                        dump.record(view, frame);
                    }
                }
                Err(FaultKind::Runtime(error)) => {
                    // The runtime refused the swapchain's own calls, as it does for one it has lost. The swapchain
                    // is not worth keeping: it is destroyed here, and the next pass makes a new one and copies the
                    // picture again. A copy the GPU refused leaves the swapchain alone.
                    self.destroy(runtime, view, &mut faults);
                    faults.push(Fault {
                        view: Some(view),
                        kind: FaultKind::Runtime(error),
                    });
                }
                Err(kind) => faults.push(Fault { view: Some(view), kind }),
            }
        }
        faults
    }

    /// Destroys every swapchain, for the end of the session.
    pub fn clear(&mut self, runtime: &mut impl Runtime) -> Vec<Fault> {
        let mut faults = Vec::new();
        let views: Vec<u64> = self.surfaces.keys().copied().collect();
        for view in views {
            self.destroy(runtime, view, &mut faults);
        }
        faults
    }

    fn destroy(&mut self, runtime: &mut impl Runtime, view: u64, faults: &mut Vec<Fault>) {
        if let Some(surface) = self.surfaces.remove(&view)
            && let Err(error) = runtime.destroy_swapchain(surface.id)
        {
            faults.push(Fault {
                view: Some(view),
                kind: FaultKind::Runtime(error),
            });
        }
    }
}

#[cfg(test)]
impl Surfaces {
    fn len(&self) -> usize {
        self.surfaces.len()
    }

    fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

/// Makes a swapchain the size of `frame`.
fn create(runtime: &mut impl Runtime, frame: &Frame, format: SwapchainFormat) -> Result<Surface, RuntimeError> {
    let id = runtime.create_swapchain(frame.width, frame.height, format.vk_format())?;
    match runtime.swapchain_images(id) {
        Ok(images) => Ok(Surface {
            id,
            width: frame.width,
            height: frame.height,
            images,
            shown: None,
            recaptured: false,
        }),
        Err(error) => {
            // The swapchain is useless without its images. A failure to destroy it adds nothing to report.
            let _ = runtime.destroy_swapchain(id);
            Err(error)
        }
    }
}

/// Acquires an image, copies `frame` into it and releases it. The image is released whether or not the copy worked.
fn upload(
    runtime: &mut impl Runtime,
    presenter: &mut impl Presenter,
    surface: &Surface,
    frame: &Frame,
    format: SwapchainFormat,
) -> Result<(), FaultKind> {
    let index = runtime.acquire_image(surface.id).map_err(FaultKind::Runtime)?;
    let copied = match surface.images.get(index as usize) {
        Some(&image) => presenter
            .upload(
                Target {
                    image,
                    width: surface.width,
                    height: surface.height,
                    format,
                },
                &Pixels {
                    width: frame.width,
                    height: frame.height,
                    stride: frame.stride,
                    format: source_format(frame.format),
                    bytes: &frame.bytes,
                },
            )
            .map_err(FaultKind::Gpu),
        None => Err(FaultKind::Runtime(RuntimeError(format!(
            "the runtime acquired image {index} of a swapchain that has {}",
            surface.images.len()
        )))),
    };
    let released = runtime.release_image(surface.id).map_err(FaultKind::Runtime);
    copied.and(released)
}

#[cfg(test)]
pub(crate) mod tests {
    use xr_gpu::FakePresenter;
    use xr_session::testing::{Call, FakeRuntime, image_handle};

    use super::*;

    /// Frames keyed by View, set by the test.
    #[derive(Default)]
    pub struct FakeFrames {
        pub frames: BTreeMap<u64, Frame>,
    }

    impl FrameSource for FakeFrames {
        fn pump(&mut self) -> Vec<xr_capture::views::Fault> {
            Vec::new()
        }
        fn sync(&mut self, _wanted: &BTreeSet<u64>) -> Vec<xr_capture::views::Fault> {
            Vec::new()
        }
        fn latest(&self, node: u64) -> Option<&Frame> {
            self.frames.get(&node)
        }
    }

    pub fn frame(width: u32, height: u32, serial: u64, fill: u8) -> Frame {
        Frame {
            width,
            height,
            stride: width * 4,
            format: ShmFormat::Argb8888,
            bytes: vec![fill; (width * height * 4) as usize],
            serial,
        }
    }

    fn wanted(views: &[u64]) -> BTreeSet<u64> {
        views.iter().copied().collect()
    }

    struct Rig {
        runtime: FakeRuntime,
        presenter: FakePresenter,
        frames: FakeFrames,
        surfaces: Surfaces,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                runtime: FakeRuntime::with_graphics([], 16),
                presenter: FakePresenter::default(),
                frames: FakeFrames::default(),
                surfaces: Surfaces::new(None),
            }
        }

        fn sync(&mut self, views: &[u64]) -> Vec<Fault> {
            let caps = xr_session::runtime::Runtime::capabilities(&self.runtime);
            self.surfaces.sync(
                &mut self.runtime,
                &mut self.presenter,
                &caps,
                &wanted(views),
                &self.frames,
            )
        }

        fn creates(&self) -> usize {
            self.runtime
                .calls
                .iter()
                .filter(|call| matches!(call, Call::CreateSwapchain { .. }))
                .count()
        }
    }

    #[test]
    fn a_view_with_a_picture_gets_one_swapchain_and_one_upload() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        assert!(rig.sync(&[2]).is_empty());

        let chain = rig.surfaces.swapchain(2).expect("the swapchain is ready");
        assert_eq!((chain.width, chain.height), (8, 4));
        assert_eq!(rig.creates(), 1);
        assert_eq!(rig.presenter.uploads.len(), 1);
        let upload = &rig.presenter.uploads[0];
        assert_eq!(
            upload.target.image,
            image_handle(chain.id, 0),
            "the first acquired image is written"
        );
        assert_eq!((upload.target.width, upload.target.height), (8, 4));
        assert_eq!(upload.bytes.len(), 8 * 4 * 4);
        // The runtime lists R8G8B8A8_SRGB first, so the picture is converted to it.
        assert_eq!(upload.target.format, SwapchainFormat::Rgba8Srgb);
        assert_eq!(rig.runtime.count(Call::AcquireImage(chain.id)), 1);
        assert_eq!(rig.runtime.count(Call::ReleaseImage(chain.id)), 1);
    }

    #[test]
    fn a_view_without_a_picture_has_no_swapchain() {
        let mut rig = Rig::new();
        assert!(rig.sync(&[2]).is_empty());
        assert_eq!(rig.surfaces.swapchain(2), None);
        assert_eq!(rig.creates(), 0);
    }

    #[test]
    fn the_swapchain_is_made_once_and_the_picture_copied_only_when_the_serial_changes() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.sync(&[2]);
        rig.sync(&[2]);
        rig.sync(&[2]);
        assert_eq!(rig.creates(), 1, "one swapchain for the View");
        assert_eq!(
            rig.presenter.uploads.len(),
            1,
            "an unchanged picture is not copied again"
        );

        rig.frames.frames.insert(2, frame(8, 4, 2, 9));
        rig.sync(&[2]);
        assert_eq!(rig.creates(), 1);
        assert_eq!(rig.presenter.uploads.len(), 2, "a new serial is copied");
        assert_eq!(rig.presenter.uploads[1].bytes[0], 9);
        // The runtime hands the images out in turn.
        let chain = rig.surfaces.swapchain(2).expect("ready");
        assert_eq!(rig.presenter.uploads[1].target.image, image_handle(chain.id, 1));
    }

    #[test]
    fn a_new_size_remakes_the_swapchain() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.sync(&[2]);
        let first = rig.surfaces.swapchain(2).expect("ready").id;

        rig.frames.frames.insert(2, frame(16, 8, 2, 7));
        assert!(rig.sync(&[2]).is_empty());
        let second = rig.surfaces.swapchain(2).expect("ready");
        assert_ne!(second.id, first, "a new swapchain");
        assert_eq!((second.width, second.height), (16, 8));
        assert_eq!(rig.creates(), 2);
        assert_eq!(
            rig.runtime.count(Call::DestroySwapchain(first)),
            1,
            "the old one is destroyed"
        );
        assert!(!rig.runtime.swapchains.contains_key(&first.0));
        assert_eq!(rig.presenter.uploads.len(), 2);
        assert_eq!(rig.presenter.uploads[1].target.width, 16);
    }

    #[test]
    fn a_swapchain_the_runtime_has_lost_is_destroyed_and_made_again() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.sync(&[2]);
        let lost = rig.surfaces.swapchain(2).expect("ready").id;
        rig.runtime.lost.insert(lost.0);

        // A new picture needs the lost swapchain, which the runtime refuses.
        rig.frames.frames.insert(2, frame(8, 4, 2, 9));
        let faults = rig.sync(&[2]);
        assert!(
            matches!(
                faults.as_slice(),
                [Fault {
                    view: Some(2),
                    kind: FaultKind::Runtime(_)
                }]
            ),
            "the refusal is reported: {faults:?}"
        );
        assert_eq!(
            rig.surfaces.swapchain(2),
            None,
            "the View has no ready swapchain this frame"
        );
        assert_eq!(
            rig.runtime.count(Call::DestroySwapchain(lost)),
            1,
            "the lost swapchain is destroyed"
        );

        // The next pass makes a new swapchain and shows the picture in it.
        assert!(rig.sync(&[2]).is_empty());
        let remade = rig.surfaces.swapchain(2).expect("a new swapchain is ready");
        assert_ne!(remade.id, lost);
        assert_eq!(rig.creates(), 2);
        assert_eq!(rig.presenter.uploads.last().map(|upload| upload.bytes[0]), Some(9));
        assert_eq!(rig.runtime.swapchains.len(), 1, "no swapchain leaks");
    }

    #[test]
    fn a_view_that_is_no_longer_wanted_loses_its_swapchain() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.frames.frames.insert(3, frame(8, 4, 1, 7));
        rig.sync(&[2, 3]);
        let gone = rig.surfaces.swapchain(3).expect("ready").id;

        assert!(rig.sync(&[2]).is_empty());
        assert_eq!(rig.surfaces.swapchain(3), None);
        assert_eq!(rig.runtime.count(Call::DestroySwapchain(gone)), 1);
        assert_eq!(rig.runtime.swapchains.len(), 1, "only View 2's swapchain is left");
        assert_eq!(rig.surfaces.len(), 1);
    }

    #[test]
    fn each_view_has_its_own_swapchain() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 1));
        rig.frames.frames.insert(3, frame(4, 4, 1, 2));
        rig.sync(&[2, 3]);
        let a = rig.surfaces.swapchain(2).expect("ready");
        let b = rig.surfaces.swapchain(3).expect("ready");
        assert_ne!(a.id, b.id);
        assert_eq!(rig.creates(), 2);
    }

    #[test]
    fn a_recreated_capture_is_copied_even_with_the_same_serial() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.sync(&[2]);
        // The capture fails and is dropped, then a new one starts counting from 1 again.
        rig.frames.frames.remove(&2);
        rig.sync(&[2]);
        assert!(
            rig.surfaces.swapchain(2).is_some(),
            "the last picture stays shown meanwhile"
        );
        rig.frames.frames.insert(2, frame(8, 4, 1, 5));
        rig.sync(&[2]);
        assert_eq!(rig.presenter.uploads.len(), 2);
        assert_eq!(rig.presenter.uploads[1].bytes[0], 5);
        assert_eq!(rig.creates(), 1);
    }

    #[test]
    fn a_session_without_graphics_reports_one_clear_fault_and_makes_nothing() {
        let mut rig = Rig::new();
        rig.runtime = FakeRuntime::new([]);
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.frames.frames.insert(3, frame(8, 4, 1, 7));
        let faults = rig.sync(&[2, 3]);
        assert_eq!(
            faults,
            vec![Fault {
                view: None,
                kind: FaultKind::NoGraphics
            }]
        );
        assert!(faults[0].to_string().contains("XR_KHR_vulkan_enable2"));
        assert!(rig.runtime.calls.is_empty(), "no swapchain call was made");
        assert!(rig.presenter.uploads.is_empty());
    }

    #[test]
    fn a_runtime_with_no_usable_format_reports_it() {
        let mut rig = Rig::new();
        rig.runtime.capabilities.swapchain_formats = vec![37];
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        let faults = rig.sync(&[2]);
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].kind, FaultKind::NoFormat);
        assert_eq!(rig.creates(), 0);
    }

    #[test]
    fn a_failed_copy_still_releases_the_image_and_is_tried_again() {
        let mut rig = Rig::new();
        rig.presenter.fail_on_upload = Some(1);
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        let faults = rig.sync(&[2]);
        assert_eq!(faults.len(), 1);
        assert!(matches!(faults[0].kind, FaultKind::Gpu(_)));
        assert_eq!(rig.surfaces.swapchain(2), None, "nothing was shown yet");
        let id = SwapchainId(1);
        assert_eq!(
            rig.runtime.count(Call::ReleaseImage(id)),
            1,
            "the acquired image was released"
        );

        assert!(rig.sync(&[2]).is_empty(), "the next frame copies again");
        assert!(rig.surfaces.swapchain(2).is_some());
        assert_eq!(rig.creates(), 1, "the swapchain is kept");
    }

    #[test]
    fn a_failed_swapchain_is_reported_and_tried_again() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        // Calls: CreateSwapchain is the first call the runtime records.
        rig.runtime.fail_on_call = Some(1);
        let faults = rig.sync(&[2]);
        assert_eq!(faults.len(), 1);
        assert!(matches!(faults[0].kind, FaultKind::Runtime(_)));
        assert!(rig.sync(&[2]).is_empty());
        assert!(rig.surfaces.swapchain(2).is_some());
    }

    #[test]
    fn clear_destroys_every_swapchain() {
        let mut rig = Rig::new();
        rig.frames.frames.insert(2, frame(8, 4, 1, 7));
        rig.frames.frames.insert(3, frame(8, 4, 1, 7));
        rig.sync(&[2, 3]);
        assert!(rig.surfaces.clear(&mut rig.runtime).is_empty());
        assert!(rig.surfaces.is_empty());
        assert!(rig.runtime.swapchains.is_empty());
    }

    #[test]
    fn xrgb_pictures_are_copied_opaque() {
        let mut rig = Rig::new();
        let mut picture = frame(2, 1, 1, 0);
        picture.format = ShmFormat::Xrgb8888;
        picture.bytes = vec![10, 20, 30, 0, 40, 50, 60, 0];
        rig.frames.frames.insert(2, picture);
        rig.sync(&[2]);
        // R8G8B8A8: red and blue swapped, alpha forced to 255.
        assert_eq!(rig.presenter.uploads[0].bytes, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }
}

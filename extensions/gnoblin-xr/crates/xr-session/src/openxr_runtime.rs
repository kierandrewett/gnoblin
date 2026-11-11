//! The OpenXR-backed [`Runtime`].
//!
//! This is the only module in the crate that uses `unsafe`. The loader, `xrCreateSession` and the colour-scale
//! chaining are `unsafe` in the `openxr` crate. Every other call in this file is safe.
//!
//! There are two sessions, both [`OpenxrRuntime`]:
//!
//! - [`OpenxrRuntime::connect`] opens a headless (`XR_MND_headless`) session. It has no graphics binding, no
//!   swapchains and no layers: the frame loop runs with empty layer lists.
//! - [`OpenxrRuntime::connect_vulkan`] opens a session on a Vulkan device made through `XR_KHR_vulkan_enable2`. It
//!   creates swapchains, hands out their images as raw `VkImage` handles, and submits quad and projection layers.

#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use openxr as xr;
use xr_gpu::VulkanDevice;
use xr_input::{ControllerState, pointing_ray};
use xr_layers::{Eye, EyeVisibility, Fov, ImageRect, SubmittedLayer, SwapchainId};
use xr_scene::{Pose, Space};

use crate::openxr_input::Input;
use crate::runtime::{Capabilities, FrameTiming, Runtime, RuntimeError, SessionEvent, SessionState};

/// A graphics binding a session can be created with. It adds what the swapchain calls need on top of
/// [`xr::Graphics`]: whether the binding can create swapchains at all, and the raw handle of an image.
pub trait Binding: xr::Graphics {
    /// Whether sessions on this binding can create swapchains. The headless binding cannot.
    const SWAPCHAINS: bool;
    /// Whether sessions on this binding read controller input. The headless binding has no input source.
    const INPUT: bool;
    /// The raw `VkImage` handle of a swapchain image, if the binding is Vulkan.
    fn image_handle(image: &Self::SwapchainImage) -> Option<u64>;
}

impl Binding for xr::Headless {
    const SWAPCHAINS: bool = false;
    const INPUT: bool = false;
    fn image_handle(_: &Self::SwapchainImage) -> Option<u64> {
        None
    }
}

impl Binding for xr::Vulkan {
    const SWAPCHAINS: bool = true;
    const INPUT: bool = true;
    fn image_handle(image: &Self::SwapchainImage) -> Option<u64> {
        Some(*image)
    }
}

/// A session driven through the OpenXR loader.
///
/// Field order matters for drop: the input actions, swapchains, session and spaces are dropped first, then the Vulkan device they
/// used (if any), then the instance, and the entry (which holds the loaded library) last.
pub struct OpenxrRuntime<G: Binding = xr::Headless> {
    /// The controller actions, attached to the session. `None` on a session with no input source.
    input: Option<Input>,
    swapchains: BTreeMap<u64, xr::Swapchain<G>>,
    next_swapchain: u64,
    session: xr::Session<G>,
    waiter: xr::FrameWaiter,
    stream: xr::FrameStream<G>,
    /// The local reference space. Poses are returned relative to it.
    local: xr::Space,
    /// The view reference space. Its origin is the head.
    head: xr::Space,
    events: xr::EventDataBuffer,
    capabilities: Capabilities,
    /// The Vulkan device the session was created on. It must outlive the session.
    _device: Option<Arc<VulkanDevice>>,
    instance: xr::Instance,
    _entry: xr::Entry,
}

fn load_entry() -> Result<xr::Entry, RuntimeError> {
    // SAFETY: `Entry::load` opens the platform OpenXR loader by name and
    // resolves its symbols. The loader is trusted system code, and the
    // returned `Entry` keeps the library mapped for as long as it lives.
    unsafe { xr::Entry::load(&()) }.map_err(|e| err("loading the loader", e))
}

fn application(name: &str) -> xr::ApplicationInfo<'_> {
    xr::ApplicationInfo {
        application_name: name,
        application_version: 0,
        engine_name: "gnoblin-xr",
        engine_version: 0,
        api_version: xr::Version::new(1, 0, 0),
    }
}

/// Fails with a clear message when the runtime does not offer `XR_KHR_vulkan_enable2`.
fn require_vulkan(available: &xr::ExtensionSet) -> Result<(), RuntimeError> {
    if available.khr_vulkan_enable2 {
        Ok(())
    } else {
        Err(RuntimeError(
            "the runtime does not offer XR_KHR_vulkan_enable2, so no swapchain can be created and no layer shown"
                .to_owned(),
        ))
    }
}

impl OpenxrRuntime<xr::Headless> {
    /// Loads the OpenXR loader, creates a headless instance and a session.
    ///
    /// Fails when the loader is missing, when the runtime lacks
    /// `XR_MND_headless`, or when no head-mounted system is present.
    pub fn connect(application_name: &str) -> Result<Self, RuntimeError> {
        let entry = load_entry()?;
        let available = entry
            .enumerate_extensions()
            .map_err(|e| err("enumerating extensions", e))?;
        if !available.mnd_headless {
            return Err(RuntimeError("the runtime does not offer XR_MND_headless".to_owned()));
        }
        let mut enabled = xr::ExtensionSet::default();
        enabled.mnd_headless = true;

        let instance = entry
            .create_instance(&application(application_name), &enabled, &[], &())
            .map_err(|e| err("creating the instance", e))?;
        let system = instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(|e| err("selecting the head-mounted system", e))?;

        // SAFETY: `Headless` takes no graphics binding and its session create
        // info is empty, so the call passes no pointers that could dangle. The
        // system id comes from this instance, and the instance outlives the
        // session because both are owned by `Self`.
        let (session, waiter, stream) =
            unsafe { instance.create_session::<xr::Headless>(system, &xr::headless::SessionCreateInfo {}) }
                .map_err(|e| err("creating the headless session", e))?;
        Self::assemble(entry, instance, system, session, waiter, stream, None, false)
    }
}

impl OpenxrRuntime<xr::Vulkan> {
    /// Loads the OpenXR loader, creates an instance with `XR_KHR_vulkan_enable2`, a Vulkan device through it, and a
    /// session on that device. Returns the runtime and the device, which a [`xr_gpu::VulkanPresenter`] shares.
    ///
    /// Fails, with the reason, when the loader is missing, when the runtime lacks `XR_KHR_vulkan_enable2`, when no
    /// head-mounted system is present, or when the Vulkan device cannot be made.
    pub fn connect_vulkan(application_name: &str) -> Result<(Self, Arc<VulkanDevice>), RuntimeError> {
        let entry = load_entry()?;
        let available = entry
            .enumerate_extensions()
            .map_err(|e| err("enumerating extensions", e))?;
        require_vulkan(&available)?;
        let mut enabled = xr::ExtensionSet::default();
        enabled.khr_vulkan_enable2 = true;
        // The colour scale carries a layer's opacity. A runtime without it still shows the layers, opaque.
        let color_scale_bias = available.khr_composition_layer_color_scale_bias;
        enabled.khr_composition_layer_color_scale_bias = color_scale_bias;
        // The Quest Touch Plus controller has an interaction profile of its own, which needs this extension to bind.
        enabled.meta_touch_controller_plus = available.meta_touch_controller_plus;

        let instance = entry
            .create_instance(&application(application_name), &enabled, &[], &())
            .map_err(|e| err("creating the instance", e))?;
        let system = instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(|e| err("selecting the head-mounted system", e))?;
        let device = VulkanDevice::create(&instance, system).map_err(|e| RuntimeError(e.to_string()))?;

        // SAFETY: the binding holds the handles of the Vulkan instance, physical device, device and queue that
        // `VulkanDevice::create` made for this OpenXR instance and system. The queue family and index name a queue
        // that exists. The device is stored in `Self` and so outlives the session.
        let (session, waiter, stream) =
            unsafe { instance.create_session::<xr::Vulkan>(system, &device.session_create_info()) }
                .map_err(|e| err("creating the Vulkan session", e))?;
        let runtime = Self::assemble(
            entry,
            instance,
            system,
            session,
            waiter,
            stream,
            Some(Arc::clone(&device)),
            color_scale_bias,
        )?;
        Ok((runtime, device))
    }
}

impl<G: Binding> OpenxrRuntime<G> {
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        entry: xr::Entry,
        instance: xr::Instance,
        system: xr::SystemId,
        session: xr::Session<G>,
        waiter: xr::FrameWaiter,
        stream: xr::FrameStream<G>,
        device: Option<Arc<VulkanDevice>>,
        color_scale_bias: bool,
    ) -> Result<Self, RuntimeError> {
        let local = session
            .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            .map_err(|e| err("creating the local space", e))?;
        let head = session
            .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
            .map_err(|e| err("creating the view space", e))?;
        let properties = instance
            .system_properties(system)
            .map_err(|e| err("reading the system properties", e))?;
        let swapchain_formats = if G::SWAPCHAINS {
            session
                .enumerate_swapchain_formats()
                .map_err(|e| err("enumerating swapchain formats", e))?
                .into_iter()
                .map(G::lower_format)
                .collect()
        } else {
            Vec::new()
        };
        let capabilities = Capabilities {
            swapchains: G::SWAPCHAINS,
            max_layer_count: properties.graphics_properties.max_layer_count,
            color_scale_bias,
            swapchain_formats,
        };
        let input = if G::INPUT {
            Some(Input::create(&instance, &session)?)
        } else {
            None
        };
        Ok(Self {
            input,
            swapchains: BTreeMap::new(),
            next_swapchain: 0,
            session,
            waiter,
            stream,
            local,
            head,
            events: xr::EventDataBuffer::new(),
            capabilities,
            _device: device,
            instance,
            _entry: entry,
        })
    }

    fn swapchain(&mut self, id: SwapchainId) -> crate::runtime::Result<&mut xr::Swapchain<G>> {
        self.swapchains
            .get_mut(&id.0)
            .ok_or_else(|| RuntimeError(format!("swapchain {} does not exist", id.0)))
    }
}

impl<G: Binding> Runtime for OpenxrRuntime<G> {
    fn poll_event(&mut self) -> crate::runtime::Result<Option<SessionEvent>> {
        loop {
            let event = self
                .instance
                .poll_event(&mut self.events)
                .map_err(|e| err("polling events", e))?;
            match event {
                None => return Ok(None),
                Some(xr::Event::SessionStateChanged(changed)) => {
                    let state = map_state(changed.state());
                    return Ok(Some(SessionEvent::StateChanged(state)));
                }
                // Other event kinds are not used by the frame loop yet.
                Some(_) => continue,
            }
        }
    }

    fn begin_session(&mut self) -> crate::runtime::Result<()> {
        self.session
            .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
            .map(|_| ())
            .map_err(|e| err("xrBeginSession", e))
    }

    fn end_session(&mut self) -> crate::runtime::Result<()> {
        self.session.end().map(|_| ()).map_err(|e| err("xrEndSession", e))
    }

    fn wait_frame(&mut self) -> crate::runtime::Result<FrameTiming> {
        let state = self.waiter.wait().map_err(|e| err("xrWaitFrame", e))?;
        Ok(FrameTiming {
            predicted_display_time: state.predicted_display_time.as_nanos(),
            should_render: state.should_render,
        })
    }

    fn begin_frame(&mut self) -> crate::runtime::Result<()> {
        self.stream.begin().map_err(|e| err("xrBeginFrame", e))
    }

    fn end_frame(&mut self, display_time: i64, layers: &[SubmittedLayer]) -> crate::runtime::Result<()> {
        let time = xr::Time::from_nanos(display_time);
        if layers.is_empty() {
            return self
                .stream
                .end(time, xr::EnvironmentBlendMode::OPAQUE, &[])
                .map_err(|e| err("xrEndFrame", e));
        }
        // The colour scale structs and the projection views are chained into the layers by pointer, so they are
        // built first and not touched while the layers borrow them.
        let scales: Vec<Option<Box<xr::sys::CompositionLayerColorScaleBiasKHR>>> =
            layers.iter().map(color_scale).collect();
        let mut projection_views = Vec::with_capacity(layers.len());
        for layer in layers {
            projection_views.push(match layer {
                SubmittedLayer::Projection(projection) => {
                    let chain = self
                        .swapchains
                        .get(&projection.swapchain.0)
                        .ok_or_else(|| unknown_swapchain(projection.swapchain))?;
                    Some(projection.eyes.map(|eye| {
                        xr::CompositionLayerProjectionView::new()
                            .pose(to_posef(&eye.pose))
                            .fov(to_fovf(eye.fov))
                            .sub_image(
                                xr::SwapchainSubImage::new()
                                    .swapchain(chain)
                                    .image_rect(to_rect(eye.rect)),
                            )
                    }))
                }
                SubmittedLayer::Quad(_) => None,
            });
        }

        let mut built: Vec<Built<'_, G>> = Vec::with_capacity(layers.len());
        for ((layer, scale), views) in layers.iter().zip(&scales).zip(&projection_views) {
            let flags = xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA;
            let next = scale
                .as_deref()
                .map_or(std::ptr::null(), |scale| std::ptr::from_ref(scale).cast());
            built.push(match layer {
                SubmittedLayer::Quad(quad) => {
                    let chain = self
                        .swapchains
                        .get(&quad.swapchain.0)
                        .ok_or_else(|| unknown_swapchain(quad.swapchain))?;
                    let space = self.space(quad.space);
                    let layer = xr::CompositionLayerQuad::new()
                        .layer_flags(flags)
                        .space(space)
                        .eye_visibility(to_visibility(quad.eye_visibility))
                        .sub_image(
                            xr::SwapchainSubImage::new()
                                .swapchain(chain)
                                .image_rect(to_rect(quad.rect)),
                        )
                        .pose(to_posef(&quad.pose))
                        .size(xr::Extent2Df {
                            width: quad.size[0] as f32,
                            height: quad.size[1] as f32,
                        });
                    let mut raw = layer.into_raw();
                    raw.next = next;
                    // SAFETY: `raw` came from `into_raw` of a layer built by the safe builder, and only `next` was
                    // changed. It points at a colour scale struct in `scales`, or is null. `scales` is not modified
                    // or dropped until after `xrEndFrame`, and the swapchain and space borrows are unchanged.
                    Built::Quad(unsafe { xr::CompositionLayerQuad::from_raw(raw) })
                }
                SubmittedLayer::Projection(projection) => {
                    let views = views
                        .as_ref()
                        .ok_or_else(|| RuntimeError("projection views missing".to_owned()))?;
                    let space = self.space(projection.space);
                    let layer = xr::CompositionLayerProjection::new()
                        .layer_flags(flags)
                        .space(space)
                        .views(views);
                    let mut raw = layer.into_raw();
                    raw.next = next;
                    // SAFETY: as for the quad: only `next` was changed, and it points into `scales` or is null. The
                    // views slice is borrowed from `projection_views`, which is not modified until after the call.
                    Built::Projection(unsafe { xr::CompositionLayerProjection::from_raw(raw) })
                }
            });
        }
        let bases: Vec<&xr::CompositionLayerBase<'_, G>> = built.iter().map(Built::base).collect();
        self.stream
            .end(time, xr::EnvironmentBlendMode::OPAQUE, &bases)
            .map_err(|e| err("xrEndFrame", e))
    }

    fn locate_head(&mut self, display_time: i64) -> crate::runtime::Result<Option<Pose>> {
        self.head_pose(display_time)
    }

    fn locate_controller(&mut self, display_time: i64) -> crate::runtime::Result<ControllerState> {
        let Some(input) = &self.input else {
            return Ok(ControllerState::default());
        };
        let sample = input.poll(&self.session, &self.local, xr::Time::from_nanos(display_time))?;
        // With no tracked controller the ray is the gaze: it leaves the head, straight ahead.
        let head = if sample.aim.is_none() {
            self.head_pose(display_time)?
        } else {
            None
        };
        Ok(ControllerState {
            ray: pointing_ray(sample.aim.as_ref(), head.as_ref()),
            select: sample.select,
            secondary: sample.secondary,
            scroll: sample.scroll,
        })
    }

    fn locate_views(&mut self, display_time: i64) -> crate::runtime::Result<Option<[Eye; 2]>> {
        let (flags, views) = self
            .session
            .locate_views(
                xr::ViewConfigurationType::PRIMARY_STEREO,
                xr::Time::from_nanos(display_time),
                &self.local,
            )
            .map_err(|e| err("xrLocateViews", e))?;
        let valid = xr::ViewStateFlags::POSITION_VALID | xr::ViewStateFlags::ORIENTATION_VALID;
        let [left, right] = views.as_slice() else {
            return Ok(None);
        };
        if !flags.contains(valid) {
            return Ok(None);
        }
        let eye = |view: &xr::View| Eye {
            pose: from_posef(&view.pose),
            fov: Fov {
                left: view.fov.angle_left,
                right: view.fov.angle_right,
                up: view.fov.angle_up,
                down: view.fov.angle_down,
            },
        };
        Ok(Some([eye(left), eye(right)]))
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn create_swapchain(&mut self, width: u32, height: u32, format: i64) -> crate::runtime::Result<SwapchainId> {
        if !G::SWAPCHAINS {
            return Err(RuntimeError::no_graphics("xrCreateSwapchain"));
        }
        let chain = self
            .session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | xr::SwapchainUsageFlags::SAMPLED
                    | xr::SwapchainUsageFlags::TRANSFER_DST
                    | xr::SwapchainUsageFlags::TRANSFER_SRC,
                format: G::raise_format(format),
                sample_count: 1,
                width,
                height,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .map_err(|e| err("xrCreateSwapchain", e))?;
        self.next_swapchain += 1;
        self.swapchains.insert(self.next_swapchain, chain);
        Ok(SwapchainId(self.next_swapchain))
    }

    fn swapchain_images(&mut self, id: SwapchainId) -> crate::runtime::Result<Vec<u64>> {
        let images = self
            .swapchain(id)?
            .enumerate_images()
            .map_err(|e| err("xrEnumerateSwapchainImages", e))?;
        images
            .iter()
            .map(|image| G::image_handle(image).ok_or_else(|| RuntimeError::no_graphics("xrEnumerateSwapchainImages")))
            .collect()
    }

    fn acquire_image(&mut self, id: SwapchainId) -> crate::runtime::Result<u32> {
        let chain = self.swapchain(id)?;
        let index = chain.acquire_image().map_err(|e| err("xrAcquireSwapchainImage", e))?;
        chain
            .wait_image(xr::Duration::INFINITE)
            .map_err(|e| err("xrWaitSwapchainImage", e))?;
        Ok(index)
    }

    fn release_image(&mut self, id: SwapchainId) -> crate::runtime::Result<()> {
        self.swapchain(id)?
            .release_image()
            .map_err(|e| err("xrReleaseSwapchainImage", e))
    }

    fn destroy_swapchain(&mut self, id: SwapchainId) -> crate::runtime::Result<()> {
        self.swapchains
            .remove(&id.0)
            .map(|_| ())
            .ok_or_else(|| unknown_swapchain(id))
    }
}

impl<G: Binding> OpenxrRuntime<G> {
    /// The head's pose in the local space at `display_time`, if the runtime reports it valid.
    fn head_pose(&self, display_time: i64) -> crate::runtime::Result<Option<Pose>> {
        let location = self
            .head
            .locate(&self.local, xr::Time::from_nanos(display_time))
            .map_err(|e| err("locating the head", e))?;
        let flags = location.location_flags;
        let valid = xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID;
        if !flags.contains(valid) {
            return Ok(None);
        }
        Ok(Some(from_posef(&location.pose)))
    }

    /// The interaction profiles the runtime accepted bindings for. Empty on a session with no input source.
    pub fn bound_profiles(&self) -> &[&'static str] {
        self.input.as_ref().map_or(&[], Input::bound_profiles)
    }

    fn space(&self, space: Space) -> &xr::Space {
        match space {
            Space::World => &self.local,
            Space::View => &self.head,
        }
    }
}

/// A built layer, owned until `xrEndFrame` has taken it.
enum Built<'a, G: xr::Graphics> {
    Quad(xr::CompositionLayerQuad<'a, G>),
    Projection(xr::CompositionLayerProjection<'a, G>),
}

impl<'a, G: xr::Graphics> Built<'a, G> {
    fn base(&self) -> &xr::CompositionLayerBase<'a, G> {
        match self {
            Self::Quad(layer) => layer,
            Self::Projection(layer) => layer,
        }
    }
}

fn color_scale(layer: &SubmittedLayer) -> Option<Box<xr::sys::CompositionLayerColorScaleBiasKHR>> {
    let scale = match layer {
        SubmittedLayer::Quad(quad) => quad.color_scale,
        SubmittedLayer::Projection(projection) => projection.color_scale,
    }?;
    let color = |rgba: [f32; 4]| xr::Color4f {
        r: rgba[0],
        g: rgba[1],
        b: rgba[2],
        a: rgba[3],
    };
    Some(Box::new(xr::sys::CompositionLayerColorScaleBiasKHR {
        ty: xr::sys::CompositionLayerColorScaleBiasKHR::TYPE,
        next: std::ptr::null(),
        color_scale: color(scale),
        color_bias: color([0.0; 4]),
    }))
}

fn unknown_swapchain(id: SwapchainId) -> RuntimeError {
    RuntimeError(format!("swapchain {} does not exist", id.0))
}

#[allow(clippy::cast_possible_truncation)]
fn to_posef(pose: &Pose) -> xr::Posef {
    xr::Posef {
        orientation: xr::Quaternionf {
            x: pose.orientation[0] as f32,
            y: pose.orientation[1] as f32,
            z: pose.orientation[2] as f32,
            w: pose.orientation[3] as f32,
        },
        position: xr::Vector3f {
            x: pose.position[0] as f32,
            y: pose.position[1] as f32,
            z: pose.position[2] as f32,
        },
    }
}

pub(crate) fn from_posef(pose: &xr::Posef) -> Pose {
    let p = pose.position;
    let o = pose.orientation;
    Pose {
        position: [f64::from(p.x), f64::from(p.y), f64::from(p.z)],
        orientation: [f64::from(o.x), f64::from(o.y), f64::from(o.z), f64::from(o.w)],
    }
}

fn to_fovf(fov: Fov) -> xr::Fovf {
    xr::Fovf {
        angle_left: fov.left,
        angle_right: fov.right,
        angle_up: fov.up,
        angle_down: fov.down,
    }
}

fn to_rect(rect: ImageRect) -> xr::Rect2Di {
    xr::Rect2Di {
        offset: xr::Offset2Di { x: rect.x, y: rect.y },
        extent: xr::Extent2Di {
            width: rect.width,
            height: rect.height,
        },
    }
}

fn to_visibility(visibility: EyeVisibility) -> xr::EyeVisibility {
    match visibility {
        EyeVisibility::Both => xr::EyeVisibility::BOTH,
        EyeVisibility::Left => xr::EyeVisibility::LEFT,
        EyeVisibility::Right => xr::EyeVisibility::RIGHT,
    }
}

fn map_state(state: xr::SessionState) -> SessionState {
    if state == xr::SessionState::IDLE {
        SessionState::Idle
    } else if state == xr::SessionState::READY {
        SessionState::Ready
    } else if state == xr::SessionState::SYNCHRONIZED {
        SessionState::Synchronized
    } else if state == xr::SessionState::VISIBLE {
        SessionState::Visible
    } else if state == xr::SessionState::FOCUSED {
        SessionState::Focused
    } else if state == xr::SessionState::STOPPING {
        SessionState::Stopping
    } else if state == xr::SessionState::LOSS_PENDING {
        SessionState::LossPending
    } else if state == xr::SessionState::EXITING {
        SessionState::Exiting
    } else {
        SessionState::Unknown
    }
}

pub(crate) fn err(what: &str, e: impl std::fmt::Debug) -> RuntimeError {
    RuntimeError(format!("{what}: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runtime_without_vulkan_enable2_is_refused_with_a_clear_error() {
        let error = require_vulkan(&xr::ExtensionSet::default()).expect_err("no extension");
        assert!(error.0.contains("XR_KHR_vulkan_enable2"), "{error}");
    }

    #[test]
    fn a_runtime_with_vulkan_enable2_is_accepted() {
        let mut available = xr::ExtensionSet::default();
        available.khr_vulkan_enable2 = true;
        assert_eq!(require_vulkan(&available), Ok(()));
    }

    /// Connects to a real OpenXR runtime (Monado with `XR_MND_headless`),
    /// reaches `Ready`, then runs one frame with no layers.
    ///
    /// Run with `GNOBLIN_XR_MONADO=1 cargo test -- --ignored`. The test is
    /// ignored by default because it needs an installed runtime.
    #[test]
    #[ignore = "needs a Monado runtime; set GNOBLIN_XR_MONADO=1 and run with --ignored"]
    fn monado_headless_runs_one_empty_frame() {
        if std::env::var_os("GNOBLIN_XR_MONADO").as_deref() != Some("1".as_ref()) {
            return;
        }
        let mut runtime = OpenxrRuntime::connect("gnoblin-xr-test").expect("connect");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut ready = false;
        while !ready && std::time::Instant::now() < deadline {
            match runtime.poll_event().expect("poll") {
                Some(SessionEvent::StateChanged(SessionState::Ready)) => ready = true,
                Some(_) => {}
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        assert!(ready, "the session never reached READY");

        runtime.begin_session().expect("begin");
        let timing = runtime.wait_frame().expect("wait");
        runtime.begin_frame().expect("begin frame");
        runtime
            .end_frame(timing.predicted_display_time, &[])
            .expect("end frame");
        runtime.end_session().expect("end");
    }

    /// Runs a Vulkan session against Monado's simulated driver and checks the controller input: the action set
    /// attaches (`connect_vulkan` fails otherwise), at least one interaction profile took the suggested bindings,
    /// `xrSyncActions` succeeds on every frame, the right-hand aim pose is located, and `locate_controller` reports a ray
    /// built from it. Monado's simulated driver makes controllers only when it is told which: `SIMULATED_LEFT=simple
    /// SIMULATED_RIGHT=simple` give `khr/simple_controller` ones (without them it has a head and no hands, and the ray is
    /// the gaze).
    ///
    /// Run with `GNOBLIN_XR_MONADO=1 cargo test -- --ignored`, with `XR_RUNTIME_JSON` pointing at Monado's manifest and a
    /// running `monado-service` started with `SIMULATED_ENABLE=1 SIMULATED_LEFT=simple SIMULATED_RIGHT=simple
    /// XRT_COMPOSITOR_NULL=1`.
    #[test]
    #[ignore = "needs Monado with the simulated driver and a Vulkan driver; set GNOBLIN_XR_MONADO=1 and run with --ignored"]
    fn monado_simulated_controllers_give_an_aim_ray_and_sync_actions() {
        if std::env::var_os("GNOBLIN_XR_MONADO").as_deref() != Some("1".as_ref()) {
            return;
        }
        let (mut runtime, _device) = OpenxrRuntime::connect_vulkan("gnoblin-xr-input-test").expect("connect");
        let bound = runtime.bound_profiles().to_vec();
        eprintln!("profiles that took the bindings: {bound:?}");
        assert!(!bound.is_empty(), "no interaction profile took the suggested bindings");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut ready = false;
        while !ready && std::time::Instant::now() < deadline {
            match runtime.poll_event().expect("poll") {
                Some(SessionEvent::StateChanged(state)) => ready = state == SessionState::Ready,
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        assert!(ready, "the session never reached READY");
        runtime.begin_session().expect("begin session");

        let mut state = SessionState::Ready;
        let mut aimed = 0;
        let mut gazed = 0;
        let mut focused_frames = 0;
        for frame in 0..120 {
            while let Some(SessionEvent::StateChanged(next)) = runtime.poll_event().expect("poll") {
                eprintln!("session state: {next:?}");
                state = next;
            }
            let timing = runtime.wait_frame().expect("wait frame");
            runtime.begin_frame().expect("begin frame");
            let time = timing.predicted_display_time;

            let sample = runtime
                .input
                .as_ref()
                .expect("a Vulkan session reads input")
                .poll(&runtime.session, &runtime.local, xr::Time::from_nanos(time))
                .expect("xrSyncActions and the action reads succeed");
            let controller = runtime.locate_controller(time).expect("locate_controller");
            if frame % 20 == 0 {
                eprintln!("frame {frame} ({state:?}): sample {sample:?}, ray {:?}", controller.ray);
            }
            if state == SessionState::Focused {
                focused_frames += 1;
            }
            if sample.aim.is_some() {
                aimed += 1;
            } else if controller.ray.is_some() {
                gazed += 1;
            }
            if let Some(ray) = controller.ray {
                let length = ray.direction.iter().map(|c| c * c).sum::<f64>().sqrt();
                assert!(
                    (length - 1.0).abs() < 1e-3,
                    "the ray direction is a unit vector: {ray:?}"
                );
            } else {
                panic!("frame {frame}: neither the aim pose nor the head is located: {sample:?}");
            }
            runtime.end_frame(time, &[]).expect("end frame");
        }
        eprintln!("frames with an aim pose: {aimed}, with gaze only: {gazed}, focused: {focused_frames}");
        assert!(
            aimed > 0,
            "the simulated controller's aim pose was never located ({gazed} gaze-only frames)"
        );
        // The simulated controllers have nothing to press, so select, secondary and scroll stay at rest.
    }

    /// Runs a Vulkan session against Monado with a Vulkan driver (lavapipe on Agent37), creates a swapchain, uploads a
    /// picture through the real [`xr_gpu::VulkanPresenter`], reads the image back and compares it with the converted
    /// source, and ends two frames with a quad layer: one opaque, one with a colour scale.
    ///
    /// Run with `GNOBLIN_XR_MONADO=1 cargo test -- --ignored`, with `XR_RUNTIME_JSON` pointing at Monado's manifest and
    /// a running `monado-service`.
    #[test]
    #[ignore = "needs Monado and a Vulkan driver; set GNOBLIN_XR_MONADO=1 and run with --ignored"]
    fn monado_vulkan_session_holds_the_uploaded_pixels_and_accepts_a_quad() {
        use xr_gpu::{Pixels, Presenter, SourceFormat, Target, VulkanPresenter, choose_format, convert, packed_len};
        use xr_layers::{Limits, compose};
        use xr_scene::{IDENTITY, Layer, Placement};

        if std::env::var_os("GNOBLIN_XR_MONADO").as_deref() != Some("1".as_ref()) {
            return;
        }
        let (mut runtime, device) = OpenxrRuntime::connect_vulkan("gnoblin-xr-test").expect("connect");
        let mut presenter = VulkanPresenter::new(device).expect("presenter");
        let capabilities = runtime.capabilities();
        assert!(capabilities.swapchains, "a Vulkan session creates swapchains");
        eprintln!("capabilities: {capabilities:?}");
        let format = choose_format(&capabilities.swapchain_formats).expect("a supported swapchain format");

        // A 64x48 picture: a gradient with an alpha that is not 255, so a wrong channel order shows.
        let (width, height) = (64_u32, 48_u32);
        let mut source = Vec::new();
        for y in 0..height {
            for x in 0..width {
                source.extend_from_slice(&[(x * 4) as u8, (y * 5) as u8, (x + y) as u8, 200]);
            }
        }
        let pixels = Pixels {
            width,
            height,
            stride: width * 4,
            format: SourceFormat::Argb8888,
            bytes: &source,
        };
        let mut expected = vec![0; packed_len(width, height)];
        convert(&pixels, format, &mut expected).expect("converts");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut ready = false;
        while !ready && std::time::Instant::now() < deadline {
            match runtime.poll_event().expect("poll") {
                Some(SessionEvent::StateChanged(state)) => {
                    eprintln!("session state: {state:?}");
                    ready = state == SessionState::Ready;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        assert!(ready, "the session never reached READY");
        runtime.begin_session().expect("begin session");
        for _ in 0..200 {
            if let Some(SessionEvent::StateChanged(state)) = runtime.poll_event().expect("poll") {
                eprintln!("session state after begin: {state:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let mut chain = None;
        for opacity in [1.0, 0.5] {
            let timing = runtime.wait_frame().expect("wait frame");
            runtime.begin_frame().expect("begin frame");
            let id = match chain {
                Some(id) => id,
                None => {
                    let id = runtime
                        .create_swapchain(width, height, format.vk_format())
                        .expect("swapchain");
                    chain = Some(id);
                    id
                }
            };
            let images = runtime.swapchain_images(id).expect("images");
            let index = runtime.acquire_image(id).expect("acquire");
            let target = Target {
                image: images[index as usize],
                width,
                height,
                format,
            };
            presenter.upload(target, &pixels).expect("upload");
            let read = presenter.read_back(target).expect("read back");
            assert_eq!(read, expected, "the swapchain image holds the converted picture");
            runtime.release_image(id).expect("release");

            let placement = Placement {
                id: 2,
                source: 3,
                pose: Pose {
                    position: [0.0, 0.0, -1.0],
                    orientation: IDENTITY,
                },
                size: [1.6, 1.2],
                extent: [64.0, 48.0],
                opacity,
                space: Space::World,
                input: true,
            };
            let limits = Limits {
                max_layer_count: capabilities.max_layer_count,
                reserved: 0,
                color_scale_bias: capabilities.color_scale_bias,
            };
            let submission = compose(
                &[Layer::Quad(placement)],
                &|_| Some(xr_layers::SwapchainRef { id, width, height }),
                None,
                &limits,
            );
            assert_eq!(submission.layers.len(), 1);
            runtime
                .end_frame(timing.predicted_display_time, &submission.layers)
                .expect("xrEndFrame accepts the quad");
        }
        let id = chain.expect("a swapchain was made");
        runtime.destroy_swapchain(id).expect("destroy");
    }
}

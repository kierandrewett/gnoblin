//! The EGL bootstrap: a GLES 3 context on a surfaceless display or on a GBM device.
//!
//! This module is the crate's second FFI shim, and part of the reason the crate opts out
//! of `unsafe_code = "forbid"`: every way to make an EGL display in Smithay is `unsafe`,
//! because Smithay cannot check that the native display a caller names is valid and
//! because it tracks displays process-wide — a second `eglTerminate` on a display this
//! process does not own would invalidate every context on it. The rest of the crate
//! never names this module: [`super::Gles::new`] and [`super::Gles::new_on`] are the only
//! callers, so a session that runs the software path never touches EGL.
//!
//! # Why surfaceless, and why GBM on the hardware path
//!
//! A headless session has no window and no DRM device, and the GLES renderer draws into
//! its own offscreen target — so the context needs no surface, only a display. Mesa's
//! `EGL_MESA_platform_surfaceless` is exactly that: a display not tied to X11, Wayland, a
//! GPU node or a seat, which is why a CI machine with llvmpipe and a developer's machine
//! with a real GPU both work with no configuration.
//!
//! The hardware path is different: its scan-out buffers are allocated by GBM on the DRM
//! node, so the context must be made on that same device or the buffers cannot be drawn
//! into. [`gbm_display`] names that device, and [`context_on`] makes a context on any
//! display the caller holds.
#![allow(unsafe_code)]

use gnoblin_log::{debug_in, info_in};
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::egl::context::{GlAttributes, PixelFormatRequirements};
use smithay::backend::egl::native::EGLSurfacelessDisplay;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::glow::GlowRenderer;

use crate::render::RenderError;

/// A GLES 3 context on a surfaceless EGL display, wrapped in Smithay's Glow renderer.
///
/// # Errors
///
/// [`RenderError::Unavailable`] when no EGL library, no surfaceless platform or no GLES 3
/// context is available. The reason is logged where it happened, which is what a caller
/// reporting "this machine cannot run the GLES path" needs.
pub(super) fn context() -> Result<GlowRenderer, RenderError> {
    // SAFETY: Smithay's `EGLDisplay::new` is unsafe for two reasons, and neither applies
    // here. It cannot check the native display it is given, and `EGLSurfacelessDisplay`
    // names no pointer at all: it asks the driver for its own surfaceless platform
    // display. Its other caveat is a process that mixes this with its own
    // `eglGetPlatformDisplay`/`eglTerminate` on the same display — this process reaches
    // EGL through Smithay alone, so the display's lifetime is Smithay's to track.
    let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }.map_err(|error| {
        debug_in!("backend", ?error, "[render] no surfaceless EGL display");
        RenderError::Unavailable
    })?;
    context_on(&display)
}

/// The EGL display of a GBM device: the display scan-out buffers are allocated for, and the
/// one a context drawing into them must be made on.
///
/// The caller keeps the display for as long as any context made on it lives.
///
/// # Errors
///
/// [`RenderError::Unavailable`] when EGL cannot open the device. The reason is logged.
pub(crate) fn gbm_display(device: &GbmDevice<DrmDeviceFd>) -> Result<EGLDisplay, RenderError> {
    // SAFETY: `GbmDevice` names the DRM node this process holds as master, which the seat
    // lent it and which outlives every context on this display because the backend holds
    // the device and the display together and drops the contexts first. Nothing else in the
    // process terminates this display: it is reached through Smithay alone.
    unsafe { EGLDisplay::new(device.clone()) }.map_err(|error| {
        debug_in!("backend", ?error, "[render] no EGL display on the GBM device");
        RenderError::Unavailable
    })
}

/// A GLES 3 context on `display`, wrapped in Smithay's Glow renderer.
///
/// # Errors
///
/// [`RenderError::Unavailable`] when the display offers no GLES 3 context. The reason is
/// logged.
pub(super) fn context_on(display: &EGLDisplay) -> Result<GlowRenderer, RenderError> {
    let context = match EGLContext::new_with_config(display, gl_attributes(), requirements()) {
        Ok(context) => context,
        Err(error) => {
            // A driver without a renderable RGBA8 config may still offer a configless
            // context, which is enough here: every draw goes into a framebuffer object,
            // so no surface and no config are needed beyond colour depth on the texture.
            debug_in!(
                "backend",
                ?error,
                "[render] no configured GLES 3 context; trying a configless one"
            );
            EGLContext::new(display).map_err(|error| {
                debug_in!("backend", ?error, "[render] no GLES 3 context");
                RenderError::Unavailable
            })?
        }
    };
    info_in!("backend", "[render] the GLES context is ready");
    // SAFETY: `GlowRenderer::new` is unsafe because a context active on another thread
    // would be undefined behaviour. This context was created by this call on this thread,
    // is shared with nothing, and nothing else in the process makes a context current.
    unsafe { GlowRenderer::new(context) }.map_err(|error| {
        debug_in!("backend", ?error, "[render] the GLES renderer could not be created");
        RenderError::Unavailable
    })
}

/// The context attributes: GLES 3.0 and nothing else.
///
/// 3.0 is the floor and the ceiling both: the shaders are compiled as GLSL ES 3.10
/// (`naga`'s lowest output), and asking for 3.0 on a driver that offers 3.2 means nothing
/// here can silently depend on newer behaviour.
fn gl_attributes() -> GlAttributes {
    GlAttributes {
        version: (3, 0),
        profile: None,
        debug: false,
        vsync: false,
    }
}

/// The pixel format: RGBA8, because the target is an RGBA8 texture and a config without
/// alpha cannot render into one.
fn requirements() -> PixelFormatRequirements {
    PixelFormatRequirements {
        // A surfaceless context renders into the renderer's own RGB8 texture, so a config
        // without depth, stencil or multisampling is all it needs; `hardware_accelerated`
        // stays unset because llvmpipe is a perfectly good answer in CI.
        hardware_accelerated: None,
        color_bits: Some(24),
        float_color_buffer: false,
        alpha_bits: Some(8),
        depth_bits: None,
        stencil_bits: None,
        multisampling: Some(0),
    }
}

//! Which renderer draws an output, and the one type that holds it.
//!
//! [`RendererKind`] is the choice and [`AnyRenderer`] is the choice made: a software, GLES
//! or Vulkan renderer behind the four calls an offscreen output needs. This is the only
//! place in the crate that dispatches on the renderer or tests the `vulkan` feature to do
//! so; `render/mod.rs` explains why it is an enum and not a trait.

use gnoblin_config::{CaptureSecureTreatment, RendererChoice};
use gnoblin_types::{Rect, Scale, Size};

use gnoblin_render_frame::{
    error::RenderError,
    frame::{OutputFrame, SecureTreatment},
    image::Image,
};
use gnoblin_render_gles::Gles;
use gnoblin_render_software::Software;
#[cfg(feature = "vulkan")]
use gnoblin_render_vulkan::Vulkan;

#[cfg(not(feature = "vulkan"))]
use crate::log;

/// The renderers this crate can draw an output with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RendererKind {
    /// Smithay's Pixman renderer: pixels with no GPU.
    Software,
    /// Smithay's GLES renderer: effects and user shaders, on a surfaceless EGL context when
    /// it draws offscreen.
    Gles,
    /// The Vulkan renderer on the device the loader offers, which may be a software driver.
    ///
    /// A build without the `vulkan` feature cannot create it: [`AnyRenderer::new`] refuses
    /// with [`RenderError::Unavailable`] and a log line that names the feature.
    Vulkan,
}

impl RendererKind {
    /// What `compositor.renderer = auto` means.
    ///
    /// Software, because it is the reference every other renderer is compared against, it
    /// needs no GPU, no EGL and no session, and so it is the one choice that cannot fail on
    /// a machine where the session starts at all. Vulkan is not eligible until it draws
    /// effects and can scan out (`design/vulkan-backend.md`), and GLES is not chosen for a
    /// session that did not ask for it because a missing EGL library would then stop a
    /// session that software would have run. The rule is written here so a backend that
    /// honours the setting cannot grow its own.
    pub const AUTO: Self = Self::Software;

    /// The renderer's name, for a log line and a startup report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Gles => "gles",
            Self::Vulkan => "vulkan",
        }
    }

    /// The renderer a `compositor.renderer` setting names; `auto` is [`Self::AUTO`].
    #[must_use]
    pub const fn from_setting(setting: RendererChoice) -> Self {
        match setting {
            RendererChoice::Auto => Self::AUTO,
            RendererChoice::Software => Self::Software,
            RendererChoice::Gles => Self::Gles,
            RendererChoice::Vulkan => Self::Vulkan,
        }
    }
}

impl Default for RendererKind {
    fn default() -> Self {
        Self::AUTO
    }
}

/// The treatment a `capture.secure-treatment` setting names.
///
/// This is a free function rather than a `From` impl because both types live in other crates.
#[must_use]
pub const fn secure_treatment_from_setting(setting: CaptureSecureTreatment) -> SecureTreatment {
    match setting {
        CaptureSecureTreatment::Mask => SecureTreatment::Mask,
        CaptureSecureTreatment::Omit => SecureTreatment::Omit,
    }
}

/// A renderer of one of the [`RendererKind`]s, drawing into an offscreen buffer it owns.
///
/// The variants are boxed because a GLES renderer owns a context and its programs and is
/// several times the size of a pixman one; without the box every holder would reserve the
/// larger size, and a renderer is created once per output.
pub enum AnyRenderer {
    /// Pixman, into an offscreen image buffer.
    Software(Box<Software>),
    /// GLES, into a texture on a surfaceless context.
    Gles(Box<Gles>),
    /// Vulkan, into an offscreen image on the device the loader offers.
    #[cfg(feature = "vulkan")]
    Vulkan(Box<Vulkan>),
}

/// Runs one expression against whichever renderer is installed.
///
/// It is where the `vulkan` feature is tested for dispatch: a call that every renderer
/// answers the same way is written once and the macro supplies the three arms.
macro_rules! on_renderer {
    ($renderer:expr, $name:ident => $call:expr) => {
        match $renderer {
            Self::Software($name) => $call,
            Self::Gles($name) => $call,
            #[cfg(feature = "vulkan")]
            Self::Vulkan($name) => $call,
        }
    };
}

impl AnyRenderer {
    /// A renderer of `kind` with an offscreen buffer of `physical` pixels.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] when the size covers no pixels, and
    /// [`RenderError::Unavailable`] when this machine cannot create the renderer at all:
    /// for GLES, no EGL library, no surfaceless platform or no GLES 3 context; for Vulkan,
    /// no loader, no device or a refused shader (the log names which), or a build without
    /// the `vulkan` feature.
    pub fn new(kind: RendererKind, physical: Size) -> Result<Self, RenderError> {
        match kind {
            RendererKind::Software => Software::new(physical).map(|software| Self::Software(Box::new(software))),
            RendererKind::Gles => Gles::new(physical).map(|gles| Self::Gles(Box::new(gles))),
            RendererKind::Vulkan => Self::vulkan(physical),
        }
    }

    #[cfg(feature = "vulkan")]
    fn vulkan(physical: Size) -> Result<Self, RenderError> {
        Vulkan::new(physical)
            .map(|vulkan| Self::Vulkan(Box::new(vulkan)))
            .map_err(gnoblin_render_vulkan::VulkanError::into_render_error)
    }

    #[cfg(not(feature = "vulkan"))]
    fn vulkan(_physical: Size) -> Result<Self, RenderError> {
        log::warn!(
            area = "render",
            "this build has no Vulkan renderer (the `vulkan` feature is off)"
        );
        Err(RenderError::Unavailable)
    }

    /// Which renderer this is.
    #[must_use]
    pub fn kind(&self) -> RendererKind {
        match self {
            Self::Software(_) => RendererKind::Software,
            Self::Gles(_) => RendererKind::Gles,
            #[cfg(feature = "vulkan")]
            Self::Vulkan(_) => RendererKind::Vulkan,
        }
    }

    /// Draws a frame, repainting only its damage.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn draw(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), RenderError> {
        on_renderer!(self, renderer => renderer.draw(frame, scale))
    }

    /// Reads the frame back, with the cursor composed in when `with_cursor` is set.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn capture(&mut self, with_cursor: bool) -> Result<Image, RenderError> {
        on_renderer!(self, renderer => renderer.capture(with_cursor))
    }

    /// Replaces the buffer with one of the given size.
    ///
    /// # Errors
    ///
    /// The installed renderer's [`RenderError`].
    pub fn resize(&mut self, physical: Size) -> Result<(), RenderError> {
        on_renderer!(self, renderer => renderer.resize(physical))
    }

    /// The GLES renderer, when this is one.
    ///
    /// A caller that wants the draw counter or a shader error asks for it here; the other
    /// renderers answer [`None`], because a question about a GL device has no answer without
    /// one.
    #[must_use]
    pub fn gles_mut(&mut self) -> Option<&mut Gles> {
        match self {
            Self::Gles(gles) => Some(gles.as_mut()),
            _ => None,
        }
    }
}

/// Reads a renderer back for a capture, with the secure layers of the frame it last drew replaced by `treatment`.
///
/// `display` is that frame, or `None` when it has no secure layer, in which case the read is
/// the plain one. Otherwise the replaced frame is drawn and read, and the display is drawn
/// again with full damage so the next present shows the real layers. The display is restored
/// even when the read fails.
pub(crate) fn capture_redacted<R>(
    renderer: &mut R,
    display: Option<&OutputFrame>,
    bounds: Rect,
    scale: Scale,
    with_cursor: bool,
    treatment: SecureTreatment,
    draw: fn(&mut R, &OutputFrame, Scale) -> Result<(), RenderError>,
    read: fn(&mut R, bool) -> Result<Image, RenderError>,
) -> Result<Image, RenderError> {
    let Some(display) = display else {
        return read(renderer, with_cursor);
    };
    let replaced = display.redacted(treatment, bounds);
    let image = draw(renderer, &replaced, scale).and_then(|()| read(renderer, with_cursor));
    let restored = draw(renderer, &display.fully_damaged(bounds), scale);
    let image = image?;
    restored?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_renderer_setting_selects_the_renderer_and_auto_keeps_the_default() {
        assert_eq!(
            RendererKind::from_setting(RendererChoice::Auto),
            RendererKind::default(),
            "auto is what a session that says nothing gets"
        );
        assert_eq!(RendererKind::AUTO, RendererKind::Software, "auto stays software");
        assert_eq!(
            RendererKind::from_setting(RendererChoice::Software),
            RendererKind::Software
        );
        assert_eq!(RendererKind::from_setting(RendererChoice::Gles), RendererKind::Gles);
        assert_eq!(RendererKind::from_setting(RendererChoice::Vulkan), RendererKind::Vulkan);
    }

    #[test]
    fn a_software_renderer_reports_its_kind_and_has_no_gl_device() {
        let mut renderer = AnyRenderer::new(RendererKind::Software, Size::new(8, 8)).expect("a software renderer");
        assert_eq!(renderer.kind(), RendererKind::Software);
        assert!(renderer.gles_mut().is_none(), "only a GLES renderer answers gles_mut");
    }

    #[cfg(not(feature = "vulkan"))]
    #[test]
    fn a_build_without_the_vulkan_feature_refuses_the_vulkan_renderer() {
        assert!(matches!(
            AnyRenderer::new(RendererKind::Vulkan, Size::new(8, 8)),
            Err(RenderError::Unavailable)
        ));
    }
}

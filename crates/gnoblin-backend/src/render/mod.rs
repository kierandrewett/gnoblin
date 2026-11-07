//! The render path of the backend: the session scene's composition into a frame, and the
//! choice of renderer that draws it.
//!
//! This is the Renderer card of `design/rust-compositor.md` at the mechanism layer
//! (D12: GLES now, Pixman for software). The parts live in their own crates, so each
//! renderer's external libraries stay out of the others:
//!
//! | Crate                      | Owns                                                                |
//! | -------------------------- | ------------------------------------------------------------------- |
//! | `gnoblin-render-frame`     | the renderer-neutral seam: `OutputFrame`, layers, overlays, `Image`, geometry, shared WGSL shaders, `RenderError`, `wl_shm` writer |
//! | `gnoblin-render-software`  | Pixman: the frame into an offscreen buffer, damage-only             |
//! | `gnoblin-render-gles`      | Smithay's GLES renderer: the same frame with effects and shaders    |
//! | `gnoblin-render-vulkan`    | `ash`: the same frame on a GPU or lavapipe (`vulkan` feature)        |
//!
//! This module keeps what needs the scene and the choice of renderer:
//!
//! | Module        | Owns                                                                |
//! | ------------- | ------------------------------------------------------------------- |
//! | [`compose`]   | the session's `gnoblin_render::Scene` and the frame it produces     |
//! | [`renderer`]  | `RendererKind`, the choice, and `AnyRenderer`, the choice made      |
//!
//! # Why the frame is a value
//!
//! An `OutputFrame` (in `gnoblin-render-frame`) is the seam between the compositor and a
//! renderer, and it names no renderer API: a GLES path reads the same value a Pixman path
//! does. That is what makes D12's "write every effect once, compile per backend" possible
//! later, and it is why the scene-to-frame step ([`compose::Composer`]) can be unit-tested
//! without a device.
//!
//! # Choosing a renderer
//!
//! The three renderers share four calls (`new`, `draw`, `capture`, `resize`) over an
//! `OutputFrame`, and the choice is `RendererKind`, with `RendererKind::AUTO` as the
//! single statement of what `compositor.renderer = auto` means. Each backend picks its
//! renderer in one place:
//!
//! | Backend  | Picks                                                                          |
//! | -------- | ------------------------------------------------------------------------------ |
//! | headless | `HeadlessConfig::renderer`, any kind, built by `AnyRenderer::new`              |
//! | nested   | Pixman, hard-wired in `nested/surface.rs`; the compositor refuses others       |
//! | DRM      | GLES, hard-wired in `drm/scanout.rs` on the device's EGL display               |
//!
//! Nested and DRM do not go through `AnyRenderer` yet: DRM needs to draw into a scan-out
//! buffer the device owns, which the offscreen constructor does not offer (see "Target
//! ownership" in `design/vulkan-backend.md`).
//!
//! ## Why an enum and not a trait
//!
//! `AnyRenderer` is an enum with one `match` per call, written once by a macro. A trait
//! would name the same four calls, but they are all the three renderers have in common:
//! GLES alone has an EGL display and context, a scan-out buffer, user shaders, a shader
//! error and a draw counter; Vulkan owns its device and target; Pixman has neither. Every
//! caller that wants one of those would have to downcast out of a `Box<dyn ...>`, or the
//! trait would grow methods two of the three implement as no-ops. The set is closed and
//! chosen at build time, so an enum costs nothing a trait buys: adding a renderer is a
//! compile error at every `match` instead of a silent default.
//!
//! # Contract with the backend
//!
//! `Backend::render` asks the `Host` to fill a frame for one output and hands it to that
//! output's `Target`. The host takes the scene's damage once per iteration
//! (`Host::begin_frame`) and a target that has nothing to draw draws nothing, which is
//! what makes an idle iteration free.
//!
//! # Colour
//!
//! Every colour crosses this boundary in one form: `gnoblin_types::Color`, linear
//! components from 0 to 1 where the stored values are display sRGB. A renderer clamps at
//! draw time, so an overshooting animation cannot produce an invalid colour, and nothing
//! here applies a transfer function: colour management is Gnoblin's own pipeline, which
//! D12 leaves for later.

/// The scene and the drawing of one output's frame from it. See [`compose::Composer`].
pub mod compose;
/// Which renderer draws an output, and the one type that holds it.
pub mod renderer;
/// The CPU renderer, built on Pixman. Moves to its own crate in phase 4.
pub mod software;

//! Which backend a session drives.
//!
//! The backend is the one place a session touches hardware, so it is the one place
//! [`SessionConfig`](crate::SessionConfig) has to name a choice. It is an enum and not a trait
//! object because the choice is data: [`crate::backend_select`] reads it from the configuration document, the
//! `GNOBLIN_BACKEND` variable and the host, and a test passes it in. Nothing else in this crate has to know which
//! one ran.
//!
//! # Failure modes first
//!
//! 1. **The hardware backend cannot start here.** The run is refused with the step that failed and what to do about
//!    it (no seat, no GPU node, no permission, no GLES on the GPU) rather than falling back to a headless session
//!    nobody asked for: a session that renders nowhere is a worse answer than an error. A machine with no seat
//!    reaches it: `drm_without_a_seat_is_refused_with_the_step_and_the_way_out`; every [`DrmError`] has its own
//!    hint: `every_drm_failure_names_the_way_out`.
//! 2. **A nested session cannot reach its parent.** The parent socket is named in the
//!    error and the compositor does not start a session with no output:
//!    `a_nested_session_without_a_parent_is_refused`.
//! 3. **The two halves disagree.** The name a startup log prints is the backend that
//!    was built: `the_headless_selection_names_the_backend_it_builds`.
//! 4. **A renderer is named that the backend cannot draw with.** The `compositor.renderer`
//!    setting is named in the refusal and no other renderer is used in its place: a headless or
//!    nested session refuses a renderer this machine cannot create:
//!    `a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting` and
//!    `a_nested_session_with_a_gpu_renderer_contacts_its_parent_instead_of_refusing_the_renderer`.

use gnoblin_backend::Backend as BackendTrait;
use gnoblin_backend::headless::{Headless, HeadlessConfig, HeadlessHandle};
use gnoblin_backend::nested::{Nested, NestedConfig, NestedError};
use gnoblin_backend::render::renderer::RendererKind;
#[cfg(feature = "drm")]
use gnoblin_backend_drm::{Drm, DrmError, RealModeset};
use gnoblin_config::RendererChoice;
use gnoblin_render_frame::frame::SecureTreatment;
use gnoblin_wayland::State;

use crate::Error;

/// Where a test hotplugs outputs and devices of a headless session it started: filled when the session builds its
/// backend, empty before that and for a backend with no virtual outputs. Every clone shares one session.
#[derive(Clone, Debug, Default)]
pub struct OutputTap {
    handle: std::sync::Arc<std::sync::Mutex<Option<HeadlessHandle>>>,
}

impl OutputTap {
    /// A tap with no session behind it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects a 60 Hz virtual output at scale one on the session's next turn. `false` when no headless session
    /// serves yet, or the size has a zero side.
    pub fn connect(&self, connector: &str, width: u32, height: u32) -> bool {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle
            .as_ref()
            .is_some_and(|handle| handle.connect_virtual_output(connector, width, height).is_ok())
    }

    /// Disconnects an output on the session's next turn. `false` when no headless session serves yet.
    pub fn disconnect(&self, output: gnoblin_types::OutputId) -> bool {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle.as_ref().is_some_and(|handle| {
            handle.disconnect_output(output);
            true
        })
    }

    /// Delivers `event` on the session's next turn, as a device would produce it: a
    /// [`gnoblin_backend::InputEvent::DeviceAdded`] announces a virtual device, which the document's `input` section
    /// configures. `false` when no headless session serves yet.
    pub fn push_input(&self, event: gnoblin_backend::InputEvent) -> bool {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle.as_ref().is_some_and(|handle| {
            handle.push_input(event);
            true
        })
    }

    /// What the document's `input` section gave a virtual device, or `None` for a device the session does not have
    /// (or no headless session yet).
    #[must_use]
    pub fn device_settings(
        &self,
        device: gnoblin_types::DeviceId,
    ) -> Option<Vec<gnoblin_backend::input_settings::Setting>> {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle.as_ref()?.device_settings(device)
    }

    fn fill(&self, handle: HeadlessHandle) {
        *self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }
}

/// Builds the [`Error::RendererRefused`] for `renderer` on `backend`, naming the
/// `compositor.renderer` setting. `reason` says why that backend cannot draw with it.
fn refused(renderer: RendererChoice, backend: &'static str, reason: &'static str) -> Error {
    Error::RendererRefused {
        leaf: "compositor.renderer",
        renderer: renderer.as_str(),
        backend,
        reason,
    }
}

/// The renderer the hardware backend scans out with for a `compositor.renderer` setting: `auto` is GLES, until the selection
/// chain makes it Vulkan where a device is eligible, and the CPU renderer, which cannot scan out, is refused with why.
#[cfg(feature = "drm")]
fn drm_renderer(renderer: RendererChoice) -> Result<RendererKind, &'static str> {
    match renderer {
        RendererChoice::Auto | RendererChoice::Gles => Ok(RendererKind::Gles),
        RendererChoice::Vulkan => Ok(RendererKind::Vulkan),
        RendererChoice::Software => Err("the hardware backend scans out with the GLES or the Vulkan renderer only"),
    }
}

/// What a person does about a hardware backend that could not start. One line per kind of failure, because the
/// answer differs: a machine with no seat wants a login session, a machine with no GPU wants another backend, and a
/// GPU that refuses is a permissions problem.
#[cfg(feature = "drm")]
#[must_use]
pub fn drm_hint(error: &DrmError) -> &'static str {
    use gnoblin_backend_drm::session::SessionError;
    use gnoblin_backend_drm::udev::ChoiceError;
    match error {
        DrmError::Session(SessionError::Libseat(_)) => {
            "there is no seat to take: start the session from a display manager or a login on a VT (logind or seatd),              or set compositor.backend to nested or headless"
        }
        DrmError::Session(SessionError::Open { .. }) => {
            "the seat refused the GPU node: the session is not the active one on its seat, or this user may not open              the device"
        }
        DrmError::Session(SessionError::Switch { .. }) => "the seat refused a VT switch",
        DrmError::Discovery(_) => "udev could not be read: check that udevd is running and its socket is reachable",
        DrmError::NoGpu(ChoiceError::NoGpu) => {
            "this machine has no DRM card node: load the kernel's KMS driver, or set compositor.backend to nested or              headless"
        }
        DrmError::NoGpu(ChoiceError::Missing(_) | ChoiceError::NotACardNode(_)) => {
            "GNOBLIN_DRM_DEVICE names a node that cannot drive a display: unset it, or name a /dev/dri/card* node"
        }
        DrmError::Seat(_) => "XDG_SEAT names a different seat from the one this session is on: unset it",
        DrmError::Device(_) => {
            "the GPU refused to be driven: it may be a device without KMS, or another process may hold DRM master"
        }
        DrmError::Renderer(_) => {
            "the GPU cannot give the renderer the hardware backend scans out with (GLES or Vulkan, as compositor.renderer says): name the other one, or use the nested or headless              backend on this machine"
        }
    }
}

/// The backend a session drives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// A virtual output with a software renderer: no DRM, no GPU, no session.
    ///
    /// It is what makes a session runnable anywhere — in CI, in a test and on a
    /// machine with no display — and its pixels are real, so what it proves about the
    /// frame path holds for a DRM session too.
    #[default]
    Headless,
    /// A session hosted in another Wayland compositor: one toplevel per output, on the
    /// parent's socket.
    Nested(NestedConfig),
    /// Direct rendering on the hardware: the GPU, its displays and libinput on the seat this process runs in.
    ///
    /// It scans out with the GLES renderer, or the Vulkan one when `compositor.renderer` says so, and a mode change on a real display stays off unless the process
    /// sets `GNOBLIN_DRM_MODESET=1` (`RealModeset`). A machine that cannot start it is refused with the reason
    /// (`Error::Drm`), never given another backend. A build without the `drm` feature still has this value, because the
    /// selection is data, and refuses it with [`Error::DrmNotBuilt`].
    Drm,
}

impl Backend {
    /// The backend this selection names, drawing with the renderer `renderer` selects, ready for
    /// [`gnoblin_wayland::Server::new`].
    ///
    /// `auto` is the renderer the backend draws with today: software for the headless and nested
    /// backends. A named renderer is never replaced by another.
    ///
    /// `treatment` is what a capture draws in place of a secure layer, and both backends take it.
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] when the parent compositor cannot host the session, `Error::Drm` when the hardware
    /// cannot be opened (`Error::DrmNotBuilt` in a build without the `drm` feature), and [`Error::RendererRefused`] when the backend cannot draw with the named renderer.
    pub fn build(
        self,
        renderer: RendererChoice,
        treatment: SecureTreatment,
    ) -> Result<Box<dyn BackendTrait<State>>, Error> {
        self.build_tapped(renderer, treatment, None)
    }

    /// [`Backend::build`] that also hands a headless backend's hotplug handle to `tap`.
    ///
    /// # Errors
    ///
    /// The same refusals as [`Backend::build`].
    pub fn build_tapped(
        self,
        renderer: RendererChoice,
        treatment: SecureTreatment,
        tap: Option<&OutputTap>,
    ) -> Result<Box<dyn BackendTrait<State>>, Error> {
        match self {
            Backend::Headless => {
                let config = HeadlessConfig {
                    secure_treatment: treatment,
                    ..HeadlessConfig::from_setting(renderer)
                };
                match Headless::with_config(config) {
                    Ok(headless) => {
                        if let Some(tap) = tap {
                            tap.fill(headless.handle());
                        }
                        Ok(Box::new(headless))
                    }
                    Err(_) => Err(refused(
                        renderer,
                        "headless",
                        "the renderer could not be created on this machine; the log says why",
                    )),
                }
            }
            Backend::Nested(config) => {
                let config = NestedConfig {
                    secure_treatment: treatment,
                    renderer: RendererKind::from_setting(renderer),
                    ..config
                };
                match Nested::connect(&config) {
                    Ok(nested) => Ok(Box::new(nested)),
                    // The renderer is created for the first output, so a machine that cannot make it is refused
                    // naming the setting, exactly as a headless session is.
                    Err(NestedError::Render(_)) => Err(refused(
                        renderer,
                        "nested",
                        "the renderer could not be created on this machine; the log says why",
                    )),
                    Err(error) => Err(Error::Nested(error)),
                }
            }
            #[cfg(not(feature = "drm"))]
            Backend::Drm => Err(Error::DrmNotBuilt),
            #[cfg(feature = "drm")]
            Backend::Drm => {
                // The scan-out path draws with GLES or Vulkan. `auto` is GLES here, and the software renderer, which
                // cannot scan out, is refused rather than replaced.
                let kind = match drm_renderer(renderer) {
                    Ok(kind) => kind,
                    Err(reason) => return Err(refused(renderer, "drm", reason)),
                };
                let seat = std::env::var("XDG_SEAT")
                    .ok()
                    .filter(|seat| !seat.is_empty())
                    .unwrap_or_else(|| "seat0".to_owned());
                match Drm::open(&seat, RealModeset::from_process_env(), kind, treatment) {
                    Ok(drm) => Ok(Box::new(drm)),
                    Err(error) => Err(Error::Drm {
                        hint: drm_hint(&error),
                        error,
                    }),
                }
            }
        }
    }

    /// The backend's name, for a log line and a startup report.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Headless => "headless",
            Backend::Nested(_) => "nested",
            Backend::Drm => "drm",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    #[cfg(feature = "drm")]
    use super::drm_hint;
    use crate::Error;
    use gnoblin_backend::nested::NestedConfig;
    use gnoblin_config::RendererChoice;
    use gnoblin_render_frame::frame::SecureTreatment;

    #[test]
    fn the_default_backend_is_the_one_that_starts_anywhere() {
        assert_eq!(Backend::default(), Backend::Headless);
    }

    #[test]
    fn the_headless_selection_names_the_backend_it_builds() {
        // The two halves must agree: a variant that built one backend and named another
        // would mislabel every startup log.
        let backend = Backend::Headless;
        let name = backend.name();
        assert_eq!(
            name,
            backend
                .build(RendererChoice::Auto, SecureTreatment::default())
                .expect("headless builds")
                .name()
        );
    }

    #[cfg(not(feature = "drm"))]
    #[test]
    fn a_build_without_the_drm_feature_refuses_the_hardware_backend_by_name() {
        match Backend::Drm.build(RendererChoice::Auto, SecureTreatment::default()) {
            Err(error @ Error::DrmNotBuilt) => assert_eq!(
                error.to_string(),
                "the drm backend cannot start: this build has no DRM backend; rebuild with feature drm"
            ),
            Err(other) => panic!("drm refused with the wrong error: {other}"),
            Ok(_) => panic!("a build without the drm feature must not start the hardware backend"),
        }
    }

    #[cfg(feature = "drm")]
    #[test]
    fn the_hardware_backend_scans_out_with_gles_or_vulkan_and_refuses_software_by_name() {
        use super::{RendererKind, drm_renderer};
        // Decided before the seat is touched, so these never reach hardware.
        assert_eq!(
            drm_renderer(RendererChoice::Auto),
            Ok(RendererKind::Gles),
            "auto stays GLES here"
        );
        assert_eq!(drm_renderer(RendererChoice::Gles), Ok(RendererKind::Gles));
        assert_eq!(drm_renderer(RendererChoice::Vulkan), Ok(RendererKind::Vulkan));
        assert!(
            drm_renderer(RendererChoice::Software).is_err(),
            "the CPU renderer cannot scan out"
        );
        match Backend::Drm.build(RendererChoice::Software, SecureTreatment::default()) {
            Err(Error::RendererRefused {
                leaf,
                renderer,
                backend,
                ..
            }) => assert_eq!((leaf, renderer, backend), ("compositor.renderer", "software", "drm")),
            Err(other) => panic!("software on drm refused with the wrong error: {other}"),
            Ok(_) => panic!("drm must not draw with software"),
        }
    }

    #[cfg(feature = "drm")]
    #[test]
    fn every_drm_failure_names_the_way_out() {
        use gnoblin_backend_drm::DrmError;
        use gnoblin_backend_drm::session::SessionError;
        use gnoblin_backend_drm::udev::ChoiceError;
        use gnoblin_render_frame::error::RenderError;

        let failures: Vec<(DrmError, &str)> = vec![
            (
                DrmError::Session(SessionError::Libseat("no backend".to_owned())),
                "no seat",
            ),
            (
                DrmError::Session(SessionError::Open {
                    path: "/dev/dri/card0".to_owned(),
                    reason: "denied".to_owned(),
                }),
                "refused the GPU node",
            ),
            (DrmError::Discovery("no socket".to_owned()), "udev"),
            (DrmError::NoGpu(ChoiceError::NoGpu), "no DRM card node"),
            (
                DrmError::NoGpu(ChoiceError::Missing("/dev/x".to_owned())),
                "GNOBLIN_DRM_DEVICE",
            ),
            (DrmError::Seat("seat1 is not seat0".to_owned()), "XDG_SEAT"),
            (DrmError::Device("busy".to_owned()), "DRM master"),
            (DrmError::Renderer(RenderError::Unavailable), "GLES"),
        ];
        let mut hints = std::collections::BTreeSet::new();
        for (error, expected) in failures {
            let text = Error::Drm {
                hint: drm_hint(&error),
                error,
            }
            .to_string();
            assert!(text.starts_with("the drm backend cannot start: "), "{text}");
            assert!(text.contains(expected), "{text} must mention {expected}");
            hints.insert(text);
        }
        assert_eq!(hints.len(), 8, "each failure has its own message");
    }

    #[test]
    fn a_nested_session_without_a_parent_is_refused() {
        let socket = std::env::temp_dir().join(format!("gnoblin-no-parent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        match Backend::Nested(NestedConfig::new(&socket)).build(RendererChoice::Auto, SecureTreatment::default()) {
            Err(Error::Nested(_)) => {}
            Err(other) => panic!("nested refused with the wrong error: {other}"),
            Ok(_) => panic!("a missing parent socket must not start a session"),
        }
    }

    #[test]
    fn software_is_the_headless_default_and_an_explicit_choice() {
        // `auto` keeps today's behaviour, which for a headless session is software, and
        // `software` asks for the same thing by name.
        for choice in [RendererChoice::Auto, RendererChoice::Software] {
            assert!(
                Backend::Headless.build(choice, SecureTreatment::default()).is_ok(),
                "{choice} builds headless"
            );
        }
    }

    #[test]
    fn a_nested_session_with_a_gpu_renderer_contacts_its_parent_instead_of_refusing_the_renderer() {
        // The nested backend draws with every renderer the headless one does, so a GPU renderer is not refused
        // before the parent is reached: the failure here is the missing parent socket, and the renderer is only
        // refused (naming the setting) when the machine cannot create it.
        let socket = std::env::temp_dir().join(format!("gnoblin-renderer-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        for choice in [RendererChoice::Vulkan, RendererChoice::Gles] {
            match Backend::Nested(NestedConfig::new(&socket)).build(choice, SecureTreatment::default()) {
                Err(Error::Nested(_)) => {}
                Err(other) => panic!("{choice} with no parent failed with the wrong error: {other}"),
                Ok(_) => panic!("a missing parent socket must not start a session"),
            }
        }
    }

    #[test]
    fn a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting() {
        // Whether this machine has a Vulkan device decides which half runs: either the renderer
        // is made, or the refusal names the setting, and nothing else is built in its place.
        match Backend::Headless.build(RendererChoice::Vulkan, SecureTreatment::default()) {
            Ok(backend) => println!("a Vulkan device is present: {} built", backend.name()),
            Err(Error::RendererRefused {
                leaf,
                renderer,
                backend,
                ..
            }) => {
                assert_eq!((leaf, renderer, backend), ("compositor.renderer", "vulkan", "headless"));
            }
            Err(other) => panic!("vulkan refused with the wrong error: {other}"),
        }
    }
}

//! The OpenXR-backed [`Runtime`].
//!
//! This is the only module in the crate that uses `unsafe`. The loader and
//! `xrCreateSession` are `unsafe` in the `openxr` crate. Every other call in
//! this file is safe. The session is headless (`XR_MND_headless`): it has no
//! graphics binding and submits no swapchains, so the frame loop runs with
//! empty layer lists.

#![allow(unsafe_code)]

use openxr as xr;
use xr_scene::Pose;

use crate::runtime::{FrameTiming, Runtime, RuntimeError, SessionEvent, SessionState};

/// The `XR_MND_headless` runtime, driven through the OpenXR loader.
///
/// Field order matters for drop: the session and its handles are dropped
/// before the instance, and the instance before the entry (which holds the
/// loaded library).
pub struct OpenxrRuntime {
    session: xr::Session<xr::Headless>,
    waiter: xr::FrameWaiter,
    stream: xr::FrameStream<xr::Headless>,
    /// The local reference space. Poses are returned relative to it.
    local: xr::Space,
    /// The view reference space. Its origin is the head.
    head: xr::Space,
    events: xr::EventDataBuffer,
    instance: xr::Instance,
    _entry: xr::Entry,
}

impl OpenxrRuntime {
    /// Loads the OpenXR loader, creates a headless instance and a session.
    ///
    /// Fails when the loader is missing, when the runtime lacks
    /// `XR_MND_headless`, or when no head-mounted system is present.
    pub fn connect(application_name: &str) -> Result<Self, RuntimeError> {
        // SAFETY: `Entry::load` opens the platform OpenXR loader by name and
        // resolves its symbols. The loader is trusted system code, and the
        // returned `Entry` keeps the library mapped for as long as it lives.
        let entry = unsafe { xr::Entry::load(&()) }.map_err(|e| err("loading the loader", e))?;

        let available = entry
            .enumerate_extensions()
            .map_err(|e| err("enumerating extensions", e))?;
        if !available.mnd_headless {
            return Err(RuntimeError("the runtime does not offer XR_MND_headless".to_owned()));
        }
        let mut enabled = xr::ExtensionSet::default();
        enabled.mnd_headless = true;

        let app = xr::ApplicationInfo {
            application_name,
            application_version: 0,
            engine_name: "gnoblin-xr",
            engine_version: 0,
            api_version: xr::Version::new(1, 0, 0),
        };
        let instance = entry
            .create_instance(&app, &enabled, &[], &())
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

        let local = session
            .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            .map_err(|e| err("creating the local space", e))?;
        let head = session
            .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
            .map_err(|e| err("creating the view space", e))?;

        Ok(Self {
            session,
            waiter,
            stream,
            local,
            head,
            events: xr::EventDataBuffer::new(),
            instance,
            _entry: entry,
        })
    }
}

impl Runtime for OpenxrRuntime {
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

    fn end_frame(&mut self, display_time: i64) -> crate::runtime::Result<()> {
        self.stream
            .end(
                xr::Time::from_nanos(display_time),
                xr::EnvironmentBlendMode::OPAQUE,
                &[],
            )
            .map_err(|e| err("xrEndFrame", e))
    }

    fn locate_head(&mut self, display_time: i64) -> crate::runtime::Result<Option<Pose>> {
        let location = self
            .head
            .locate(&self.local, xr::Time::from_nanos(display_time))
            .map_err(|e| err("locating the head", e))?;
        let flags = location.location_flags;
        let valid = xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID;
        if !flags.contains(valid) {
            return Ok(None);
        }
        let p = location.pose.position;
        let o = location.pose.orientation;
        Ok(Some(Pose {
            position: [f64::from(p.x), f64::from(p.y), f64::from(p.z)],
            orientation: [f64::from(o.x), f64::from(o.y), f64::from(o.z), f64::from(o.w)],
        }))
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

fn err(what: &str, e: impl std::fmt::Debug) -> RuntimeError {
    RuntimeError(format!("{what}: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        runtime.end_frame(timing.predicted_display_time).expect("end frame");
        runtime.end_session().expect("end");
    }
}

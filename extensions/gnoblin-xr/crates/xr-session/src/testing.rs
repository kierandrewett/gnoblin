//! A scripted [`Runtime`] for the frame loop and bridge tests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use xr_input::{ControllerState, HandState, Hands};
use xr_layers::{SubmittedLayer, SwapchainId};
use xr_scene::{Pose, Ray};

use crate::runtime::{Capabilities, FrameTiming, Result, Runtime, RuntimeError, SessionEvent};

/// One step of the script: an event the runtime reports, or a frame it wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Event(SessionEvent),
    Frame(FrameTiming),
}

/// A call the frame loop made on the runtime, with its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    BeginSession,
    EndSession,
    WaitFrame,
    BeginFrame,
    EndFrame(i64),
    LocateHead(i64),
    LocateHands(i64),
    CreateSwapchain { width: u32, height: u32, format: i64 },
    AcquireImage(SwapchainId),
    ReleaseImage(SwapchainId),
    DestroySwapchain(SwapchainId),
}

/// How many images each fake swapchain has.
pub const IMAGES_PER_SWAPCHAIN: u32 = 3;

/// The raw image handle of image `index` of the fake swapchain `id`.
pub const fn image_handle(id: SwapchainId, index: u32) -> u64 {
    id.0 * 1000 + index as u64
}

/// A swapchain the fake created and has not destroyed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeSwapchain {
    pub width: u32,
    pub height: u32,
    pub format: i64,
    /// The index `acquire_image` returns next.
    pub next: u32,
    /// The image acquired and not yet released.
    pub acquired: Option<u32>,
}

/// Plays a script. `poll_event` returns the `Event` steps at the front of the
/// script; `wait_frame` consumes the next `Frame` step and fails if the script
/// holds an event there instead.
///
/// The swapchain calls keep real bookkeeping: ids are numbered from 1, each swapchain has
/// [`IMAGES_PER_SWAPCHAIN`] images, and acquiring twice, releasing an image that is not acquired and using a
/// destroyed swapchain are errors.
#[derive(Debug, Default)]
pub struct FakeRuntime {
    pub script: VecDeque<Step>,
    pub head: Option<Pose>,
    /// One sample per `locate_head` call, consumed from the front before `head` applies: a head that moves, or is
    /// lost for a frame (`None`).
    pub head_script: VecDeque<Option<Pose>>,
    /// What `locate_hands` reports once `hands_script` is used up.
    pub hands: Hands,
    /// One sample per `locate_hands` call, consumed from the front before `hands` applies: a hand that appears, moves
    /// and is lost.
    pub hands_script: VecDeque<Hands>,
    pub capabilities: Capabilities,
    pub calls: Vec<Call>,
    /// The layers of each `end_frame`, with its display time.
    pub submitted: Vec<(i64, Vec<SubmittedLayer>)>,
    /// The swapchains that exist now, by id.
    pub swapchains: BTreeMap<u64, FakeSwapchain>,
    /// The swapchains the runtime has lost: acquiring an image of one fails, and destroying it still works.
    pub lost: BTreeSet<u64>,
    next_swapchain: u64,
    /// Makes the call that is this many (1-based) calls in fail with
    /// `RuntimeError("injected failure")`. The failing call is still recorded.
    pub fail_on_call: Option<usize>,
}

impl FakeRuntime {
    pub fn new(script: impl IntoIterator<Item = Step>) -> Self {
        Self {
            script: script.into_iter().collect(),
            ..Self::default()
        }
    }

    /// A runtime that has a graphics binding: it can create swapchains in the two formats the extension fills, and
    /// allows `max_layer_count` layers.
    pub fn with_graphics(script: impl IntoIterator<Item = Step>, max_layer_count: u32) -> Self {
        Self {
            capabilities: Capabilities {
                swapchains: true,
                max_layer_count,
                color_scale_bias: true,
                swapchain_formats: vec![43, 50],
            },
            ..Self::new(script)
        }
    }

    pub fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|recorded| **recorded == call).count()
    }

    fn record(&mut self, call: Call) -> Result<()> {
        self.calls.push(call);
        if self.fail_on_call == Some(self.calls.len()) {
            return Err(RuntimeError("injected failure".to_owned()));
        }
        Ok(())
    }

    fn swapchain(&mut self, id: SwapchainId) -> Result<&mut FakeSwapchain> {
        self.swapchains
            .get_mut(&id.0)
            .ok_or_else(|| RuntimeError(format!("swapchain {} does not exist", id.0)))
    }
}

impl Runtime for FakeRuntime {
    fn poll_event(&mut self) -> Result<Option<SessionEvent>> {
        match self.script.front() {
            Some(&Step::Event(event)) => {
                self.script.pop_front();
                Ok(Some(event))
            }
            _ => Ok(None),
        }
    }

    fn begin_session(&mut self) -> Result<()> {
        self.record(Call::BeginSession)
    }

    fn end_session(&mut self) -> Result<()> {
        self.record(Call::EndSession)
    }

    fn wait_frame(&mut self) -> Result<FrameTiming> {
        self.record(Call::WaitFrame)?;
        match self.script.pop_front() {
            Some(Step::Frame(timing)) => Ok(timing),
            other => Err(RuntimeError(format!("script expected a frame, found {other:?}"))),
        }
    }

    fn begin_frame(&mut self) -> Result<()> {
        self.record(Call::BeginFrame)
    }

    fn end_frame(&mut self, display_time: i64, layers: &[SubmittedLayer]) -> Result<()> {
        self.record(Call::EndFrame(display_time))?;
        if let Some((id, _)) = self.swapchains.iter().find(|(_, chain)| chain.acquired.is_some()) {
            return Err(RuntimeError(format!(
                "swapchain {id} still has an acquired image at xrEndFrame"
            )));
        }
        self.submitted.push((display_time, layers.to_vec()));
        Ok(())
    }

    fn locate_head(&mut self, display_time: i64) -> Result<Option<Pose>> {
        self.record(Call::LocateHead(display_time))?;
        Ok(self.head_script.pop_front().unwrap_or(self.head))
    }

    fn locate_hands(&mut self, display_time: i64) -> Result<Hands> {
        self.record(Call::LocateHands(display_time))?;
        Ok(self.hands_script.pop_front().unwrap_or(self.hands))
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn create_swapchain(&mut self, width: u32, height: u32, format: i64) -> Result<SwapchainId> {
        self.record(Call::CreateSwapchain { width, height, format })?;
        if !self.capabilities.swapchains {
            return Err(RuntimeError::no_graphics("xrCreateSwapchain"));
        }
        self.next_swapchain += 1;
        let id = SwapchainId(self.next_swapchain);
        self.swapchains.insert(
            id.0,
            FakeSwapchain {
                width,
                height,
                format,
                next: 0,
                acquired: None,
            },
        );
        Ok(id)
    }

    fn swapchain_images(&mut self, id: SwapchainId) -> Result<Vec<u64>> {
        self.swapchain(id)?;
        Ok((0..IMAGES_PER_SWAPCHAIN).map(|index| image_handle(id, index)).collect())
    }

    fn acquire_image(&mut self, id: SwapchainId) -> Result<u32> {
        self.record(Call::AcquireImage(id))?;
        if self.lost.contains(&id.0) {
            return Err(RuntimeError(format!("swapchain {} is lost", id.0)));
        }
        let chain = self.swapchain(id)?;
        if chain.acquired.is_some() {
            return Err(RuntimeError(format!(
                "swapchain {} already has an acquired image",
                id.0
            )));
        }
        let index = chain.next;
        chain.next = (index + 1) % IMAGES_PER_SWAPCHAIN;
        chain.acquired = Some(index);
        Ok(index)
    }

    fn release_image(&mut self, id: SwapchainId) -> Result<()> {
        self.record(Call::ReleaseImage(id))?;
        let chain = self.swapchain(id)?;
        if chain.acquired.take().is_none() {
            return Err(RuntimeError(format!("swapchain {} has no acquired image", id.0)));
        }
        Ok(())
    }

    fn destroy_swapchain(&mut self, id: SwapchainId) -> Result<()> {
        self.record(Call::DestroySwapchain(id))?;
        self.swapchains
            .remove(&id.0)
            .map(|_| ())
            .ok_or_else(|| RuntimeError(format!("swapchain {} does not exist", id.0)))
    }
}

/// The pose whose aim ray is `ray`: at its origin, turned so that `-Z` points along its direction. A direction that is
/// not finite or has no length points straight ahead.
fn pose_along(ray: &Ray) -> Pose {
    let length = ray.direction.iter().map(|c| c * c).sum::<f64>().sqrt();
    let [bx, by, bz] = if length.is_finite() && length > 0.0 {
        ray.direction.map(|c| c / length)
    } else {
        [0.0, 0.0, -1.0]
    };
    // The rotation that takes `(0, 0, -1)` to the direction: the cross product, and one plus the dot product.
    let orientation = if bz > 1.0 - 1.0e-12 {
        // Straight back: a half turn about y.
        [0.0, 1.0, 0.0, 0.0]
    } else {
        let q = [by, -bx, 0.0, 1.0 - bz];
        let norm = q.iter().map(|c| c * c).sum::<f64>().sqrt();
        q.map(|c| c / norm)
    };
    Pose {
        position: ray.origin,
        orientation,
    }
}

/// The right hand a pointer test describes with a [`ControllerState`]: the ray is the aim pose, select the trigger,
/// secondary the grip and the scroll the thumbstick. A sample with no ray and nothing pressed is no controller at all,
/// as a headless runtime reports.
#[must_use]
pub fn hands_from_pointer(state: &ControllerState) -> Hands {
    if *state == ControllerState::default() {
        return Hands::default();
    }
    Hands {
        right: Some(HandState {
            aim: state.ray.as_ref().map(pose_along),
            trigger: state.select,
            grip: state.secondary,
            thumbstick: state.scroll,
            ..HandState::default()
        }),
        ..Hands::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pose_for_a_ray_aims_down_that_ray() {
        for direction in [
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.3, -0.4, -0.5],
            [0.0, 0.0, 1.0],
        ] {
            let ray = Ray {
                origin: [0.1, 1.2, -0.3],
                direction,
            };
            let aimed = Ray::from_pose(&pose_along(&ray));
            let length = direction.iter().map(|c| c * c).sum::<f64>().sqrt();
            assert_eq!(aimed.origin, ray.origin);
            for (got, want) in aimed.direction.into_iter().zip(direction) {
                assert!(
                    (got - want / length).abs() < 1e-9,
                    "{direction:?}: {:?}",
                    aimed.direction
                );
            }
        }
    }

    #[test]
    fn a_sample_with_nothing_in_it_is_no_controller() {
        assert_eq!(hands_from_pointer(&ControllerState::default()), Hands::default());
        let pressed = ControllerState {
            select: 1.0,
            ..ControllerState::default()
        };
        assert_eq!(hands_from_pointer(&pressed).right.map(|hand| hand.trigger), Some(1.0));
    }
}

//! The master-stack layout the tiled tests share.
//!
//! Each test binary that needs tiling includes this file, so the arrangement lives in
//! one place and the tests read the same geometry.

use std::time::Instant;

use gnoblin_types::WorkspaceId;
use gnoblin_wayland::LayoutControl;
use gnoblin_wm::layout::{Camera, Handled, Job, LayoutEngine, LayoutFailure, LayoutOutcome, Params, Placement, Reply};

/// The master-stack arrangement the tiled tests need: the first window takes the left half
/// of the output and the rest share the right half.
struct MasterEngine {
    ready: Option<(gnoblin_wm::layout::Generation, Result<Reply, LayoutFailure>)>,
}

fn master_reply(params: &Params) -> Reply {
    let (width, height) = (params.output_width(), params.output_height());
    let half = width / 2.0;
    let windows = params.windows();
    let Some((master, stack)) = windows.split_first() else {
        return Reply::Handled(Handled {
            placements: Vec::new(),
            camera: Camera::Orthographic,
            effects: Vec::new(),
        });
    };
    let mut placements = vec![Placement::rect(master.id, 0.0, 0.0, half, height)];
    let slot = height / stack.len().max(1) as f64;
    for (index, window) in stack.iter().enumerate() {
        placements.push(Placement::rect(window.id, half, slot * index as f64, half, slot));
    }
    Reply::Handled(Handled {
        placements,
        camera: Camera::Orthographic,
        effects: Vec::new(),
    })
}

impl LayoutEngine for MasterEngine {
    fn kind(&self) -> gnoblin_wm::layout::LayoutKind {
        gnoblin_wm::layout::LayoutKind::Tiling
    }

    fn extent(&self) -> gnoblin_wm::layout::LayoutExtent {
        gnoblin_wm::layout::LayoutExtent::Output
    }

    fn declares(&self, _command: &str) -> bool {
        false
    }

    fn submit(&mut self, job: Job, generation: gnoblin_wm::layout::Generation) -> gnoblin_wm::layout::Ticket {
        let result = match job {
            Job::Arrange(params) | Job::Request(gnoblin_wm::layout::Request { params, .. }) => {
                Ok(master_reply(&params))
            }
            Job::Command(..) => Ok(Reply::Declined),
        };
        self.ready = Some((generation, result));
        gnoblin_wm::layout::Ticket::new(generation)
    }

    fn collect(&mut self, generation: gnoblin_wm::layout::Generation, _deadline: Instant) -> LayoutOutcome {
        match self.ready.take() {
            Some((ready, result)) if ready == generation => LayoutOutcome::Ready { generation, result },
            other => {
                self.ready = other;
                LayoutOutcome::Pending
            }
        }
    }
}

/// Every workspace runs the master-stack engine and no test changes the layout.
pub struct MasterLayouts;

impl LayoutControl for MasterLayouts {
    fn engine(&self, _workspace: WorkspaceId, _output: Option<&str>) -> Box<dyn LayoutEngine> {
        Box::new(MasterEngine { ready: None })
    }

    fn set_workspace(&self, _workspace: WorkspaceId, _selector: &gnoblin_config::LayoutSelector) -> Result<(), String> {
        Err("the test layouts take no runtime choice".to_owned())
    }

    fn set_output(&self, _output: &str, _selector: &gnoblin_config::LayoutSelector) -> Result<(), String> {
        Err("the test layouts take no runtime choice".to_owned())
    }

    fn layout_name(&self, _workspace: WorkspaceId, _output: Option<&str>) -> String {
        "master".to_owned()
    }

    fn output_layout(&self, _output: &str) -> String {
        "master".to_owned()
    }
}

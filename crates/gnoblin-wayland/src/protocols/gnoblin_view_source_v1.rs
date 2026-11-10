//! `gnoblin_view_source_v1`: the owner-only source of View captures.
//!
//! The global `gnoblin_view_source_manager_v1` makes a View node capturable. A source
//! names one View by its node id, and a capture session on that source yields the
//! View's picture through the same `ext_image_copy_capture_v1` path an output uses.
//!
//! Only the owner's privileged connection sees the global ([`Global::ViewSource`]). A
//! trusted service and an application never see it, so neither can read a View's
//! pixels through it.
//!
//! A source is only made for a node the frame loop has published as a View (the capture
//! tables' `views` set, refreshed once per loop iteration). Any other node is a protocol
//! error, `invalid_node`, and no source object is created.
use std::sync::Arc;

use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::image_capture_source::{ImageCaptureSource, ImageCaptureSourceData};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_capture::Target;
use gnoblin_types::NodeId;

use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use crate::protocols::wire::view_source::gnoblin_view_source_manager_v1::{
    self as manager, GnoblinViewSourceManagerV1,
};

/// The global's data: who may bind it.
pub(crate) struct ViewSourceGlobal {
    /// The roster that answers which connection is the owner's privileged one.
    access: Arc<Access>,
}

impl GlobalDispatch2<GnoblinViewSourceManagerV1, State> for ViewSourceGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<GnoblinViewSourceManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ViewSourceManager);
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::ViewSource)
    }
}

/// The manager carries no state of its own: every source it creates is recorded in the
/// capture tables.
#[derive(Debug)]
struct ViewSourceManager;

impl Dispatch2<GnoblinViewSourceManagerV1, State> for ViewSourceManager {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &GnoblinViewSourceManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::CreateSource { id, node } => {
                let node = NodeId::new(u64::from(node));
                if !state.capture.views.contains_key(&node) {
                    resource.post_error(manager::Error::InvalidNode as u32, "the node is not a View");
                    return;
                }
                let source = ImageCaptureSource::new();
                let source_resource = data_init.init(id, ImageCaptureSourceData { source: source.clone() });
                source.add_instance(&source_resource);
                state.capture.sources.insert(source.id(), Target::View(node));
            }
            manager::Request::Destroy => {}
        }
    }
}

/// Registers the global with the access roster that decides who sees it.
pub(crate) fn create_global(display: &DisplayHandle, access: &Arc<Access>) {
    let _ = display.create_global::<State, GnoblinViewSourceManagerV1, ViewSourceGlobal>(
        1,
        ViewSourceGlobal {
            access: Arc::clone(access),
        },
    );
}

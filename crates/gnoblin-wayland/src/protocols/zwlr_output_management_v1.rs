//! `zwlr_output_manager_v1` (version 4): a client reads the outputs and asks for a new layout.
//!
//! A manager announces one head per output, each with its modes, and a `done` event
//! carrying the configuration serial after every change. A client builds a configuration
//! against that serial: it enables or disables heads and sets a mode, position, transform,
//! scale or adaptive sync on each. `test` asks whether the host would accept the layout
//! and changes nothing; `apply` asks the host to make it so. The host is the
//! [`OutputConfigHost`](crate::output_config::OutputConfigHost).
//!
//! ## Failure modes first
//!
//! 1. **A configuration built on an old serial.** Its `apply` or `test` is answered
//!    `cancelled` and nothing changes.
//! 2. **A head removed while the configuration is open.** The host is never asked; the
//!    configuration `failed`s.
//! 3. **Enabling one head twice in one configuration.** Protocol error `already_configured_head`.
//! 4. **A mode from another head.** Protocol error `invalid_mode`.
//! 5. **Setting one property twice on one head.** Protocol error `already_set`.
//! 6. **Reusing a configuration after `apply` or `test`.** Protocol error `already_used`.
//! 7. **A head config for a head not in the configuration.** Protocol error `unconfigured_head`
//!    on apply.
//! 8. **A host that refuses the layout.** The configuration `failed`s; the host is the one
//!    that rolls back. A `test` never changes state.
//! 9. **A client that disconnects mid-configuration.** Its pending requests are dropped
//!    because its objects are no longer alive.
//! 10. **No host support.** The global is not offered.
//!
//! Heads are republished from the host's [`HeadInfo`](crate::output_config::HeadInfo)
//! snapshot each time [`State::publish_output_heads`](crate::State) runs, which bumps the
//! serial and retires the previous head objects with `finished`.

use std::sync::{Arc, Mutex};

use gnoblin_log::{debug_in, trace_in};
use smithay::reexports::wayland_server::backend::{ClientId as ClientKey, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{
        self as config_head, ZwlrOutputConfigurationHeadV1 as ZwlrOutputManagementV1ConfigurationHead,
    },
    zwlr_output_configuration_v1::{self as config, ZwlrOutputConfigurationV1 as ZwlrOutputManagementV1Configuration},
    zwlr_output_head_v1::{self as head, AdaptiveSyncState, ZwlrOutputHeadV1 as ZwlrOutputManagementV1Head},
    zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1 as ZwlrOutputManagementV1},
    zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1 as ZwlrOutputManagementV1Mode},
};

use crate::State;
use crate::access::{Access, Global};
use crate::output_config::{HeadInfo, HeadRequest, ModeRequest};

/// The version of `zwlr_output_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 4;

/// Creates the `zwlr_output_manager_v1` global.
pub(crate) fn global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ZwlrOutputManagementV1, ManagerGlobalData>(
        MANAGER_VERSION,
        ManagerGlobalData {
            access: Arc::clone(access),
        },
    )
}

/// The global data of the manager: who may bind it.
pub(crate) struct ManagerGlobalData {
    access: Arc<Access>,
}

/// The user data of a bound manager object.
pub(crate) struct ManagerUserData;

/// The user data of a head object: the output it describes.
pub(crate) struct HeadUserData {
    id: gnoblin_types::OutputId,
}

/// The user data of a mode object: the head it belongs to and its timing.
pub(crate) struct ModeUserData {
    head: gnoblin_types::OutputId,
    mode: ModeRequest,
}

/// The shared state of one configuration and its configuration-head objects.
#[derive(Default)]
pub(crate) struct ConfigInner {
    /// The serial the client built the configuration on.
    serial: u32,
    /// Whether `apply` or `test` has been sent, after which the object is spent.
    used: bool,
    /// One entry per head the configuration names, in the order the client named them.
    heads: Vec<(gnoblin_types::OutputId, HeadRequest)>,
}

/// The user data of a configuration object.
pub(crate) struct ConfigUserData {
    inner: Arc<Mutex<ConfigInner>>,
}

/// The user data of a configuration-head object: its slot in the configuration.
pub(crate) struct ConfigHeadUserData {
    inner: Arc<Mutex<ConfigInner>>,
    index: usize,
}

/// A configuration the client asked to apply or test, waiting for the host.
pub(crate) struct PendingConfig {
    /// The configuration object that receives `succeeded`, `failed` or `cancelled`.
    pub(crate) object: ZwlrOutputManagementV1Configuration,
    pub(crate) serial: u32,
    pub(crate) requests: Vec<HeadRequest>,
    pub(crate) test_only: bool,
}

/// The output manager's bookkeeping inside the compositor state.
pub(crate) struct ZwlrOutputManagementV1State {
    /// Creates the head and mode objects each announcement sends.
    display: DisplayHandle,
    /// The configuration serial; bumped by every change a client can observe.
    serial: u32,
    /// The heads as the host last reported them.
    heads: Vec<HeadInfo>,
    /// Every bound manager, so a change can be announced to each.
    managers: Vec<ZwlrOutputManagementV1>,
    /// The head objects each manager was told about, retired on the next change.
    head_objects: Vec<(ZwlrOutputManagementV1, Vec<ZwlrOutputManagementV1Head>)>,
    /// Configurations waiting for the host.
    pending: Vec<PendingConfig>,
}

impl ZwlrOutputManagementV1State {
    pub(crate) fn new(display: DisplayHandle) -> Self {
        Self {
            display,
            serial: 0,
            heads: Vec::new(),
            managers: Vec::new(),
            head_objects: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Takes the configurations waiting for the host, leaving none behind.
    pub(crate) fn take_pending(&mut self) -> Vec<PendingConfig> {
        std::mem::take(&mut self.pending)
    }
}

impl State {
    /// Replaces the advertised heads with the host's snapshot and announces the change.
    ///
    /// Every bound manager retires its old head objects with `finished`, receives the new
    /// ones, and is sent `done` with a new serial. A client holding a configuration on the
    /// old serial is cancelled the next time it applies or tests.
    pub(crate) fn publish_output_heads(&mut self, heads: Vec<HeadInfo>) {
        let manager = &mut self.output_manager;
        manager.heads = heads;
        manager.serial = manager.serial.wrapping_add(1);
        let serial = manager.serial;
        let managers = manager.managers.clone();
        for bound in &managers {
            announce(
                bound,
                &mut manager.head_objects,
                &manager.heads,
                serial,
                &manager.display,
            );
        }
    }

    /// Answers the configurations the client asked for, in the order they were made.
    ///
    /// The host runs each one after the dispatch that queued it. A configuration whose serial
    /// is stale is `cancelled`; a head that vanished is `failed`; otherwise the host's answer
    /// is `succeeded` or `failed`.
    pub(crate) fn apply_output_configs(&mut self, host: &mut dyn crate::output_config::OutputConfigHost) -> bool {
        let mut changed = false;
        for pending in self.output_manager.take_pending() {
            if !pending.object.is_alive() {
                continue;
            }
            if pending.serial != self.output_manager.serial {
                pending.object.cancelled();
                debug_in!(
                    "wayland",
                    "[output-manager] a configuration on a stale serial was cancelled"
                );
                continue;
            }
            let known = self.output_manager.heads.iter().map(|head| head.id).collect::<Vec<_>>();
            if pending.requests.iter().any(|request| !known.contains(&request.head)) {
                pending.object.failed();
                debug_in!("wayland", "[output-manager] a configuration named a head that is gone");
                continue;
            }
            match host.apply(&pending.requests, pending.test_only) {
                Ok(()) => {
                    changed |= !pending.test_only;
                    pending.object.succeeded();
                }
                Err(error) => {
                    debug_in!("wayland", %error, "[output-manager] the host refused the configuration");
                    pending.object.failed();
                }
            }
        }
        changed
    }
}

/// Sends a manager its heads and `done` for `serial`, retiring the head objects it had.
fn announce(
    manager: &ZwlrOutputManagementV1,
    head_objects: &mut Vec<(ZwlrOutputManagementV1, Vec<ZwlrOutputManagementV1Head>)>,
    heads: &[HeadInfo],
    serial: u32,
    display: &DisplayHandle,
) {
    let slot = head_objects.iter().position(|(bound, _)| bound == manager);
    if let Some(index) = slot {
        for old in head_objects[index].1.drain(..) {
            old.finished();
        }
    } else {
        head_objects.push((manager.clone(), Vec::new()));
    }
    let Some(client) = manager.client() else {
        return;
    };
    let Some((_, objects)) = head_objects.iter_mut().find(|(bound, _)| bound == manager) else {
        return;
    };
    for info in heads {
        let Ok(head_object) = client.create_resource::<ZwlrOutputManagementV1Head, HeadUserData, State>(
            display,
            manager.version(),
            HeadUserData { id: info.id },
        ) else {
            continue;
        };
        manager.head(&head_object);
        head_object.name(info.name.clone());
        head_object.description(info.description.clone());
        head_object.physical_size(info.physical_mm.0, info.physical_mm.1);
        for (index, mode_info) in info.modes.iter().enumerate() {
            let Ok(mode_object) = client.create_resource::<ZwlrOutputManagementV1Mode, ModeUserData, State>(
                display,
                head_object.version(),
                ModeUserData {
                    head: info.id,
                    mode: ModeRequest {
                        width: mode_info.width,
                        height: mode_info.height,
                        refresh_mhz: mode_info.refresh_mhz,
                    },
                },
            ) else {
                continue;
            };
            head_object.mode(&mode_object);
            mode_object.size(mode_info.width, mode_info.height);
            mode_object.refresh(mode_info.refresh_mhz);
            if mode_info.preferred {
                mode_object.preferred();
            }
            if info.current == Some(index) {
                head_object.current_mode(&mode_object);
            }
        }
        head_object.enabled(i32::from(info.enabled));
        head_object.position(info.position.0, info.position.1);
        head_object
            .transform(wl_output::Transform::try_from(info.transform as u32).unwrap_or(wl_output::Transform::Normal));
        head_object.scale(info.scale);
        head_object.make(info.make.clone());
        head_object.model(info.model.clone());
        head_object.serial_number(info.serial.clone());
        head_object.adaptive_sync(if info.adaptive_sync {
            AdaptiveSyncState::Enabled
        } else {
            AdaptiveSyncState::Disabled
        });
        objects.push(head_object);
    }
    manager.done(serial);
}

impl GlobalDispatch2<ZwlrOutputManagementV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagementV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerUserData);
        state.output_manager.managers.push(manager.clone());
        let serial = state.output_manager.serial;
        let heads = state.output_manager.heads.clone();
        announce(
            &manager,
            &mut state.output_manager.head_objects,
            &heads,
            serial,
            &state.output_manager.display,
        );
        debug_in!("wayland", client = ?client.id(), "[output-manager] a client bound the output manager");
    }

    fn can_view(&self, client: &Client) -> bool {
        self.access.allows(&client.id(), Global::OutputManagement)
    }
}

impl Dispatch2<ZwlrOutputManagementV1, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        manager: &ZwlrOutputManagementV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::CreateConfiguration { id, serial } => {
                let inner = Arc::new(Mutex::new(ConfigInner {
                    serial,
                    ..ConfigInner::default()
                }));
                data_init.init(id, ConfigUserData { inner });
            }
            manager::Request::Stop => {
                manager.finished();
                state.output_manager.managers.retain(|bound| bound != manager);
            }
            _ => trace_in!("wayland", "[output-manager] an unknown manager request"),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientKey, manager: &ZwlrOutputManagementV1) {
        state.output_manager.managers.retain(|bound| bound != manager);
        state.output_manager.head_objects.retain(|(bound, _)| bound != manager);
    }
}

impl Dispatch2<ZwlrOutputManagementV1Head, State> for HeadUserData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _head: &ZwlrOutputManagementV1Head,
        request: head::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            head::Request::Release => {}
            _ => trace_in!("wayland", "[output-manager] an unknown head request"),
        }
    }
}

impl Dispatch2<ZwlrOutputManagementV1Mode, State> for ModeUserData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _mode: &ZwlrOutputManagementV1Mode,
        request: mode::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            mode::Request::Release => {}
            _ => trace_in!("wayland", "[output-manager] an unknown mode request"),
        }
    }
}

impl Dispatch2<ZwlrOutputManagementV1Configuration, State> for ConfigUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        object: &ZwlrOutputManagementV1Configuration,
        request: config::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            config::Request::EnableHead { id, head } => {
                let Some(id_of) = head_id(&head) else {
                    return;
                };
                let mut inner = self.inner.lock().expect("configuration lock");
                if inner.used {
                    object.post_error(
                        config::Error::AlreadyUsed,
                        "the configuration was already applied or tested",
                    );
                    return;
                }
                if inner.heads.iter().any(|(existing, _)| *existing == id_of) {
                    object.post_error(
                        config::Error::AlreadyConfiguredHead,
                        "the head is already part of this configuration",
                    );
                    return;
                }
                let index = inner.heads.len();
                inner.heads.push((
                    id_of,
                    HeadRequest {
                        head: id_of,
                        enabled: true,
                        mode: None,
                        position: None,
                        transform: None,
                        scale: None,
                        adaptive_sync: None,
                    },
                ));
                data_init.init(
                    id,
                    ConfigHeadUserData {
                        inner: Arc::clone(&self.inner),
                        index,
                    },
                );
            }
            config::Request::DisableHead { head } => {
                let Some(id_of) = head_id(&head) else {
                    return;
                };
                let mut inner = self.inner.lock().expect("configuration lock");
                if inner.used {
                    object.post_error(
                        config::Error::AlreadyUsed,
                        "the configuration was already applied or tested",
                    );
                    return;
                }
                if inner.heads.iter().any(|(existing, _)| *existing == id_of) {
                    object.post_error(
                        config::Error::AlreadyConfiguredHead,
                        "the head is already part of this configuration",
                    );
                    return;
                }
                inner.heads.push((
                    id_of,
                    HeadRequest {
                        head: id_of,
                        enabled: false,
                        mode: None,
                        position: None,
                        transform: None,
                        scale: None,
                        adaptive_sync: None,
                    },
                ));
            }
            config::Request::Apply => queue(state, object, &self.inner, false),
            config::Request::Test => queue(state, object, &self.inner, true),
            config::Request::Destroy => {}
            _ => trace_in!("wayland", "[output-manager] an unknown configuration request"),
        }
    }
}

/// Marks the configuration spent and queues it for the host, or cancels it at once when its
/// serial is stale.
fn queue(
    state: &mut State,
    object: &ZwlrOutputManagementV1Configuration,
    inner: &Arc<Mutex<ConfigInner>>,
    test_only: bool,
) {
    let mut guard = inner.lock().expect("configuration lock");
    if guard.used {
        object.post_error(
            config::Error::AlreadyUsed,
            "the configuration was already applied or tested",
        );
        return;
    }
    guard.used = true;
    let requests: Vec<HeadRequest> = guard.heads.iter().map(|(_, request)| request.clone()).collect();
    let serial = guard.serial;
    drop(guard);
    if serial != state.output_manager.serial {
        object.cancelled();
        return;
    }
    state.output_manager.pending.push(PendingConfig {
        object: object.clone(),
        serial,
        requests,
        test_only,
    });
}

/// The output a head object describes, or [`None`] for a head the compositor did not create.
fn head_id(head: &ZwlrOutputManagementV1Head) -> Option<gnoblin_types::OutputId> {
    head.data::<HeadUserData>().map(|data| data.id)
}

impl Dispatch2<ZwlrOutputManagementV1ConfigurationHead, State> for ConfigHeadUserData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        object: &ZwlrOutputManagementV1ConfigurationHead,
        request: config_head::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let mut inner = self.inner.lock().expect("configuration lock");
        if inner.used {
            object.post_error(
                config::Error::AlreadyUsed as u32,
                "the configuration was already applied or tested",
            );
            return;
        }
        let entry = &mut inner.heads[self.index].1;
        match request {
            config_head::Request::SetMode { mode } => {
                if entry.mode.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "the mode was already set");
                    return;
                }
                let Some(mode_data) = mode.data::<ModeUserData>() else {
                    return;
                };
                if mode_data.head != entry.head {
                    object.post_error(config_head::Error::InvalidMode, "the mode belongs to another head");
                    return;
                }
                entry.mode = Some(mode_data.mode);
            }
            config_head::Request::SetCustomMode { width, height, refresh } => {
                if entry.mode.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "the mode was already set");
                    return;
                }
                entry.mode = Some(ModeRequest {
                    width,
                    height,
                    refresh_mhz: refresh,
                });
            }
            config_head::Request::SetPosition { x, y } => {
                if entry.position.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "the position was already set");
                    return;
                }
                entry.position = Some((x, y));
            }
            config_head::Request::SetTransform { transform } => {
                if entry.transform.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "the transform was already set");
                    return;
                }
                match transform {
                    WEnum::Value(value) => entry.transform = Some(value as i32),
                    WEnum::Unknown(_) => {
                        object.post_error(
                            config_head::Error::InvalidTransform,
                            "the transform is outside the enum",
                        );
                    }
                }
            }
            config_head::Request::SetScale { scale } => {
                if entry.scale.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "the scale was already set");
                    return;
                }
                if scale <= 0.0 {
                    object.post_error(config_head::Error::InvalidScale, "the scale is not positive");
                    return;
                }
                entry.scale = Some(scale);
            }
            config_head::Request::SetAdaptiveSync { state } => {
                if entry.adaptive_sync.is_some() {
                    object.post_error(config_head::Error::AlreadySet, "adaptive sync was already set");
                    return;
                }
                match state {
                    WEnum::Value(AdaptiveSyncState::Enabled) => entry.adaptive_sync = Some(true),
                    WEnum::Value(AdaptiveSyncState::Disabled) => entry.adaptive_sync = Some(false),
                    _ => object.post_error(
                        config_head::Error::InvalidAdaptiveSyncState,
                        "the adaptive sync state is neither disabled nor enabled",
                    ),
                }
            }
            _ => trace_in!("wayland", "[output-manager] an unknown configuration-head request"),
        }
    }
}

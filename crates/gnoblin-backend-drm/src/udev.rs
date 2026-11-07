//! Which DRM nodes exist, which one drives the desktop, and what a hotplug does to that
//! choice.
//!
//! A machine can have several GPUs and every one of them has a card node and a render node.
//! The compositor drives one of them — the *primary* — and the choice has to be the same on
//! every boot, so it is a rule rather than "whatever udev enumerated first". This module
//! owns that rule ([`Devices`], [`primary`]) and the udev glue [`Discovery`] that feeds it.
//!
//! # `GNOBLIN_DRM_DEVICE`
//!
//! `GNOBLIN_DRM_DEVICE` is the one override, and this paragraph is its documentation: an
//! absolute path to a *card* node (`/dev/dri/card1`), or to the node itself. It exists for
//! a machine whose primary GPU the boot flag gets wrong and for the smoke test in
//! `docs/drm-smoke.md`, which points the session at a `vkms` or `virtio-gpu` node. It is
//! read in exactly one place ([`Devices::from_environment`]) so the rule cannot drift.
//!
//! The override is a *request*, not a preference: a session whose override names a node the
//! machine does not have refuses to start rather than quietly lighting a monitor on another
//! GPU, because a person who wrote the variable down is debugging exactly that.
//!
//! # Primary selection, in order
//!
//! 1. A card node the override names. A render node is refused: it cannot modeset.
//! 2. The card node the firmware marked `boot_vga`, which is the GPU the machine booted on
//!    and therefore the one with a display attached to it.
//! 3. The first card node in sorted path order, so a machine with no `boot_vga` (a VM, some
//!    hybrid laptops) still picks one deterministically instead of following enumeration
//!    order.
//!
//! # Hotplug
//!
//! Adding a GPU never changes the primary: the outputs on the new device are not being
//! scanned out, and moving the desktop to a GPU that arrived late is a visible flash with
//! nothing gained. Removing the primary is [`DeviceChange::PrimaryLost`], which ends the
//! session's hardware backend: the outputs are gone, the renderer's device is gone, and a
//! session that kept drawing into it would fault inside the driver.
//!
//! # Failure modes first
//!
//! 1. **The override names a node that is not there.** The session is refused with the path
//!    named, instead of falling back to another GPU: `a_missing_override_is_refused`.
//! 2. **The override names a render node.** A render node cannot modeset, so it is refused
//!    with that reason rather than failing later at the first commit:
//!    `an_override_on_a_render_node_is_refused`.
//! 3. **A machine whose GPUs carry no boot flag picks by enumeration order**, which differs
//!    between boots; the choice is sorted by path so it cannot:
//!    `without_a_boot_flag_the_first_path_wins_whatever_the_order`.
//! 4. **A GPU removed mid-session is ignored.** Removing the primary is
//!    [`DeviceChange::PrimaryLost`] and removing any other GPU is recorded:
//!    `removing_the_primary_ends_the_backend_and_no_other_does`.
//! 5. **A GPU that arrives takes over.** Adding a card node never changes the primary:
//!    `an_added_gpu_never_becomes_the_primary`.

use std::path::{Path, PathBuf};

use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::reexports::calloop::LoopHandle;

use crate::log;

/// The environment variable that names the card node to drive.
///
/// It is [`DeviceOverrideSource::device`] that reads it; see the module documentation for what it
/// means.
pub const DEVICE_ENV: &str = "GNOBLIN_DRM_DEVICE";

/// What a `/dev/dri` node can be used for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrmNodeKind {
    /// A card node: it can modeset, which is what a compositor needs.
    Card,
    /// A render node: allocation and rendering only, no modesetting and no master.
    Render,
}

/// A DRM node as the machine describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DrmNode {
    /// The node's path, such as `/dev/dri/card0`.
    pub path: PathBuf,
    /// What the node can be used for.
    pub kind: DrmNodeKind,
    /// Whether the firmware marked this GPU as the one the machine booted on.
    pub boot_vga: bool,
    /// The kernel driver's name, when sysfs names one.
    pub driver: Option<String>,
}

impl DrmNode {
    /// A node described by its path, read from what sysfs says about it.
    ///
    /// A node whose attributes cannot be read is still a node: `boot_vga` is an optional
    /// promise, and a driver name is a log line. Treating an unreadable attribute as a
    /// probe failure would refuse a machine whose GPU works.
    pub fn read(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = if name.starts_with("render") {
            DrmNodeKind::Render
        } else {
            DrmNodeKind::Card
        };
        let device = Path::new("/sys/class/drm").join(&name).join("device");
        Self {
            path: path.to_owned(),
            kind,
            boot_vga: std::fs::read_to_string(device.join("boot_vga"))
                .map(|text| boot_vga(&text))
                .unwrap_or(false),
            driver: driver(&device),
        }
    }

    /// Whether this node can drive a display.
    #[must_use]
    pub fn is_card(&self) -> bool {
        self.kind == DrmNodeKind::Card
    }
}

/// What a `boot_vga` attribute means.
///
/// The attribute holds `1` for the boot GPU and `0` for every other, and it is absent on
/// machines and drivers that do not set it. Anything that is not `1` is "no promise".
#[must_use]
pub fn boot_vga(attribute: &str) -> bool {
    attribute.trim() == "1"
}

/// The kernel driver's name from a device directory, if sysfs has one.
fn driver(device: &Path) -> Option<String> {
    let link = std::fs::read_link(device.join("driver")).ok()?;
    link.file_name().map(|name| name.to_string_lossy().into_owned())
}

/// Where the environment's device override comes from.
///
/// It is a value rather than a bare `std::env::var` call so that a test can exercise both
/// the rule and the session's behaviour with no environment mutation.
#[derive(Clone, Debug, Default)]
pub struct DeviceOverrideSource {
    /// The text of `GNOBLIN_DRM_DEVICE`, or [`None`] when it is unset or empty.
    pub device: Option<PathBuf>,
}

impl DeviceOverrideSource {
    /// The environment this process was started in.
    #[must_use]
    pub fn read() -> Self {
        let device = std::env::var_os(DEVICE_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        Self { device }
    }
}

/// Why no primary GPU could be chosen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ChoiceError {
    /// The override names a path this machine does not have.
    Missing(String),
    /// The override names a node that cannot modeset.
    NotACardNode(String),
    /// The machine has no card node at all.
    NoGpu,
}

impl std::fmt::Display for ChoiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(
                formatter,
                "{DEVICE_ENV} names {path}, which is not a DRM node on this machine"
            ),
            Self::NotACardNode(path) => write!(
                formatter,
                "{DEVICE_ENV} names {path}, which is a render node and cannot drive a display"
            ),
            Self::NoGpu => write!(formatter, "this machine has no DRM card node"),
        }
    }
}

impl std::error::Error for ChoiceError {}

/// Which node is the primary, from a list of what the machine has.
///
/// The list is not assumed to be in any order: the answer is one of the rules in the module
/// documentation, never "the first entry".
///
/// # Errors
///
/// [`ChoiceError`] when an override names a node that is absent or unusable, and
/// [`ChoiceError::NoGpu`] when the machine has no card node.
pub fn primary(nodes: &[DrmNode], environment: &DeviceOverrideSource) -> Result<PathBuf, ChoiceError> {
    if let Some(requested) = &environment.device {
        let Some(node) = nodes.iter().find(|node| node.path == *requested) else {
            return Err(ChoiceError::Missing(requested.display().to_string()));
        };
        if !node.is_card() {
            return Err(ChoiceError::NotACardNode(node.path.display().to_string()));
        }
        return Ok(node.path.clone());
    }
    let mut cards: Vec<&DrmNode> = nodes.iter().filter(|node| node.is_card()).collect();
    cards.sort_by(|left, right| left.path.cmp(&right.path));
    if let Some(boot) = cards.iter().find(|node| node.boot_vga) {
        return Ok(boot.path.clone());
    }
    cards.first().map(|node| node.path.clone()).ok_or(ChoiceError::NoGpu)
}

/// What a device event did to the set of GPUs the backend is driving.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceChange {
    /// The event was about a node that is not a card node, or a duplicate.
    Ignored,
    /// A GPU that is not the primary arrived or left; nothing on screen changes.
    Recorded,
    /// The GPU this session draws with is gone, so the backend cannot continue.
    PrimaryLost,
}

/// The GPUs this session knows about, and which one it drives.
///
/// It is the pure half of discovery: the udev source says "this node appeared" and this
/// decides what that means, so the rules are tested without a device.
#[derive(Clone, Debug)]
pub struct Devices {
    /// The path of the GPU the compositor drives, chosen once by [`primary`].
    primary: PathBuf,
    /// Every GPU node the machine has, sorted by path, so the primary is not necessarily first.
    known: Vec<DrmNode>,
}

impl Devices {
    /// The GPUs a machine has, primary first.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] when no primary can be chosen; the caller reports it and does not
    /// start a hardware session.
    pub fn new(mut nodes: Vec<DrmNode>, environment: &DeviceOverrideSource) -> Result<Self, ChoiceError> {
        let primary = primary(&nodes, environment)?;
        nodes.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self { primary, known: nodes })
    }

    /// The node this session drives.
    #[must_use]
    pub fn primary(&self) -> &Path {
        &self.primary
    }

    /// Whether a path is the primary.
    #[must_use]
    pub fn is_primary(&self, path: &Path) -> bool {
        self.primary == path
    }

    /// Every known node, primary first then by path.
    #[must_use]
    pub fn all(&self) -> &[DrmNode] {
        &self.known
    }

    /// The GPUs that are not being driven, for a log line and the diagnostics report.
    pub fn additional(&self) -> impl Iterator<Item = &DrmNode> {
        self.known
            .iter()
            .filter(|node| node.is_card() && node.path != self.primary)
    }

    /// A node appeared.
    pub fn added(&mut self, node: DrmNode) -> DeviceChange {
        if !node.is_card() {
            return DeviceChange::Ignored;
        }
        if self.known.iter().any(|known| known.path == node.path) {
            return DeviceChange::Ignored;
        }
        log::debug!(area = "udev", path = %node.path.display(), "a GPU arrived");
        self.known.push(node);
        self.known.sort_by(|left, right| left.path.cmp(&right.path));
        DeviceChange::Recorded
    }

    /// A node went away.
    pub fn removed(&mut self, path: &Path) -> DeviceChange {
        let was_known = self.known.iter().any(|node| node.path == path);
        self.known.retain(|node| node.path != path);
        if path == self.primary {
            log::warn!(area = "udev", path = %path.display(), "the GPU this session draws with is gone");
            return DeviceChange::PrimaryLost;
        }
        if was_known {
            log::info!(area = "udev", path = %path.display(), "a GPU this session does not drive went away");
            return DeviceChange::Recorded;
        }
        DeviceChange::Ignored
    }
}

/// What the udev monitor said, waiting for the backend's dispatch.
///
/// The monitor reports a removal by device number and nothing else, so the queue carries
/// the path the number was registered with; a removal this session never saw arrive is
/// dropped rather than guessed at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiscoveryEvent {
    /// A node appeared at this path.
    Added(PathBuf),
    /// A node at this path was removed.
    Removed(PathBuf),
    /// A node at this path changed: a connector was plugged or unplugged, or its modes changed.
    Changed(PathBuf),
}

/// What the monitor and the backend share: the path behind each device number, and the
/// events the backend has not drained yet.
#[derive(Default)]
struct MonitorShared {
    /// The node path udev last reported for a device number.
    paths: std::collections::HashMap<u64, PathBuf>,
    /// Events waiting for [`Discovery::take_events`].
    events: Vec<DiscoveryEvent>,
}

/// The udev side of discovery: the initial enumeration and the monitor that follows it.
///
/// It owns no decision. It reports paths, and [`Devices`] decides what they mean, which is
/// what keeps the hotplug rules testable.
pub struct Discovery {
    /// The monitor that reports hotplug; `None` once [`Discovery::register`] has moved it into the loop.
    udev: Option<UdevBackend>,
    /// The path table and pending events, shared with the loop callback that fills them.
    shared: std::sync::Arc<std::sync::Mutex<MonitorShared>>,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Discovery")
            .field("monitoring", &self.udev.is_some())
            .finish_non_exhaustive()
    }
}

impl Discovery {
    /// Starts monitoring the seat's DRM devices.
    ///
    /// # Errors
    ///
    /// [`std::io::Error`] when udev cannot be reached, which is a machine with no udev (a
    /// bare container) rather than a machine with no GPU.
    pub fn new(seat: &str) -> std::io::Result<Self> {
        Ok(Self {
            udev: Some(UdevBackend::new(seat)?),
            shared: std::sync::Arc::new(std::sync::Mutex::new(MonitorShared::default())),
        })
    }

    /// Every node the machine has now, with the device number udev knows it by.
    ///
    /// The numbers are remembered so that a later removal, which the monitor reports by
    /// number alone, can be turned back into the path the backend knows.
    pub fn snapshot(&self) -> Vec<DrmNode> {
        let Some(udev) = &self.udev else {
            return Vec::new();
        };
        let mut shared = self.shared.lock().unwrap_or_else(|error| error.into_inner());
        udev.device_list()
            .map(|(number, path)| {
                shared.paths.insert(number, path.to_owned());
                DrmNode::read(path)
            })
            .collect()
    }

    /// Registers the monitor with the compositor's loop.
    ///
    /// # Errors
    ///
    /// [`std::io::Error`] when the loop refuses the source, which leaves a session that
    /// would never see a monitor being plugged in.
    pub fn register<S: 'static>(&mut self, loop_handle: &LoopHandle<'static, S>) -> std::io::Result<()> {
        let Some(udev) = self.udev.take() else {
            return Ok(());
        };
        let shared = std::sync::Arc::clone(&self.shared);
        loop_handle
            .insert_source(udev, move |event, _, _data| {
                let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
                let reported = match event {
                    UdevEvent::Added { device_id, path } => {
                        shared.paths.insert(device_id, path.clone());
                        DiscoveryEvent::Added(path)
                    }
                    UdevEvent::Removed { device_id } => match shared.paths.remove(&device_id) {
                        Some(path) => DiscoveryEvent::Removed(path),
                        None => return,
                    },
                    // A change is a driver re-reading a node that is already known: a connector
                    // was plugged, unplugged or changed its modes.
                    UdevEvent::Changed { device_id } => match shared.paths.get(&device_id) {
                        Some(path) => DiscoveryEvent::Changed(path.clone()),
                        None => return,
                    },
                };
                shared.events.push(reported);
            })
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// Everything the monitor reported since the last call.
    pub fn take_events(&self) -> Vec<DiscoveryEvent> {
        let mut shared = self.shared.lock().unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut shared.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, boot_vga: bool) -> DrmNode {
        DrmNode {
            path: PathBuf::from(format!("/dev/dri/{name}")),
            kind: DrmNodeKind::Card,
            boot_vga,
            driver: Some("i915".to_owned()),
        }
    }

    fn render() -> DrmNode {
        DrmNode {
            path: PathBuf::from("/dev/dri/renderD128"),
            kind: DrmNodeKind::Render,
            boot_vga: false,
            driver: Some("i915".to_owned()),
        }
    }

    #[test]
    fn the_boot_gpu_wins_over_the_first_node() {
        let nodes = vec![card("card1", false), card("card0", true)];
        assert_eq!(
            primary(&nodes, &DeviceOverrideSource::default()),
            Ok(PathBuf::from("/dev/dri/card0"))
        );
    }

    #[test]
    fn only_the_primary_going_away_is_fatal() {
        let mut devices = Devices::new(
            vec![card("card0", true), card("card1", false)],
            &DeviceOverrideSource::default(),
        )
        .expect("a primary exists");
        assert!(devices.is_primary(Path::new("/dev/dri/card0")));
        assert_eq!(devices.removed(Path::new("/dev/dri/card1")), DeviceChange::Recorded);
        assert_eq!(devices.removed(Path::new("/dev/dri/card0")), DeviceChange::PrimaryLost);
    }

    #[test]
    fn without_a_boot_flag_the_first_path_wins_whatever_the_order() {
        // Enumeration order differs between boots; the rule is the path, so the answer
        // cannot.
        let nodes = vec![card("card2", false), card("card0", false), card("card1", false)];
        assert_eq!(
            primary(&nodes, &DeviceOverrideSource::default()),
            Ok(PathBuf::from("/dev/dri/card0"))
        );
    }

    #[test]
    fn a_missing_override_is_refused() {
        let nodes = vec![card("card0", true)];
        let environment = DeviceOverrideSource {
            device: Some(PathBuf::from("/dev/dri/card7")),
        };
        assert_eq!(
            primary(&nodes, &environment),
            Err(ChoiceError::Missing("/dev/dri/card7".to_owned()))
        );
    }

    #[test]
    fn an_override_on_a_render_node_is_refused() {
        let nodes = vec![card("card0", true), render()];
        let environment = DeviceOverrideSource {
            device: Some(PathBuf::from("/dev/dri/renderD128")),
        };
        assert_eq!(
            primary(&nodes, &environment),
            Err(ChoiceError::NotACardNode("/dev/dri/renderD128".to_owned()))
        );
    }

    #[test]
    fn an_override_wins_over_the_boot_gpu() {
        let nodes = vec![card("card0", true), card("card1", false)];
        let environment = DeviceOverrideSource {
            device: Some(PathBuf::from("/dev/dri/card1")),
        };
        assert_eq!(primary(&nodes, &environment), Ok(PathBuf::from("/dev/dri/card1")));
    }

    #[test]
    fn a_machine_with_no_card_node_has_no_primary() {
        assert_eq!(
            primary(&[render()], &DeviceOverrideSource::default()),
            Err(ChoiceError::NoGpu)
        );
        assert_eq!(primary(&[], &DeviceOverrideSource::default()), Err(ChoiceError::NoGpu));
    }

    #[test]
    fn removing_the_primary_ends_the_backend_and_no_other_does() {
        let mut devices = Devices::new(
            vec![card("card0", true), card("card1", false)],
            &DeviceOverrideSource::default(),
        )
        .expect("a machine with two GPUs has a primary");
        assert_eq!(devices.removed(Path::new("/dev/dri/card1")), DeviceChange::Recorded);
        assert_eq!(devices.removed(Path::new("/dev/dri/renderD128")), DeviceChange::Ignored);
        assert_eq!(devices.removed(Path::new("/dev/dri/card0")), DeviceChange::PrimaryLost);
    }

    #[test]
    fn an_added_gpu_never_becomes_the_primary() {
        let mut devices =
            Devices::new(vec![card("card1", false)], &DeviceOverrideSource::default()).expect("one GPU is a primary");
        assert_eq!(devices.primary(), Path::new("/dev/dri/card1"));
        assert_eq!(devices.added(card("card0", true)), DeviceChange::Recorded);
        assert_eq!(devices.primary(), Path::new("/dev/dri/card1"));
        assert_eq!(devices.added(card("card0", true)), DeviceChange::Ignored);
        assert_eq!(devices.additional().count(), 1);
    }

    #[test]
    fn a_render_node_is_not_a_gpu_this_session_drives() {
        let mut devices = Devices::new(vec![card("card0", true)], &DeviceOverrideSource::default()).expect("one GPU");
        assert_eq!(devices.added(render()), DeviceChange::Ignored);
        assert_eq!(devices.all().len(), 1);
    }

    #[test]
    fn the_boot_flag_is_exactly_one() {
        assert!(boot_vga("1\n"));
        assert!(!boot_vga("0\n"));
        assert!(!boot_vga(""));
    }
}

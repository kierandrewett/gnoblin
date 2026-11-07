//! The EIS session state machine: a command in, the actions the protocol layer must
//! perform out.
//!
//! Nothing in this module reads a socket, spawns a thread or looks at a clock. The
//! wiring in [`super::server`] owns all of that, maps EI requests to [`EiRequest`]s and
//! performs the [`Action`]s that come back. The split is what makes every rule below
//! testable without a peer, and it is why the rules are stated here rather than in the
//! protocol handler: this type is the owner of "may this event reach the seat".
//!
//! # Identity
//!
//! A [`Service`] holds one [`Context`] per connected client. A context is what the EI
//! protocol calls a connection context: it has the client's name, the capabilities the
//! consent decision granted, the devices the client bound, and the keys, buttons and
//! contacts that are held right now.
//!
//! # Held edges belong to the connection, not to the device list
//!
//! A client that disappears must not leave a key held: the failure is invisible to the
//! user until every later keystroke goes somewhere they did not expect, and there is no
//! later event that could release it. So every path out of a context — a disconnect, a
//! refused grant, a protocol violation, a released device, a `stop_emulating` — takes
//! the held edges with it, in the same order every time, exactly once.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gnoblin_authority::Consent;
use gnoblin_backend::{InputEvent, TouchPhase};
use gnoblin_types::{OutputId, Point, Rect};

/// `KEY_MAX` from `linux/input-event-codes.h`.
///
/// Every code EI carries — a key, a button, a touch identity's tool code — is defined
/// by that header, so anything above this is not a code the kernel can produce. Zero is
/// `KEY_RESERVED` and is refused as well.
const CODE_MAX: u32 = 0x2ff;

/// The logical pixels one wheel notch counts as.
///
/// `ei_scroll.scroll` is a smooth delta in logical pixels and `InputEvent::PointerAxis`
/// is 1/120 of a step, so a smooth delta needs a conversion. Ten pixels per notch is
/// the convention the nested backend already converts at
/// (`gnoblin_backend::nested::input`), so a smooth scroll keeps its speed whichever
/// path it arrived through.
const PIXELS_PER_STEP: f32 = 10.0;

/// One scroll step in the units [`InputEvent::PointerAxis`] states.
const UNITS_PER_STEP: f32 = 120.0;

/// Where an injected event came from.
///
/// The seat treats every origin the same — the lock gate, grabs and bindings run for
/// all of them — so this is not a permission. It exists because an event's source is
/// the first thing a session wants to say when input goes somewhere unexpected, and
/// because a monitor that mixes a device and a remote client cannot otherwise tell
/// them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// A device the backend opened.
    Hardware,
    /// An EI client: a remote desktop session or a test.
    Eis,
}

/// The capabilities a seat or a device can offer.
///
/// The bit values are the EI protocol's own (`ei_seat.capability`'s mask), so a bind
/// from a client is compared without a translation table and a capability cannot be
/// advertised under one number and requested under another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Caps(u8);

impl Caps {
    /// Nothing.
    pub const NONE: Self = Self(0);
    /// Relative pointer motion.
    pub const POINTER: Self = Self(1 << 0);
    /// Absolute pointer motion.
    pub const POINTER_ABSOLUTE: Self = Self(1 << 1);
    /// Key edges.
    pub const KEYBOARD: Self = Self(1 << 2);
    /// Touch contacts.
    pub const TOUCH: Self = Self(1 << 3);
    /// Scroll axes.
    pub const SCROLL: Self = Self(1 << 4);
    /// Pointer button edges.
    pub const BUTTON: Self = Self(1 << 5);
    /// Every capability this server implements.
    pub const ALL: Self = Self(0b0011_1111);

    /// A set from the protocol's own mask, ignoring bits this server does not implement.
    ///
    /// `ei_seat.capability` carries the mask the protocol defines; a client that binds
    /// a bit this server never advertises is refused by the comparison against the
    /// grant, not by a panic on an unknown bit.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// The protocol's mask for this set.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every capability in `other` is in this set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any capability in `other` is in this set.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// The largest set in both.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The smallest set holding both.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The capabilities in this set, in protocol order.
    pub fn iter(self) -> impl Iterator<Item = Self> {
        const EVERY: [Caps; 6] = [
            Caps::POINTER,
            Caps::POINTER_ABSOLUTE,
            Caps::KEYBOARD,
            Caps::TOUCH,
            Caps::SCROLL,
            Caps::BUTTON,
        ];
        EVERY.into_iter().filter(move |capability| self.contains(*capability))
    }
}

impl std::ops::BitOr for Caps {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

impl std::ops::BitOrAssign for Caps {
    fn bitor_assign(&mut self, other: Self) {
        *self = self.union(other);
    }
}

impl std::ops::BitAnd for Caps {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        self.intersection(other)
    }
}

/// One output's rectangle in the EIS coordinate space.
///
/// The space is the session's layout with its top-left corner at the origin, because
/// `ei_device.region` states an unsigned offset and a layout may have a monitor left of
/// or above the primary one. [`Service::set_regions`] translates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// The output the rectangle belongs to.
    pub output: OutputId,
    /// Its rectangle.
    pub rect: Rect,
}

/// The decision that lets a client inject input, and what it covers.
///
/// The state of the decision is [`Consent`], which is `gnoblin-authority`'s vocabulary
/// for every consent prompt in the session, so a portal request and the input it unlocks
/// say "granted" with one type. What this type adds is the scope: the capabilities the
/// decision covers. The default is a pending decision everywhere, which advertises
/// nothing and creates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    consent: Consent,
    capabilities: Caps,
}

impl Default for Grant {
    fn default() -> Self {
        Self::pending()
    }
}

impl Grant {
    /// No decision yet: the seat advertises nothing.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            consent: Consent::Pending,
            capabilities: Caps::NONE,
        }
    }

    /// The user consented to these capabilities, and to nothing else.
    #[must_use]
    pub const fn granted(capabilities: Caps) -> Self {
        Self {
            consent: Consent::Granted,
            capabilities,
        }
    }

    /// The user refused: every pending context ends, and so does every later one.
    #[must_use]
    pub const fn refused() -> Self {
        Self {
            consent: Consent::Refused,
            capabilities: Caps::NONE,
        }
    }

    /// How far the decision has got.
    #[must_use]
    pub const fn consent(self) -> Consent {
        self.consent
    }

    /// What the decision covers.
    #[must_use]
    pub const fn capabilities(self) -> Caps {
        self.capabilities
    }
}

/// What a client wants from the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextType {
    /// The client sends events for the server to inject.
    Sender,
    /// The client receives events the server sends.
    Receiver,
}

/// Why a context stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The client disconnected, or its socket closed.
    Disconnected,
    /// The consent decision was a refusal.
    Refused,
    /// The client broke the protocol.
    Violation(Violation),
}

/// A rule the client broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violation {
    /// The client connected as a receiver, which this server does not implement.
    UnsupportedContext,
    /// The client bound a capability the seat never advertised, or bound twice.
    CapabilityEscalation,
    /// The client bound the seat a second time.
    AlreadyBound,
    /// An event named a device this context does not have.
    UnknownDevice,
    /// An event arrived before the device started emulating.
    NotEmulating,
    /// `start_emulating` arrived while the device was emulating.
    AlreadyEmulating,
    /// A sequence number that does not increase.
    SequenceNotIncreasing,
    /// An event for a capability the device was not created with.
    WrongCapability,
    /// A key code Linux does not define.
    KeyOutOfRange,
    /// A button code Linux does not define.
    ButtonOutOfRange,
    /// A second edge for the same key or button within one frame.
    RepeatedInFrame,
    /// A press for an edge that is already held.
    AlreadyHeld,
    /// An absolute event on a device created without a region.
    NoRegions,
    /// An absolute event outside every region of its device.
    OutsideRegions,
    /// A touch contact that is not down.
    NoSuchContact,
    /// A touch down for a contact that is already down.
    ContactAlreadyDown,
    /// More contacts down at once than the quota allows.
    TooManyContacts,
    /// More events between two frames than the quota allows.
    Flood,
    /// A request whose value cannot be represented, or one the protocol layer refused
    /// before it became a [`EiRequest`].
    Malformed,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::UnsupportedContext => "only sender contexts are served",
            Self::CapabilityEscalation => "a bound capability was never advertised",
            Self::AlreadyBound => "the seat was already bound",
            Self::UnknownDevice => "no such device in this context",
            Self::NotEmulating => "the device is not emulating",
            Self::AlreadyEmulating => "the device is already emulating",
            Self::SequenceNotIncreasing => "the sequence number did not increase",
            Self::WrongCapability => "the device does not have that capability",
            Self::KeyOutOfRange => "the key code is outside the Linux input range",
            Self::ButtonOutOfRange => "the button code is outside the Linux input range",
            Self::RepeatedInFrame => "the same edge arrived twice in one frame",
            Self::AlreadyHeld => "the edge is already held",
            Self::NoRegions => "the device has no region",
            Self::OutsideRegions => "the position is outside every region",
            Self::NoSuchContact => "no such touch contact is down",
            Self::ContactAlreadyDown => "the touch contact is already down",
            Self::TooManyContacts => "too many touch contacts are down",
            Self::Flood => "too many events arrived between two frames",
            Self::Malformed => "the request carries a value the session cannot accept",
        };
        formatter.write_str(text)
    }
}

/// What the protocol layer must do, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Advertise these capabilities on the seat, then send `ei_seat.done`.
    ///
    /// An empty set is the seat with nothing on it, which is what a context whose grant
    /// is still pending sees.
    Seat {
        /// What to advertise.
        capabilities: Caps,
    },
    /// Create a device with these capabilities.
    ///
    /// `regions` is the layout the device was created with, already in the EIS
    /// coordinate space, and is empty for a device that has no absolute capability: EI
    /// fixes a device's region list when the client is told it, so the list travels
    /// with the creation and never changes afterwards.
    Device {
        /// The identity the device's events will name.
        device: DeviceId,
        /// What it can send.
        capabilities: Caps,
        /// Where its absolute positions may land.
        regions: Vec<Region>,
    },
    /// Destroy a device and its interfaces.
    DestroyDevice {
        /// The device to destroy.
        device: DeviceId,
    },
    /// Deliver an event to the seat.
    Inject(Injection),
    /// Tell the client why, then close the connection.
    End {
        /// The reason.
        ending: Ending,
    },
}

/// An event on its way to the seat, with the tag that names its source.
///
/// The field is private and the only constructor is in this module, so a producer
/// cannot claim to be a device, and a session cannot lose the tag between here and the
/// seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Injection {
    origin: Origin,
    event: InputEvent,
}

impl Injection {
    /// Where the event came from.
    #[must_use]
    pub const fn origin(self) -> Origin {
        self.origin
    }

    /// The event itself, in the seat's own vocabulary.
    #[must_use]
    pub const fn event(self) -> InputEvent {
        self.event
    }

    /// The event, for a caller that only needs to deliver it.
    #[must_use]
    pub const fn into_event(self) -> InputEvent {
        self.event
    }
}

/// An EI request, as the state machine sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EiRequest {
    /// `ei_seat.bind`: the client asked for these capabilities.
    Bind {
        /// What it asked for.
        capabilities: Caps,
    },
    /// `ei_device.start_emulating`: the device may send events from now on.
    StartEmulating {
        /// The device.
        device: DeviceId,
        /// The client's sequence number, which must increase.
        sequence: u32,
    },
    /// `ei_device.stop_emulating`: the device sends no more events.
    StopEmulating {
        /// The device.
        device: DeviceId,
    },
    /// `ei_device.frame`: the end of one group of events.
    Frame {
        /// The device.
        device: DeviceId,
    },
    /// `ei_device.release`: the client is done with the device.
    Release {
        /// The device.
        device: DeviceId,
    },
    /// `ei_pointer.motion_relative`.
    RelativeMotion {
        /// The device.
        device: DeviceId,
        /// Horizontal movement in logical pixels.
        dx: f32,
        /// Vertical movement in logical pixels.
        dy: f32,
    },
    /// `ei_pointer_absolute.motion_absolute`, in the device's EIS space.
    AbsoluteMotion {
        /// The device.
        device: DeviceId,
        /// Horizontal position.
        x: f32,
        /// Vertical position.
        y: f32,
    },
    /// `ei_button.button`.
    Button {
        /// The device.
        device: DeviceId,
        /// The Linux button code.
        button: u32,
        /// Whether the button is now held.
        pressed: bool,
    },
    /// `ei_keyboard.key`.
    Key {
        /// The device.
        device: DeviceId,
        /// The Linux key code.
        key: u32,
        /// Whether the key is now held.
        pressed: bool,
    },
    /// `ei_scroll.scroll_discrete`, already in 1/120 of a step.
    ScrollDiscrete {
        /// The device.
        device: DeviceId,
        /// Horizontal movement.
        dx: i32,
        /// Vertical movement.
        dy: i32,
    },
    /// `ei_scroll.scroll`, a smooth delta in logical pixels.
    ScrollDelta {
        /// The device.
        device: DeviceId,
        /// Horizontal movement in logical pixels.
        dx: f32,
        /// Vertical movement in logical pixels.
        dy: f32,
    },
    /// `ei_touchscreen.down`.
    TouchDown {
        /// The device.
        device: DeviceId,
        /// The contact's identity.
        contact: u32,
        /// Horizontal position in the device's EIS space.
        x: f32,
        /// Vertical position in the device's EIS space.
        y: f32,
    },
    /// `ei_touchscreen.motion`.
    TouchMotion {
        /// The device.
        device: DeviceId,
        /// The contact's identity.
        contact: u32,
        /// Horizontal position in the device's EIS space.
        x: f32,
        /// The vertical position in the device's EIS space.
        y: f32,
    },
    /// `ei_touchscreen.up`.
    TouchUp {
        /// The device.
        device: DeviceId,
        /// The contact's identity.
        contact: u32,
    },
    /// `ei_touchscreen.cancel`.
    TouchCancel {
        /// The device.
        device: DeviceId,
        /// The contact's identity.
        contact: u32,
    },
    /// `ei_connection.disconnect`: the client is leaving.
    Disconnect,
}

impl EiRequest {
    /// The device the command names, if it names one.
    ///
    /// Binding and disconnecting are the connection's own two commands; everything else
    /// arrives on a device object, which is why the protocol layer never has to decide
    /// whether an event belongs to the seat or to a device.
    #[must_use]
    pub const fn device(&self) -> Option<DeviceId> {
        match self {
            Self::Bind { .. } | Self::Disconnect => None,
            Self::StartEmulating { device, .. }
            | Self::StopEmulating { device }
            | Self::Frame { device }
            | Self::Release { device }
            | Self::RelativeMotion { device, .. }
            | Self::AbsoluteMotion { device, .. }
            | Self::Button { device, .. }
            | Self::Key { device, .. }
            | Self::ScrollDiscrete { device, .. }
            | Self::ScrollDelta { device, .. }
            | Self::TouchDown { device, .. }
            | Self::TouchMotion { device, .. }
            | Self::TouchUp { device, .. }
            | Self::TouchCancel { device, .. } => Some(*device),
        }
    }
}

/// How much one device may send before the session says no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quota {
    /// Events accepted between two frames.
    ///
    /// A sender client groups the events that belong to one logical hardware event
    /// between two `ei_device.frame` requests, so this bounds one group. It is the
    /// bound on work a single client can queue before the compositor gets its turn.
    pub events_per_frame: u32,
    /// Touch contacts down at once.
    pub contacts: u8,
}

impl Default for Quota {
    fn default() -> Self {
        Self {
            events_per_frame: 256,
            contacts: 32,
        }
    }
}

/// One context's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId(u64);

impl ContextId {
    /// The raw number, for logs.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ContextId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "eis-context:{}", self.0)
    }
}

/// One device's identity within its context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(u64);

impl DeviceId {
    /// The raw number, for logs.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "eis-device:{}", self.0)
    }
}

/// Where a connection context is in its life. Transitions, with the refusals:
///
/// | from           | event                  | to                 | refused when            |
/// |----------------|------------------------|--------------------|-------------------------|
/// | AwaitingConsent| consent granted        | Advertised         | ended (no-op)           |
/// | AwaitingConsent| consent refused        | Ended(Refused)     | ended (no-op)           |
/// | Advertised     | bind                   | Bound              | bound: AlreadyBound     |
/// | AwaitingConsent| bind                   | Bound              | bound: AlreadyBound     |
/// | any live       | bind, caps not granted | (unchanged)        | CapabilityEscalation    |
/// | any live       | disconnect / violation | Ended(ending)      | ended (no-op)           |
/// | Ended          | any command            | (unchanged)        | ignored, nothing sent   |
///
/// Capabilities and devices are data beside the phase, not inside it, so an ended
/// context still answers for what it advertised and dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextPhase {
    /// Waiting for a consent decision: the seat has advertised nothing yet.
    AwaitingConsent,
    /// The seat advertised the granted capabilities; the client may bind.
    Advertised,
    /// The client bound the seat, and the devices for the grant exist.
    Bound,
    /// The context ended, after releasing what it held.
    Ended(Ending),
}

/// Whether a device is emulating, and the last sequence the client started with.
///
/// A sequence must increase across the device's whole life, so the last one survives
/// a stop: `Idle` keeps it and `Emulating` carries the one in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emulation {
    /// Not emulating; `last` is the sequence of the previous emulation, if any.
    Idle { last: Option<u32> },
    /// Emulating under this sequence.
    Emulating { sequence: u32 },
}

/// One device a client bound, and what it holds down.
#[derive(Clone, Debug)]
pub struct EiDevice {
    id: DeviceId,
    capabilities: Caps,
    regions: Vec<Region>,
    emulation: Emulation,
    keys: BTreeSet<u32>,
    buttons: BTreeSet<u32>,
    contacts: BTreeMap<u32, (OutputId, Point)>,
    frame_keys: BTreeSet<u32>,
    frame_buttons: BTreeSet<u32>,
    frame_events: u32,
    dropped: u64,
}

impl EiDevice {
    /// The identity its events name.
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.id
    }

    /// What it can send.
    #[must_use]
    pub const fn capabilities(&self) -> Caps {
        self.capabilities
    }

    /// The layout it was created with, in the EIS coordinate space.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The sequence number of the emulation in progress, if any.
    #[must_use]
    pub const fn emulating(&self) -> Option<u32> {
        match self.emulation {
            Emulation::Emulating { sequence } => Some(sequence),
            Emulation::Idle { .. } => None,
        }
    }

    /// The keys it holds down, in code order.
    pub fn held_keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys.iter().copied()
    }

    /// Releases dropped because the edge was not held, which a session can report.
    #[must_use]
    pub const fn dropped_releases(&self) -> u64 {
        self.dropped
    }

    /// Counts one event against the quota, refusing the frame past it.
    fn count(&mut self, quota: Quota) -> Result<(), Violation> {
        self.frame_events = self.frame_events.saturating_add(1);
        if self.frame_events > quota.events_per_frame {
            return Err(Violation::Flood);
        }
        Ok(())
    }

    /// The events that release everything this device holds, in a fixed order.
    ///
    /// Order is key code, then button code, then contact identity, so the same held set
    /// always produces the same releases. Every edge leaves the set as it is released,
    /// which is what makes a second call release nothing.
    fn release_all(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        for key in std::mem::take(&mut self.keys) {
            actions.push(inject(InputEvent::Key {
                keycode: key,
                pressed: false,
            }));
        }
        for button in std::mem::take(&mut self.buttons) {
            actions.push(inject(InputEvent::PointerButton { button, pressed: false }));
        }
        for (contact, (output, position)) in std::mem::take(&mut self.contacts) {
            actions.push(inject(InputEvent::Touch {
                output,
                slot: contact,
                position,
                phase: TouchPhase::Cancel,
            }));
        }
        self.frame_keys.clear();
        self.frame_buttons.clear();
        self.frame_events = 0;
        actions
    }
}

/// One connected client.
#[derive(Clone, Debug)]
pub struct Context {
    id: ContextId,
    name: Option<String>,
    grant: Grant,
    capabilities: Caps,
    devices: BTreeMap<DeviceId, EiDevice>,
    next_device: u64,
    phase: ContextPhase,
}

impl Context {
    /// A context that has been accepted and not yet told anything.
    fn new(id: ContextId, name: Option<&str>, grant: Grant) -> Self {
        Self {
            id,
            name: name.map(ToOwned::to_owned),
            grant,
            capabilities: Caps::NONE,
            devices: BTreeMap::new(),
            next_device: 0,
            phase: ContextPhase::AwaitingConsent,
        }
    }

    /// The identity the protocol layer knows it by.
    #[must_use]
    pub const fn id(&self) -> ContextId {
        self.id
    }

    /// The name the client gave in the handshake, for logs.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The consent decision this context runs under.
    #[must_use]
    pub const fn grant(&self) -> Grant {
        self.grant
    }

    /// The capabilities the seat advertised.
    #[must_use]
    pub const fn capabilities(&self) -> Caps {
        self.capabilities
    }

    /// The devices the client holds.
    pub fn devices(&self) -> impl Iterator<Item = &EiDevice> {
        self.devices.values()
    }

    /// One device, if the client has it.
    #[must_use]
    pub fn device(&self, device: DeviceId) -> Option<&EiDevice> {
        self.devices.get(&device)
    }

    /// How it ended, if it has.
    #[must_use]
    pub const fn ended(&self) -> Option<Ending> {
        match self.phase {
            ContextPhase::Ended(ending) => Some(ending),
            ContextPhase::AwaitingConsent | ContextPhase::Advertised | ContextPhase::Bound => None,
        }
    }

    /// Releases dropped because the edge was not held.
    ///
    /// A client that releases a key it never pressed made no input the user could see,
    /// so the release is dropped rather than delivered, and counted so a session can
    /// report a client that keeps getting it wrong.
    #[must_use]
    pub fn dropped_releases(&self) -> u64 {
        self.devices.values().map(EiDevice::dropped_releases).sum()
    }

    /// The action that puts this context's capabilities on the wire.
    fn advertise(&mut self) -> Vec<Action> {
        match self.grant.consent() {
            Consent::Pending => vec![Action::Seat {
                capabilities: Caps::NONE,
            }],
            Consent::Granted => {
                self.capabilities = self.grant.capabilities();
                if self.phase == ContextPhase::AwaitingConsent {
                    self.phase = ContextPhase::Advertised;
                }
                vec![Action::Seat {
                    capabilities: self.capabilities,
                }]
            }
            Consent::Refused => self.end(Ending::Refused),
        }
    }

    /// Applies a decision to a context whose grant was pending.
    fn decide(&mut self, grant: Grant) -> Vec<Action> {
        if self.ended().is_some() {
            return Vec::new();
        }
        self.grant = grant;
        self.advertise()
    }

    /// Ends the context, releasing everything it holds first.
    fn end(&mut self, ending: Ending) -> Vec<Action> {
        if self.ended().is_some() {
            return Vec::new();
        }
        self.phase = ContextPhase::Ended(ending);
        let mut actions = Vec::new();
        for device in self.devices.values_mut() {
            actions.append(&mut device.release_all());
        }
        actions.push(Action::End { ending });
        actions
    }

    /// Refuses the command, ending the context.
    fn fault(&mut self, violation: Violation) -> Vec<Action> {
        self.end(Ending::Violation(violation))
    }

    /// A device for these capabilities, created with the current layout.
    fn create_device(&mut self, capabilities: Caps, layout: &[Region]) -> (DeviceId, Vec<Region>) {
        let id = DeviceId(self.next_device);
        self.next_device += 1;
        let regions = if capabilities.intersects(Caps::POINTER_ABSOLUTE | Caps::TOUCH) {
            eis_space(layout)
        } else {
            Vec::new()
        };
        let device = EiDevice {
            id,
            capabilities,
            regions: regions.clone(),
            emulation: Emulation::Idle { last: None },
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            contacts: BTreeMap::new(),
            frame_keys: BTreeSet::new(),
            frame_buttons: BTreeSet::new(),
            frame_events: 0,
            dropped: 0,
        };
        self.devices.insert(id, device);
        (id, regions)
    }

    /// Applies one command, refusing it when it breaks a rule.
    fn handle(&mut self, command: EiRequest, quota: Quota, layout: &[Region]) -> Result<Vec<Action>, Violation> {
        if self.ended().is_some() {
            // A context that has ended accepts nothing more: every event after the end
            // would be one the client sent before it heard about it.
            return Ok(Vec::new());
        }
        if let EiRequest::Disconnect = command {
            return Ok(self.end(Ending::Disconnected));
        }
        if let EiRequest::Bind { capabilities } = command {
            return self.bind(capabilities, layout);
        }
        if let EiRequest::Release { device } = command {
            let Some(mut released) = self.devices.remove(&device) else {
                return Err(Violation::UnknownDevice);
            };
            // The client is done with the device: everything it held is released, then the
            // device and its interfaces are destroyed. Nothing it sends can name it after
            // this, which the map removal is what enforces.
            let mut actions = released.release_all();
            actions.push(Action::DestroyDevice { device: released.id });
            return Ok(actions);
        }
        let device = command.device().ok_or(Violation::UnknownDevice)?;
        let device = self.devices.get_mut(&device).ok_or(Violation::UnknownDevice)?;
        match command {
            EiRequest::StartEmulating { sequence, .. } => {
                let last = match device.emulation {
                    Emulation::Emulating { .. } => return Err(Violation::AlreadyEmulating),
                    Emulation::Idle { last } => last,
                };
                // The sequence must increase across the device's whole life, not only
                // while one emulation runs: a restart that repeats a number is the client
                // losing track of its own edits, and the seat would have no way to tell
                // which emulation an event belonged to.
                if last.is_some_and(|last| sequence <= last) {
                    return Err(Violation::SequenceNotIncreasing);
                }
                device.emulation = Emulation::Emulating { sequence };
                device.frame_keys.clear();
                device.frame_buttons.clear();
                device.frame_events = 0;
                Ok(Vec::new())
            }
            EiRequest::StopEmulating { .. } => {
                let Emulation::Emulating { sequence } = device.emulation else {
                    return Err(Violation::NotEmulating);
                };
                device.emulation = Emulation::Idle { last: Some(sequence) };
                Ok(device.release_all())
            }
            EiRequest::Frame { .. } => {
                if device.emulating().is_none() {
                    return Err(Violation::NotEmulating);
                }
                device.frame_keys.clear();
                device.frame_buttons.clear();
                device.frame_events = 0;
                Ok(Vec::new())
            }
            EiRequest::RelativeMotion { dx, dy, .. } => {
                admits(device, Caps::POINTER, quota)?;
                if !dx.is_finite() || !dy.is_finite() {
                    return Err(Violation::Malformed);
                }
                Ok(vec![inject(InputEvent::PointerMotionRelative {
                    dx: round(f64::from(dx)),
                    dy: round(f64::from(dy)),
                })])
            }
            EiRequest::AbsoluteMotion { x, y, .. } => {
                admits(device, Caps::POINTER_ABSOLUTE, quota)?;
                if !x.is_finite() || !y.is_finite() {
                    return Err(Violation::Malformed);
                }
                let (output, position) = locate(device, x, y)?;
                Ok(vec![inject(InputEvent::PointerMotion { output, position })])
            }
            EiRequest::Button { button, pressed, .. } => {
                admits(device, Caps::BUTTON, quota)?;
                if button == 0 || button > CODE_MAX {
                    return Err(Violation::ButtonOutOfRange);
                }
                if !device.frame_buttons.insert(button) {
                    return Err(Violation::RepeatedInFrame);
                }
                if pressed {
                    if !device.buttons.insert(button) {
                        return Err(Violation::AlreadyHeld);
                    }
                } else if !device.buttons.remove(&button) {
                    device.dropped = device.dropped.saturating_add(1);
                    return Ok(Vec::new());
                }
                Ok(vec![inject(InputEvent::PointerButton { button, pressed })])
            }
            EiRequest::Key { key, pressed, .. } => {
                admits(device, Caps::KEYBOARD, quota)?;
                if key == 0 || key > CODE_MAX {
                    return Err(Violation::KeyOutOfRange);
                }
                if !device.frame_keys.insert(key) {
                    return Err(Violation::RepeatedInFrame);
                }
                if pressed {
                    if !device.keys.insert(key) {
                        return Err(Violation::AlreadyHeld);
                    }
                } else if !device.keys.remove(&key) {
                    device.dropped = device.dropped.saturating_add(1);
                    return Ok(Vec::new());
                }
                Ok(vec![inject(InputEvent::Key { keycode: key, pressed })])
            }
            EiRequest::ScrollDiscrete { dx, dy, .. } => {
                admits(device, Caps::SCROLL, quota)?;
                Ok(vec![inject(InputEvent::PointerAxis {
                    horizontal: dx,
                    vertical: dy,
                })])
            }
            EiRequest::ScrollDelta { dx, dy, .. } => {
                admits(device, Caps::SCROLL, quota)?;
                if !dx.is_finite() || !dy.is_finite() {
                    return Err(Violation::Malformed);
                }
                Ok(vec![inject(InputEvent::PointerAxis {
                    horizontal: steps(dx),
                    vertical: steps(dy),
                })])
            }
            EiRequest::TouchDown { contact, x, y, .. } => {
                admits(device, Caps::TOUCH, quota)?;
                if device.contacts.len() >= usize::from(quota.contacts) {
                    return Err(Violation::TooManyContacts);
                }
                let (output, position) = locate(device, x, y)?;
                if device.contacts.insert(contact, (output, position)).is_some() {
                    return Err(Violation::ContactAlreadyDown);
                }
                Ok(vec![inject(InputEvent::Touch {
                    output,
                    slot: contact,
                    position,
                    phase: TouchPhase::Down,
                })])
            }
            EiRequest::TouchMotion { contact, x, y, .. } => {
                admits(device, Caps::TOUCH, quota)?;
                let (output, position) = locate(device, x, y)?;
                let Some(slot) = device.contacts.get_mut(&contact) else {
                    return Err(Violation::NoSuchContact);
                };
                *slot = (output, position);
                Ok(vec![inject(InputEvent::Touch {
                    output,
                    slot: contact,
                    position,
                    phase: TouchPhase::Motion,
                })])
            }
            EiRequest::TouchUp { contact, .. } | EiRequest::TouchCancel { contact, .. } => {
                admits(device, Caps::TOUCH, quota)?;
                let Some((output, position)) = device.contacts.remove(&contact) else {
                    return Err(Violation::NoSuchContact);
                };
                let phase = if matches!(command, EiRequest::TouchUp { .. }) {
                    TouchPhase::Up
                } else {
                    TouchPhase::Cancel
                };
                Ok(vec![inject(InputEvent::Touch {
                    output,
                    slot: contact,
                    position,
                    phase,
                })])
            }
            EiRequest::Bind { .. } | EiRequest::Disconnect | EiRequest::Release { .. } => Ok(Vec::new()),
        }
    }

    /// Creates the devices the client asked for, and only those the grant covers.
    fn bind(&mut self, requested: Caps, layout: &[Region]) -> Result<Vec<Action>, Violation> {
        match self.phase {
            ContextPhase::Bound => return Err(Violation::AlreadyBound),
            // Unreachable through `handle`, which refuses every command after the end.
            ContextPhase::Ended(_) => return Ok(Vec::new()),
            ContextPhase::AwaitingConsent | ContextPhase::Advertised => {}
        }
        if !self.capabilities.contains(requested) {
            return Err(Violation::CapabilityEscalation);
        }
        self.phase = ContextPhase::Bound;
        // One device per kind, because a client that asks for a pointer and a keyboard
        // gets the two things a desktop has rather than one device that is both. A
        // capability the grant does not cover is already excluded by the check above;
        // an empty group creates nothing.
        const GROUPS: [Caps; 3] = [
            Caps::POINTER
                .union(Caps::POINTER_ABSOLUTE)
                .union(Caps::SCROLL)
                .union(Caps::BUTTON),
            Caps::KEYBOARD,
            Caps::TOUCH,
        ];
        let mut actions = Vec::new();
        for group in GROUPS {
            let capabilities = requested.intersection(group);
            if capabilities.is_empty() {
                continue;
            }
            let (device, regions) = self.create_device(capabilities, layout);
            actions.push(Action::Device {
                device,
                capabilities,
                regions,
            });
        }
        Ok(actions)
    }
}

/// The set of contexts, the consent decision and the layout they share.
#[derive(Clone, Debug)]
pub struct Service {
    grant: Grant,
    regions: Vec<Region>,
    quota: Quota,
    contexts: BTreeMap<ContextId, Context>,
    next_context: u64,
}

impl Service {
    /// A service with a consent decision, a layout and a quota.
    ///
    /// The decision applies to contexts that connect from now on; a decision that
    /// arrives later is [`Service::set_grant`].
    #[must_use]
    pub fn new(grant: Grant, regions: Vec<Region>, quota: Quota) -> Self {
        Self {
            grant,
            regions,
            quota,
            contexts: BTreeMap::new(),
            next_context: 0,
        }
    }

    /// Every context, in identity order, including the ones that have ended.
    pub fn contexts(&self) -> impl Iterator<Item = &Context> {
        self.contexts.values()
    }

    /// One context.
    #[must_use]
    pub fn context(&self, id: ContextId) -> Option<&Context> {
        self.contexts.get(&id)
    }

    /// Accepts a client, which gets a context and whatever its grant is worth now.
    pub fn connect(&mut self, name: Option<&str>, context_type: ContextType) -> (ContextId, Vec<Action>) {
        let id = ContextId(self.next_context);
        self.next_context += 1;
        let mut context = Context::new(id, name, self.grant);
        let actions = if context_type == ContextType::Sender {
            context.advertise()
        } else {
            context.fault(Violation::UnsupportedContext)
        };
        self.contexts.insert(id, context);
        (id, actions)
    }

    /// Applies a consent decision to every context that is still waiting for one.
    ///
    /// Returns the actions paired with the context each belongs to, because a decision
    /// is the one command that reaches more than one client.
    pub fn set_grant(&mut self, grant: Grant) -> Vec<(ContextId, Action)> {
        self.grant = grant;
        let mut told = Vec::new();
        for context in self.contexts.values_mut() {
            if context.grant.consent() != Consent::Pending || context.ended().is_some() {
                continue;
            }
            for action in context.decide(grant) {
                told.push((context.id, action));
            }
        }
        told
    }

    /// Replaces the layout for devices created from now on.
    ///
    /// An existing device keeps the regions it was created with: EI fixes a device's
    /// region list when the client is told it, so moving a device's regions would be a
    /// protocol lie. A caller whose layout changed ends the session, or accepts that
    /// existing clients keep addressing the old geometry until they reconnect.
    pub fn set_regions(&mut self, regions: Vec<Region>) {
        self.regions = regions;
    }

    /// Applies one command from one context.
    pub fn apply(&mut self, id: ContextId, command: EiRequest) -> Vec<Action> {
        let Self {
            contexts,
            regions,
            quota,
            ..
        } = self;
        let Some(context) = contexts.get_mut(&id) else {
            return Vec::new();
        };
        match context.handle(command, *quota, regions) {
            Ok(actions) => actions,
            Err(violation) => context.fault(violation),
        }
    }

    /// The client's connection went away: release what it held and end the context.
    pub fn disconnect(&mut self, id: ContextId) -> Vec<Action> {
        self.end(id, Ending::Disconnected)
    }

    /// Ends a context with the reason the caller decided.
    ///
    /// Returns what the protocol layer must still perform — the releases and the
    /// `End` — because a context that ends has to give back everything it held before the
    /// connection goes.
    pub fn end(&mut self, id: ContextId, ending: Ending) -> Vec<Action> {
        match self.contexts.get_mut(&id) {
            Some(context) => context.end(ending),
            None => Vec::new(),
        }
    }

    /// Forgets a context that has ended and whose connection is gone.
    pub fn forget(&mut self, id: ContextId) -> Option<Context> {
        self.contexts.remove(&id)
    }
}

/// An action that delivers one event with the EIS origin.
fn inject(event: InputEvent) -> Action {
    Action::Inject(Injection {
        origin: Origin::Eis,
        event,
    })
}

/// The layout with its top-left corner at the origin.
///
/// `ei_device.region` states an unsigned offset, and a multi-monitor layout commonly
/// extends left of or above the primary output, so every device's regions are
/// translated by the layout's minimum corner. Absolute events then arrive in the same
/// translated space, which is why hit testing and the output-local position both work
/// without knowing the translation.
fn eis_space(layout: &[Region]) -> Vec<Region> {
    let (min_x, min_y) = layout
        .iter()
        .fold((0, 0), |(x, y), region| (x.min(region.rect.x), y.min(region.rect.y)));
    layout
        .iter()
        .map(|region| Region {
            output: region.output,
            rect: Rect::new(
                region.rect.x - min_x,
                region.rect.y - min_y,
                region.rect.width,
                region.rect.height,
            ),
        })
        .collect()
}

/// The output and output-local position an absolute point names.
fn locate(device: &EiDevice, x: f32, y: f32) -> Result<(OutputId, Point), Violation> {
    if device.regions.is_empty() {
        return Err(Violation::NoRegions);
    }
    for region in &device.regions {
        let left = f64::from(region.rect.x);
        let top = f64::from(region.rect.y);
        let right = left + f64::from(region.rect.width);
        let bottom = top + f64::from(region.rect.height);
        let (x, y) = (f64::from(x), f64::from(y));
        if x >= left && x < right && y >= top && y < bottom {
            return Ok((region.output, Point::new(round(x - left), round(y - top))));
        }
    }
    Err(Violation::OutsideRegions)
}

/// A float rounded to the nearest integer, saturating rather than wrapping.
fn round(value: f64) -> i32 {
    value.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// A smooth scroll delta in 1/120 of a step.
fn steps(delta: f32) -> i32 {
    let steps = (delta / PIXELS_PER_STEP * UNITS_PER_STEP).round();
    if steps.is_finite() {
        steps.clamp(i32::MIN as f32, i32::MAX as f32) as i32
    } else {
        0
    }
}

/// The checks every event on a device passes before its value is looked at.
///
/// The order is the rule's order: the device must have the capability the event needs,
/// the client must have said it is emulating, and the event must fit in the frame budget.
/// A refusal ends the session either way; the order is what makes the reason the first
/// thing that is wrong rather than the last.
fn admits(device: &mut EiDevice, capability: Caps, quota: Quota) -> Result<(), Violation> {
    if !device.capabilities.contains(capability) {
        return Err(Violation::WrongCapability);
    }
    if device.emulating().is_none() {
        return Err(Violation::NotEmulating);
    }
    device.count(quota)
}

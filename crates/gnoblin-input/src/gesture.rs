//! The touchpad gesture recogniser: a pure state machine over contact frames and ticks.
//!
//! The mechanism layer reads contacts from libinput (slot, phase, position) and feeds
//! each hardware frame here with the caller's tick. This module decides when a gesture
//! begins, which kind it is, what movement it produced, and when it ends or is
//! cancelled, and it emits typed [`GestureEvent`]s. Nothing here reads a clock, touches
//! a device or knows a screen: the caller converts ticks and device units.
//!
//! # Frames, not single contacts
//!
//! Contacts are fed one frame at a time because a decision about one contact alone is
//! wrong: in a two-finger swipe, the frame in which the first finger has moved and the
//! second has not looks exactly like a pinch if it is judged on its own. libinput
//! delivers a touch frame boundary for the same reason. One frame therefore produces at
//! most one gesture event, and every contact in the frame is applied before anything is
//! judged.
//!
//! # Recognition rules
//!
//! A gesture can only begin while 2 to 5 contacts are down, which is the range
//! `gnoblin.configure.touchpad_gestures` accepts; a sixth contact drops the gather
//! instead of reporting a finger count no binding could match, and a contact that joins
//! a gesture in progress is ignored because the finger count was fixed when it began.
//!
//! A gesture begins at the first frame where either
//!
//! - the centroid has moved at least [`GestureConfig::swipe_threshold`], or
//! - the mean distance of the contacts from their centroid has changed by at least
//!   [`GestureConfig::pinch_threshold`] of its starting value.
//!
//! The kind is whichever crossed its threshold by the larger fraction, so the first
//! movement decides and the answer does not depend on the order two equal movements
//! happened to arrive in. [`GestureKind::Swipe`] wins a tie. A swipe whose starting
//! centroid was inside the band of [`GestureConfig::edge`] is reported with that
//! [`Edge`], which is what lets a shell treat an edge swipe as its own gesture.
//!
//! # Travel and thresholds
//!
//! [`GestureEvent::End`] reports `travel` as the whole distance the centroid moved,
//! including the movement that crossed the recognition threshold. A gesture binding's
//! `threshold` is documented as "minimum movement before the path can match", and
//! distance travelled before recognition is movement, so it counts. The C router
//! measured only the movement after its begin event, which made a configured threshold
//! and its own begin threshold add up; this crate measures the movement the user made.
//!
//! # Losing the end of a stream
//!
//! Contact events can be lost: a device is removed, a terminal event never arrives.
//! [`Recogniser::expire`] cancels a stream that has not advanced for
//! [`GestureConfig::stale_ticks`] ticks and drops a gather whose contacts have gone
//! quiet, so a lost event cannot leave the recogniser waiting forever or make the next
//! gesture continue the old one. Ending or cancelling a stream forgets every contact:
//! the fingers still down after a four-finger pinch cannot start a second gesture.

use std::collections::BTreeMap;

use crate::tick::Tick;

/// The fewest contacts a gesture needs.
pub const MIN_FINGERS: u8 = 2;

/// The most contacts a gesture can have.
pub const MAX_FINGERS: u8 = 5;

/// Points each path is resampled to before it is compared.
pub const PATH_SAMPLES: usize = 16;

/// The most points a reported path keeps, so a long gesture stays bounded.
pub const MAX_PATH_POINTS: usize = 128;

/// The smallest movement, in caller units, that adds a point to the reported path.
const PATH_STEP: f64 = 1.0;

/// Below this distance the contacts are effectively at one point, so a ratio of
/// spreads has no meaning.
const SPREAD_EPSILON: f64 = 0.001;

/// Which part of a contact's life an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContactPhase {
    /// A finger touched down.
    Begin,
    /// A finger moved.
    Update,
    /// A finger lifted.
    End,
    /// The device or the seat cancelled the whole stream.
    Cancel,
}

/// One contact event, in the caller's units.
///
/// The position is the finger's own position, not a delta. Two axes are used today; a
/// contact gains a third axis by carrying a third coordinate, and until then a caller
/// with a third dimension reports its projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// The contact slot, stable for the life of the contact.
    pub slot: u8,
    /// Which part of the contact's life this is.
    pub phase: ContactPhase,
    /// Horizontal position.
    pub x: f64,
    /// Vertical position, increasing downward.
    pub y: f64,
}

/// The kind of gesture that was recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GestureKind {
    /// The contacts moved together.
    Swipe,
    /// The contacts spread or gathered.
    Pinch,
}

impl GestureKind {
    /// The name the configuration uses for this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Swipe => "swipe",
            Self::Pinch => "pinch",
        }
    }
}

/// The touchpad edge a swipe started at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    /// The near edge, where the thumb rests.
    Bottom,
    /// The far edge.
    Top,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
}

impl Edge {
    /// The name a shell event uses for this edge.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Which phase of a gesture an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GesturePhase {
    /// The gesture has been recognised and claims the movement from here.
    Begin,
    /// Movement since the previous update.
    Update,
    /// The contacts lifted; the gesture is over.
    End,
    /// The gesture ended without an end event.
    Cancel,
}

/// Why a gesture was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CancelReason {
    /// A contact reported a cancel phase.
    Cancelled,
    /// The stream stopped advancing and was retired by [`Recogniser::expire`].
    Stale,
    /// The device went away.
    DeviceRemoved,
}

/// The movement a gesture made, relative to where it started.
///
/// The first point is always `(0.0, 0.0)` and the rest are positions relative to the
/// start centroid, so a configured path is compared against the shape of the movement
/// and not against where on the touchpad it happened. A third axis joins as a third
/// value per point; [`path_matches`] then compares it too, and today's configured paths
/// keep working with a zero third value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GesturePath(Vec<(f64, f64)>);

impl GesturePath {
    /// Build a path from points, starting at `(0.0, 0.0)`.
    ///
    /// Used by callers that replay a recorded gesture, and by tests.
    pub fn from_points(points: Vec<(f64, f64)>) -> Self {
        Self(points)
    }

    /// The points, starting at `(0.0, 0.0)`.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.0
    }
}

/// A typed gesture event.
///
/// Every phase carries the gesture's identity, so a consumer that starts listening in
/// the middle of a stream still knows what it is watching.
#[derive(Clone, Debug, PartialEq)]
pub enum GestureEvent {
    /// A gesture was recognised. From here until the end or cancel it owns the
    /// movement, so a shell can animate without another handler also consuming it.
    Begin {
        /// When it began, on the caller's clock.
        tick: Tick,
        /// Swipe or pinch.
        kind: GestureKind,
        /// How many contacts it has.
        fingers: u8,
        /// The edge it started at, for a swipe.
        edge: Option<Edge>,
    },
    /// Movement since the previous update.
    Update {
        /// When this update was recognised.
        tick: Tick,
        /// Swipe or pinch.
        kind: GestureKind,
        /// How many contacts it has.
        fingers: u8,
        /// The edge it started at, for a swipe.
        edge: Option<Edge>,
        /// Horizontal centroid movement since the previous update.
        dx: f64,
        /// Vertical centroid movement since the previous update.
        dy: f64,
        /// Mean distance of the contacts from their centroid, relative to the start.
        /// One means unchanged; above one is a spread, below one a gather.
        scale: f64,
        /// Mean rotation of the contacts around their centroid, in radians.
        angle_delta: f64,
    },
    /// The gesture ended and its movement is final.
    End {
        /// When it ended.
        tick: Tick,
        /// Swipe or pinch.
        kind: GestureKind,
        /// How many contacts it began with.
        fingers: u8,
        /// The edge it started at, for a swipe.
        edge: Option<Edge>,
        /// The path it travelled.
        path: GesturePath,
        /// Total distance the centroid moved, including the movement before it was
        /// recognised.
        travel: f64,
        /// Final scale: above one for a spread, below one for a gather.
        scale: f64,
        /// Ticks from begin to end.
        duration: u64,
    },
    /// The gesture ended without an end event.
    Cancel {
        /// When the cancel was noticed.
        tick: Tick,
        /// Which kind of gesture was cancelled.
        kind: GestureKind,
        /// How many contacts it began with.
        fingers: u8,
        /// Why.
        reason: CancelReason,
    },
}

impl GestureEvent {
    /// Which phase this is.
    pub const fn phase(&self) -> GesturePhase {
        match self {
            Self::Begin { .. } => GesturePhase::Begin,
            Self::Update { .. } => GesturePhase::Update,
            Self::End { .. } => GesturePhase::End,
            Self::Cancel { .. } => GesturePhase::Cancel,
        }
    }

    /// When it happened, on the caller's clock.
    pub const fn tick(&self) -> Tick {
        match self {
            Self::Begin { tick, .. }
            | Self::Update { tick, .. }
            | Self::End { tick, .. }
            | Self::Cancel { tick, .. } => *tick,
        }
    }

    /// Swipe or pinch.
    pub const fn kind(&self) -> GestureKind {
        match self {
            Self::Begin { kind, .. }
            | Self::Update { kind, .. }
            | Self::End { kind, .. }
            | Self::Cancel { kind, .. } => *kind,
        }
    }

    /// How many contacts the gesture has.
    pub const fn fingers(&self) -> u8 {
        match self {
            Self::Begin { fingers, .. }
            | Self::Update { fingers, .. }
            | Self::End { fingers, .. }
            | Self::Cancel { fingers, .. } => *fingers,
        }
    }

    /// The edge a swipe started at.
    pub const fn edge(&self) -> Option<Edge> {
        match self {
            Self::Begin { edge, .. } | Self::Update { edge, .. } | Self::End { edge, .. } => *edge,
            Self::Cancel { .. } => None,
        }
    }
}

/// Where each edge of the touchpad is, so a swipe can be classified by where it began.
///
/// Edge swipes are not part of the published `touchpad_gestures` schema, so this is
/// opt-in: without it a swipe that starts at an edge is an ordinary swipe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBand {
    /// Touchpad width, in the units the caller reports contacts in.
    pub width: f64,
    /// Touchpad height, in the same units.
    pub height: f64,
    /// How close to an edge a contact must start to count, in the same units.
    pub band: f64,
}

impl EdgeBand {
    /// The edge a starting position is within the band of, if any.
    ///
    /// The nearest edge wins, and ties resolve left, right, top, bottom so a corner is
    /// always reported the same way.
    pub fn edge_at(&self, x: f64, y: f64) -> Option<Edge> {
        let candidates = [
            (Edge::Left, x),
            (Edge::Right, self.width - x),
            (Edge::Top, y),
            (Edge::Bottom, self.height - y),
        ];
        let mut best: Option<(Edge, f64)> = None;
        for (edge, distance) in candidates {
            if distance > self.band {
                continue;
            }
            if best.is_none_or(|(_, best_distance)| distance < best_distance) {
                best = Some((edge, distance));
            }
        }
        best.map(|(edge, _)| edge)
    }
}

/// When a gesture is recognised, in the caller's units and ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureConfig {
    /// Centroid movement that recognises a swipe. The published default is 48.
    pub swipe_threshold: f64,
    /// Change of the contact spread, relative to the starting spread, that recognises
    /// a pinch. The published default is 0.12.
    pub pinch_threshold: f64,
    /// Edge classification, or `None` for none.
    pub edge: Option<EdgeBand>,
    /// Ticks without contact progress after which a stream or gather is dropped.
    pub stale_ticks: u64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            swipe_threshold: 48.0,
            pinch_threshold: 0.12,
            edge: None,
            stale_ticks: 1000,
        }
    }
}

/// A recognised gesture in progress.
#[derive(Clone, Debug)]
struct Stream {
    kind: GestureKind,
    fingers: u8,
    edge: Option<Edge>,
    since: Tick,
    last: Tick,
    origin: (f64, f64),
    previous: (f64, f64),
    start_spread: f64,
    /// The spread at the last position the stream saw, which is the spread the end
    /// event reports: lifting the first of two fingers collapses the spread to zero,
    /// and reporting that would make every pinch look like a gather.
    last_spread: f64,
    path: Vec<(f64, f64)>,
    travel: f64,
    angles: BTreeMap<u8, f64>,
}

/// The gesture recogniser.
#[derive(Clone, Debug)]
pub struct Recogniser {
    config: GestureConfig,
    contacts: BTreeMap<u8, (f64, f64)>,
    start: BTreeMap<u8, (f64, f64)>,
    gather_since: Option<Tick>,
    last_activity: Tick,
    stream: Option<Stream>,
}

impl Recogniser {
    /// A recogniser with this configuration.
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            contacts: BTreeMap::new(),
            start: BTreeMap::new(),
            gather_since: None,
            last_activity: Tick::ZERO,
            stream: None,
        }
    }

    /// The configuration in use.
    pub const fn config(&self) -> &GestureConfig {
        &self.config
    }

    /// How many contacts are down.
    pub fn fingers(&self) -> usize {
        self.contacts.len()
    }

    /// Whether a gesture is in progress.
    pub const fn is_active(&self) -> bool {
        self.stream.is_some()
    }

    /// Feed one hardware frame of contact events and get the gesture event it produced.
    ///
    /// A frame produces at most one event, however many contacts it carries.
    pub fn feed(&mut self, tick: Tick, frame: &[Contact]) -> Option<GestureEvent> {
        if frame.is_empty() {
            return None;
        }
        self.last_activity = tick;

        if frame.iter().any(|contact| contact.phase == ContactPhase::Cancel) {
            let event = self.stream.as_ref().map(|stream| GestureEvent::Cancel {
                tick,
                kind: stream.kind,
                fingers: stream.fingers,
                reason: CancelReason::Cancelled,
            });
            self.reset();
            return event;
        }

        for contact in frame {
            match contact.phase {
                ContactPhase::Begin => {
                    if self.stream.is_some() {
                        // The finger count was fixed when the gesture began, so a
                        // contact that joins it now is not part of its movement.
                        continue;
                    }
                    self.contacts.insert(contact.slot, (contact.x, contact.y));
                    self.start.insert(contact.slot, (contact.x, contact.y));
                    self.gather_since.get_or_insert(tick);
                }
                ContactPhase::Update | ContactPhase::End => {
                    if self.contacts.contains_key(&contact.slot) {
                        self.contacts.insert(contact.slot, (contact.x, contact.y));
                    }
                }
                ContactPhase::Cancel => {}
            }
        }

        let moved = frame.iter().any(|contact| {
            matches!(contact.phase, ContactPhase::Update | ContactPhase::End)
                && self.contacts.contains_key(&contact.slot)
        });
        let movement = if moved { self.progress(tick) } else { None };

        for contact in frame {
            if contact.phase == ContactPhase::End {
                self.contacts.remove(&contact.slot);
                self.start.remove(&contact.slot);
            }
        }

        if self.stream.is_some() {
            if self.contacts.len() < usize::from(MIN_FINGERS) {
                return self.finish(tick);
            }
            let stream = self.stream.as_ref()?;
            let (kind, fingers, edge) = (stream.kind, stream.fingers, stream.edge);
            let (dx, dy, scale, angle_delta) = movement?;
            return Some(GestureEvent::Update {
                tick,
                kind,
                fingers,
                edge,
                dx,
                dy,
                scale,
                angle_delta,
            });
        }

        if self.contacts.len() > usize::from(MAX_FINGERS) {
            self.contacts.clear();
            self.start.clear();
            self.gather_since = None;
            return None;
        }
        if self.contacts.is_empty() {
            self.start.clear();
            self.gather_since = None;
        }
        self.maybe_begin(tick)
    }

    /// Retire a stream or a gather that stopped advancing.
    ///
    /// The caller runs this once per tick. A stream that stopped advancing is cancelled
    /// so a shell animation ends; a gather whose contacts went quiet is dropped so the
    /// next contact starts cleanly.
    pub fn expire(&mut self, tick: Tick) -> Option<GestureEvent> {
        if let Some(stream) = self.stream.as_ref() {
            if tick.since(stream.last) < self.config.stale_ticks {
                return None;
            }
            let (kind, fingers) = (stream.kind, stream.fingers);
            self.reset();
            return Some(GestureEvent::Cancel {
                tick,
                kind,
                fingers,
                reason: CancelReason::Stale,
            });
        }
        if self.gather_since.is_some() && tick.since(self.last_activity) >= self.config.stale_ticks {
            self.reset();
        }
        None
    }

    /// The device went away.
    ///
    /// Any gesture in progress is cancelled with [`CancelReason::DeviceRemoved`]; the
    /// caller must not expect further events from that device.
    pub fn device_removed(&mut self, tick: Tick) -> Option<GestureEvent> {
        let event = self.stream.as_ref().map(|stream| GestureEvent::Cancel {
            tick,
            kind: stream.kind,
            fingers: stream.fingers,
            reason: CancelReason::DeviceRemoved,
        });
        self.reset();
        event
    }

    /// Forget every contact and any gesture in progress.
    fn reset(&mut self) {
        self.contacts.clear();
        self.start.clear();
        self.gather_since = None;
        self.stream = None;
    }

    /// Recognise a gesture if the movement has crossed a threshold.
    fn maybe_begin(&mut self, tick: Tick) -> Option<GestureEvent> {
        let fingers = self.contacts.len();
        if !(usize::from(MIN_FINGERS)..=usize::from(MAX_FINGERS)).contains(&fingers) {
            return None;
        }
        let start = centroid(&self.start)?;
        let current = centroid(&self.contacts)?;
        let translation = distance(start, current);
        let start_spread = spread(&self.start, start);
        let current_spread = spread(&self.contacts, current);
        let scale = if start_spread > SPREAD_EPSILON {
            current_spread / start_spread
        } else {
            1.0
        };
        let pinch = (scale - 1.0).abs();
        let swipe_ratio = if self.config.swipe_threshold > 0.0 {
            translation / self.config.swipe_threshold
        } else {
            0.0
        };
        let pinch_ratio = if self.config.pinch_threshold > 0.0 {
            pinch / self.config.pinch_threshold
        } else {
            0.0
        };
        if swipe_ratio < 1.0 && pinch_ratio < 1.0 {
            return None;
        }
        let kind = if pinch_ratio > swipe_ratio {
            GestureKind::Pinch
        } else {
            GestureKind::Swipe
        };
        let edge = match (kind, self.config.edge) {
            (GestureKind::Swipe, Some(band)) => band.edge_at(start.0, start.1),
            _ => None,
        };

        let mut path = Vec::with_capacity(PATH_SAMPLES);
        path.push((0.0, 0.0));
        push_path(&mut path, (current.0 - start.0, current.1 - start.1));
        let angles = angles(&self.contacts, current);
        self.stream = Some(Stream {
            kind,
            fingers: fingers as u8,
            edge,
            since: tick,
            last: tick,
            origin: start,
            previous: current,
            start_spread,
            last_spread: current_spread,
            path,
            travel: translation,
            angles,
        });
        Some(GestureEvent::Begin {
            tick,
            kind,
            fingers: fingers as u8,
            edge,
        })
    }

    /// Fold the contacts' current positions into the stream.
    ///
    /// Returns the movement since the previous position, which the caller reports as an
    /// update or, at the end of a stream, keeps without reporting.
    fn progress(&mut self, tick: Tick) -> Option<(f64, f64, f64, f64)> {
        let stream = self.stream.as_mut()?;
        let current = centroid(&self.contacts)?;
        let dx = current.0 - stream.previous.0;
        let dy = current.1 - stream.previous.1;
        stream.travel += dx.hypot(dy);
        stream.last = tick;
        stream.previous = current;
        stream.last_spread = spread(&self.contacts, current);
        let scale = if stream.start_spread > SPREAD_EPSILON {
            stream.last_spread / stream.start_spread
        } else {
            1.0
        };
        let angle_delta = rotate(&mut stream.angles, &self.contacts, current);
        push_path(
            &mut stream.path,
            (current.0 - stream.origin.0, current.1 - stream.origin.1),
        );
        Some((dx, dy, scale, angle_delta))
    }

    /// Finish a gesture and forget every contact.
    fn finish(&mut self, tick: Tick) -> Option<GestureEvent> {
        let stream = self.stream.take()?;
        let scale = if stream.start_spread > SPREAD_EPSILON {
            stream.last_spread / stream.start_spread
        } else {
            1.0
        };
        let duration = tick.since(stream.since);
        let event = GestureEvent::End {
            tick,
            kind: stream.kind,
            fingers: stream.fingers,
            edge: stream.edge,
            path: GesturePath(stream.path),
            travel: stream.travel,
            scale,
            duration,
        };
        self.reset();
        Some(event)
    }
}

/// Whether a travelled path matches a configured one.
///
/// Both paths are normalised by their largest coordinate and resampled to
/// [`PATH_SAMPLES`] points by arc length, then compared by root-mean-square distance,
/// which is the rule the C router used: the comparison is of shape, so the same binding
/// matches gestures of different physical lengths and starting places. A path with
/// fewer than two points, or one that never moves, matches nothing.
pub fn path_matches(expected: &[(f64, f64)], actual: &[(f64, f64)], tolerance: f64) -> bool {
    let Some(expected) = resample(expected) else {
        return false;
    };
    let Some(actual) = resample(actual) else {
        return false;
    };
    let mut squared = 0.0;
    for (left, right) in expected.iter().zip(actual.iter()) {
        squared += (left.0 - right.0).powi(2) + (left.1 - right.1).powi(2);
    }
    (squared / expected.len() as f64).sqrt() <= tolerance
}

/// Normalise a path and resample it to [`PATH_SAMPLES`] points by arc length.
fn resample(path: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    if path.len() < 2 {
        return None;
    }
    let mut scale: f64 = 0.0;
    for point in path {
        scale = scale.max(point.0.abs()).max(point.1.abs());
    }
    if scale < SPREAD_EPSILON {
        return None;
    }
    let mut lengths = Vec::with_capacity(path.len());
    lengths.push(0.0);
    for index in 1..path.len() {
        let step = distance(path[index - 1], path[index]) / scale;
        lengths.push(lengths[index - 1] + step);
    }
    let total = *lengths.last()?;
    if total < SPREAD_EPSILON {
        return None;
    }
    let mut samples = Vec::with_capacity(PATH_SAMPLES);
    let mut segment = 1;
    for sample in 0..PATH_SAMPLES {
        let target = total * sample as f64 / (PATH_SAMPLES - 1) as f64;
        while segment < path.len() - 1 && lengths[segment] < target {
            segment += 1;
        }
        let start = path[segment - 1];
        let end = path[segment];
        let span = lengths[segment] - lengths[segment - 1];
        let amount = if span == 0.0 {
            0.0
        } else {
            (target - lengths[segment - 1]) / span
        };
        samples.push((
            (start.0 + (end.0 - start.0) * amount) / scale,
            (start.1 + (end.1 - start.1) * amount) / scale,
        ));
    }
    Some(samples)
}

/// The mean position of the contacts.
fn centroid(contacts: &BTreeMap<u8, (f64, f64)>) -> Option<(f64, f64)> {
    if contacts.is_empty() {
        return None;
    }
    let mut x = 0.0;
    let mut y = 0.0;
    for point in contacts.values() {
        x += point.0;
        y += point.1;
    }
    let count = contacts.len() as f64;
    Some((x / count, y / count))
}

/// The mean distance of the contacts from their centroid.
fn spread(contacts: &BTreeMap<u8, (f64, f64)>, centroid: (f64, f64)) -> f64 {
    if contacts.is_empty() {
        return 0.0;
    }
    let mut total = 0.0;
    for point in contacts.values() {
        total += distance(*point, centroid);
    }
    total / contacts.len() as f64
}

/// Each contact's angle around the centroid.
fn angles(contacts: &BTreeMap<u8, (f64, f64)>, centroid: (f64, f64)) -> BTreeMap<u8, f64> {
    contacts
        .iter()
        .map(|(slot, point)| (*slot, (point.1 - centroid.1).atan2(point.0 - centroid.0)))
        .collect()
}

/// The mean rotation since the previous call, in radians, wrapped to one turn.
fn rotate(previous: &mut BTreeMap<u8, f64>, contacts: &BTreeMap<u8, (f64, f64)>, centroid: (f64, f64)) -> f64 {
    let mut total = 0.0;
    let mut counted = 0.0;
    let mut next = BTreeMap::new();
    for (slot, point) in contacts {
        let angle = (point.1 - centroid.1).atan2(point.0 - centroid.0);
        if let Some(before) = previous.get(slot) {
            total += wrap(angle - before);
            counted += 1.0;
        }
        next.insert(*slot, angle);
    }
    *previous = next;
    if counted == 0.0 { 0.0 } else { total / counted }
}

/// Wrap an angle to the half-open range (-pi, pi].
fn wrap(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    let mut wrapped = angle % turn;
    if wrapped <= -std::f64::consts::PI {
        wrapped += turn;
    } else if wrapped > std::f64::consts::PI {
        wrapped -= turn;
    }
    wrapped
}

/// Distance between two points.
fn distance(left: (f64, f64), right: (f64, f64)) -> f64 {
    (left.0 - right.0).hypot(left.1 - right.1)
}

/// Append a point to a path, keeping the path bounded.
fn push_path(path: &mut Vec<(f64, f64)>, point: (f64, f64)) {
    let Some(last) = path.last().copied() else {
        path.push(point);
        return;
    };
    if distance(last, point) < PATH_STEP {
        return;
    }
    if path.len() < MAX_PATH_POINTS {
        path.push(point);
    } else if let Some(last) = path.last_mut() {
        *last = point;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CancelReason, Contact, ContactPhase, Edge, EdgeBand, GestureConfig, GestureEvent, GestureKind, GesturePhase,
        Recogniser, path_matches,
    };
    use crate::tick::Tick;

    fn at(slot: u8, phase: ContactPhase, x: f64, y: f64) -> Contact {
        Contact { slot, phase, x, y }
    }

    fn both(phase: ContactPhase, left: (f64, f64), right: (f64, f64)) -> [Contact; 2] {
        [at(0, phase, left.0, left.1), at(1, phase, right.0, right.1)]
    }

    /// A stationary two-finger gather on top of an existing recogniser state.
    fn gather(recogniser: &mut Recogniser, tick: u64) {
        assert!(
            recogniser
                .feed(
                    Tick::new(tick),
                    &both(ContactPhase::Begin, (100.0, 100.0), (120.0, 100.0))
                )
                .is_none()
        );
    }

    #[test]
    fn a_two_finger_swipe_begins_updates_and_ends() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        let begin = {
            gather(&mut recogniser, 1);
            recogniser
                .feed(
                    Tick::new(1),
                    &both(ContactPhase::Update, (100.0, 150.0), (120.0, 150.0)),
                )
                .expect("the centroid moved 50, past the 48 threshold")
        };
        assert_eq!(begin.phase(), GesturePhase::Begin);
        assert_eq!(begin.kind(), GestureKind::Swipe);
        assert_eq!(begin.fingers(), 2);
        assert_eq!(begin.edge(), None);

        let update = recogniser
            .feed(
                Tick::new(2),
                &both(ContactPhase::Update, (100.0, 170.0), (120.0, 170.0)),
            )
            .expect("update");
        match update {
            GestureEvent::Update {
                dx,
                dy,
                scale,
                angle_delta,
                fingers,
                ..
            } => {
                assert_eq!(fingers, 2);
                assert_eq!(dx, 0.0);
                assert!((dy - 20.0).abs() < 0.001);
                assert!(
                    (scale - 1.0).abs() < 0.001,
                    "a translating pair does not change its spread"
                );
                assert!(angle_delta.abs() < 0.001);
            }
            other => panic!("expected an update, got {other:?}"),
        }

        let end = recogniser
            .feed(Tick::new(3), &both(ContactPhase::End, (100.0, 180.0), (120.0, 180.0)))
            .expect("end");
        match end {
            GestureEvent::End {
                path,
                travel,
                duration,
                fingers,
                scale,
                ..
            } => {
                assert_eq!(fingers, 2);
                assert_eq!(duration, 2);
                assert!((travel - 80.0).abs() < 0.001, "travel was {travel}");
                assert!((scale - 1.0).abs() < 0.001);
                assert_eq!(path.points()[0], (0.0, 0.0));
                assert!(path.points().len() >= 2, "the path has the whole movement");
            }
            other => panic!("expected an end, got {other:?}"),
        }
        assert!(!recogniser.is_active());
        assert_eq!(recogniser.fingers(), 0);
    }

    #[test]
    fn one_contact_never_begins_a_gesture() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        assert!(
            recogniser
                .feed(Tick::new(1), &[at(0, ContactPhase::Begin, 100.0, 100.0)])
                .is_none()
        );
        for step in 1..10 {
            let y = 100.0 + f64::from(step) * 20.0;
            assert!(
                recogniser
                    .feed(Tick::new(2), &[at(0, ContactPhase::Update, 100.0, y)])
                    .is_none(),
                "one contact is below the two-contact floor"
            );
        }
        assert!(!recogniser.is_active());
        assert!(
            recogniser
                .feed(Tick::new(3), &[at(0, ContactPhase::End, 100.0, 300.0)])
                .is_none()
        );
        assert_eq!(recogniser.fingers(), 0);
    }

    #[test]
    fn one_finger_moving_away_from_the_other_is_a_pinch() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        gather(&mut recogniser, 1);
        let begin = recogniser
            .feed(Tick::new(2), &[at(0, ContactPhase::Update, 100.0, 160.0)])
            .expect("the spread grew, which is what one finger moving away is");
        assert_eq!(begin.kind(), GestureKind::Pinch);
    }

    #[test]
    fn a_pinch_is_recognised_by_the_spread() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        assert!(
            recogniser
                .feed(Tick::new(1), &both(ContactPhase::Begin, (100.0, 100.0), (110.0, 100.0)))
                .is_none()
        );
        let begin = recogniser
            .feed(Tick::new(2), &both(ContactPhase::Update, (70.0, 100.0), (140.0, 100.0)))
            .expect("the spread grew sevenfold");
        assert_eq!(begin.kind(), GestureKind::Pinch);
        assert_eq!(begin.fingers(), 2);
        assert_eq!(begin.edge(), None, "a pinch has no edge");
        let end = recogniser
            .feed(Tick::new(3), &both(ContactPhase::End, (70.0, 100.0), (140.0, 100.0)))
            .expect("end");
        assert!(
            matches!(end, GestureEvent::End { scale, .. } if scale > 1.0),
            "the end reports the last spread, not the collapsed one: {end:?}"
        );
    }

    #[test]
    fn a_swipe_from_an_edge_reports_that_edge() {
        let band = EdgeBand {
            width: 1000.0,
            height: 500.0,
            band: 30.0,
        };
        let config = GestureConfig {
            edge: Some(band),
            ..GestureConfig::default()
        };
        let mut recogniser = Recogniser::new(config);
        let begin = {
            assert!(
                recogniser
                    .feed(Tick::new(1), &both(ContactPhase::Begin, (990.0, 250.0), (970.0, 250.0)))
                    .is_none()
            );
            recogniser
                .feed(
                    Tick::new(2),
                    &both(ContactPhase::Update, (930.0, 250.0), (910.0, 250.0)),
                )
                .expect("begin")
        };
        assert_eq!(begin.edge(), Some(Edge::Right));
        assert_eq!(Edge::Right.as_str(), "right");
    }

    #[test]
    fn six_contacts_begin_nothing() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        let frame: Vec<Contact> = (0..6)
            .map(|slot| at(slot, ContactPhase::Begin, f64::from(slot) * 10.0, 0.0))
            .collect();
        assert!(recogniser.feed(Tick::new(1), &frame).is_none());
        assert_eq!(recogniser.fingers(), 0, "the gather is dropped");
        for _ in 0..3 {
            let frame: Vec<Contact> = (0..6).map(|slot| at(slot, ContactPhase::Update, 0.0, 100.0)).collect();
            assert!(recogniser.feed(Tick::new(2), &frame).is_none());
        }
    }

    #[test]
    fn a_contact_joining_a_gesture_does_not_change_it() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        gather(&mut recogniser, 1);
        let begin = recogniser
            .feed(
                Tick::new(1),
                &both(ContactPhase::Update, (100.0, 150.0), (120.0, 150.0)),
            )
            .expect("begin");
        assert_eq!(begin.fingers(), 2);
        let update = recogniser
            .feed(
                Tick::new(2),
                &[
                    at(2, ContactPhase::Begin, 300.0, 300.0),
                    at(0, ContactPhase::Update, 100.0, 160.0),
                    at(1, ContactPhase::Update, 120.0, 160.0),
                ],
            )
            .expect("update");
        assert_eq!(update.fingers(), 2, "the third contact is not part of the gesture");
        assert!(
            matches!(update, GestureEvent::Update { dx, .. } if dx.abs() < 0.001),
            "the third contact does not move the centroid: {update:?}"
        );
    }

    #[test]
    fn a_cancelled_stream_is_cancelled_once_and_forgets_its_contacts() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        gather(&mut recogniser, 1);
        recogniser
            .feed(
                Tick::new(1),
                &both(ContactPhase::Update, (100.0, 150.0), (120.0, 150.0)),
            )
            .expect("begin");
        let cancel = recogniser
            .feed(Tick::new(2), &[at(0, ContactPhase::Cancel, 0.0, 0.0)])
            .expect("cancel");
        assert_eq!(cancel.phase(), GesturePhase::Cancel);
        assert!(matches!(
            cancel,
            GestureEvent::Cancel {
                reason: CancelReason::Cancelled,
                ..
            }
        ));
        assert!(
            recogniser
                .feed(Tick::new(3), &[at(0, ContactPhase::Cancel, 0.0, 0.0)])
                .is_none(),
            "cancelling nothing reports nothing"
        );
        assert_eq!(recogniser.fingers(), 0);
    }

    #[test]
    fn a_stale_stream_expires_and_a_stale_gather_is_dropped() {
        let config = GestureConfig {
            stale_ticks: 10,
            ..GestureConfig::default()
        };
        let mut recogniser = Recogniser::new(config);
        gather(&mut recogniser, 1);
        recogniser
            .feed(
                Tick::new(1),
                &both(ContactPhase::Update, (100.0, 150.0), (120.0, 150.0)),
            )
            .expect("begin");
        assert!(recogniser.expire(Tick::new(2)).is_none());
        let stale = recogniser.expire(Tick::new(20)).expect("stale cancel");
        assert!(matches!(
            stale,
            GestureEvent::Cancel {
                reason: CancelReason::Stale,
                ..
            }
        ));

        let mut recogniser = Recogniser::new(config);
        gather(&mut recogniser, 1);
        assert!(recogniser.expire(Tick::new(20)).is_none());
        assert_eq!(recogniser.fingers(), 0, "the gather is dropped");
    }

    #[test]
    fn a_removed_device_cancels_the_gesture() {
        let mut recogniser = Recogniser::new(GestureConfig::default());
        gather(&mut recogniser, 1);
        recogniser
            .feed(
                Tick::new(1),
                &both(ContactPhase::Update, (100.0, 150.0), (120.0, 150.0)),
            )
            .expect("begin");
        let cancel = recogniser.device_removed(Tick::new(2)).expect("cancel");
        assert!(matches!(
            cancel,
            GestureEvent::Cancel {
                reason: CancelReason::DeviceRemoved,
                ..
            }
        ));
        assert!(recogniser.device_removed(Tick::new(3)).is_none());
    }

    #[test]
    fn a_path_matches_by_shape_not_length() {
        let right_then_down = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)];
        let same_shape_longer = [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)];
        let different = [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
        assert!(path_matches(&right_then_down, &same_shape_longer, 0.22));
        assert!(!path_matches(&right_then_down, &different, 0.22));
        assert!(!path_matches(&right_then_down, &right_then_down, -1.0));
        assert!(!path_matches(&[], &right_then_down, 0.5));
        assert!(!path_matches(&[(0.0, 0.0), (0.0, 0.0)], &right_then_down, 0.5));
        assert!(path_matches(&right_then_down, &right_then_down, 0.0));
    }
}

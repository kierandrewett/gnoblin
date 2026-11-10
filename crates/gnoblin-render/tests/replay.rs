//! Replay harness for the scene tree (design/node-core.md §7.3 and F34).
//!
//! A [`Session`] is a list of [`Frame`]s. Each frame has a monotonic clock reading
//! `now` in milliseconds and the inputs that land on it, in order: transactions,
//! applied as their actor through [`validate_and_apply`], and scripted pointer
//! events, hit-tested against the scene. [`run`] plays a session against a fresh
//! scene and a fresh [`TweenPlayer`]. After every frame it records the tree hash
//! and the outcome of each input.
//!
//! [`replay_twice`] runs a session twice and fails at the first frame whose record
//! differs between the two runs (F34). [`check`] compares one run with a recorded
//! [`Trace`] the same way. The tree hash is the shared `tree_hash` helper (every
//! node's id, parent, children in stored order and transform) extended with each
//! node's class, bounds, visibility, clip, input and effects, plus the root list.
//! It is FNV-1a over `Debug` output, so it covers every field of the transform and
//! the effects.
//!
//! Animated sets follow design/node-core.md §6.1. Each timed `set` starts a track
//! from the current value, or retargets the track in flight on the same node and
//! key. An instant `set` cancels the track on its key. The tween advances by the
//! difference between a frame's `now` and the previous frame's `now`, before that
//! frame's inputs apply. Only the transaction layer and the tween run here: the
//! harness runs no Lua.
//!
//! # File format
//!
//! A session is UTF-8 text with one directive per line. Blank lines and lines
//! starting with `#` are ignored. The first line is `gnoblin-replay 1`.
//!
//! ```text
//! gnoblin-replay 1
//! frame 0
//! txn compositor
//! new Window 9223372036854775810
//! insert 9223372036854775810 9223372036854775809 last
//! end
//! frame 16
//! pointer 50.0 50.0
//! txn compositor
//! set 9223372036854775810 opacity double:0.5 over 0.25 0.0 0.5 1.0
//! end
//! ```
//!
//! - `frame <now>`: starts a frame at `now` milliseconds. Times must not decrease.
//!   Inputs before the first frame are refused.
//! - `pointer <x> <y>`: a scripted pointer event on the current frame.
//! - `txn compositor|shell` opens a transaction. Its op lines follow, then `end`
//!   closes it. A transaction that is never closed is refused.
//!   - `new <class> <id>`
//!   - `insert <node> <parent> <index|last>`
//!   - `destroy <node>`
//!   - `call <node> <method> [<arg> ...]`, with integer arguments
//!   - `set <node> <key> <value> [over <seconds> <sample> ...]`
//!
//! Ids are decimal `u64`. A value is written `double:<f64>`, `uint:<u64>` or
//! `bool:true|false`. Floats are written with `Debug`, which round-trips exactly.
//!
//! # What the tests check
//!
//! - `same_session_replays_to_identical_frame_hashes`: a scripted session replays
//!   twice to the same records, and the hashes change as the scene does.
//! - `changing_one_op_changes_hash_from_that_frame`: changing one value moves the
//!   first differing frame to that frame, and every later hash differs.
//! - `refused_transaction_leaves_hash_sequence_unchanged`: a transaction refused
//!   on its second op changes nothing, including the track its first op would
//!   have started.
//! - `generated_600_frame_session_replays_twice`: a seeded 600-frame session of
//!   sets, inserts (reparents, and raises when a node is inserted at the end of the
//!   parent it already has), destroys and calls replays twice to the same hashes.
//! - `replay_file_round_trips`: the text format parses back to the same session,
//!   and the parsed session replays to the same records.
//! - `malformed_file_is_refused_with_its_line`: a bad directive is reported with
//!   its line number.

mod common;

use common::tree_hash;
use gnoblin_secret::SecretText;
use std::collections::HashMap;
use std::fmt::{self, Display};
use std::str::FromStr;

use gnoblin_animation::{PropertyValue, SampledCurve};
use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE};
use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, Timing, TreeApplied, TreeOp, TreeRefusal, TreeTransaction, TreeValue, ValueType,
    validate_and_apply, visual_property,
};
use gnoblin_render::{NodeSpec, Scene, TrackSpec, TweenPlayer};
use gnoblin_types::{NodeId, PointF, Rect, TrackId};

/// One replayable run: the frames, in clock order.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The frames, in order.
    pub frames: Vec<Frame>,
}

/// A frame: its clock reading and the inputs that land on it, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The compositor's monotonic clock, in milliseconds.
    pub now: u32,
    /// The inputs, applied in order after the tween advances.
    pub inputs: Vec<Input>,
}

/// One input landing on a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A transaction, applied atomically as `actor`.
    Transaction {
        /// Who sends the transaction.
        actor: Actor,
        /// The ops, in order.
        ops: Vec<TreeOp>,
    },
    /// A pointer event, hit-tested against the scene.
    Pointer {
        /// Horizontal position in logical pixels.
        x: f64,
        /// Vertical position in logical pixels.
        y: f64,
    },
}

/// What a run recorded after one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTrace {
    /// The frame's clock reading.
    pub now: u32,
    /// The tree hash after the frame's inputs applied.
    pub hash: u64,
    /// One outcome per input, in input order.
    pub outcomes: Vec<Outcome>,
}

/// The outcome of one input.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The transaction committed.
    Applied,
    /// The transaction was refused and changed nothing.
    Refused(TreeRefusal),
    /// The pointer event hit this node, or nothing.
    Hit(Option<u64>),
}

/// A whole run: one [`FrameTrace`] per frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// The records, in frame order.
    pub frames: Vec<FrameTrace>,
}

/// The first frame whose record differs between two runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Divergence {
    /// The zero-based frame index.
    pub frame: usize,
    /// The expected record, or `None` past the end of the expected run.
    pub expected: Option<FrameTrace>,
    /// The actual record, or `None` past the end of the actual run.
    pub actual: Option<FrameTrace>,
}

/// A line of a replay file that does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The one-based line number.
    pub line: usize,
    /// What is wrong with it.
    pub reason: String,
}

impl Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.reason)
    }
}

/// Play `session` from the fixed initial scene and record every frame.
#[must_use]
pub fn run(session: &Session) -> Trace {
    let mut player = Player::new();
    Trace {
        frames: session.frames.iter().map(|frame| player.frame(frame)).collect(),
    }
}

/// Run `session` twice and return the record of the first run, or the first frame
/// on which the two runs differ.
///
/// # Errors
///
/// Returns the [`Divergence`] at the first frame whose record differs between the
/// two runs.
pub fn replay_twice(session: &Session) -> Result<Trace, Divergence> {
    let first = run(session);
    let second = run(session);
    match first_divergence(&first, &second) {
        Some(divergence) => Err(divergence),
        None => Ok(first),
    }
}

/// Run `session` and compare it with `recorded`.
///
/// # Errors
///
/// Returns the [`Divergence`] at the first frame whose record differs from
/// `recorded`.
pub fn check(session: &Session, recorded: &Trace) -> Result<(), Divergence> {
    match first_divergence(recorded, &run(session)) {
        Some(divergence) => Err(divergence),
        None => Ok(()),
    }
}

/// The first frame at which `expected` and `actual` differ, if any.
#[must_use]
pub fn first_divergence(expected: &Trace, actual: &Trace) -> Option<Divergence> {
    let length = expected.frames.len().max(actual.frames.len());
    (0..length).find_map(|frame| {
        let (expected_frame, actual_frame) = (expected.frames.get(frame), actual.frames.get(frame));
        (expected_frame != actual_frame).then(|| Divergence {
            frame,
            expected: expected_frame.cloned(),
            actual: actual_frame.cloned(),
        })
    })
}

/// Serialize a session in the replay file format.
#[must_use]
pub fn to_text(session: &Session) -> String {
    let mut lines = vec!["gnoblin-replay 1".to_owned()];
    for frame in &session.frames {
        lines.push(format!("frame {}", frame.now));
        for input in &frame.inputs {
            match input {
                Input::Pointer { x, y } => lines.push(format!("pointer {x:?} {y:?}")),
                Input::Transaction { actor, ops } => {
                    lines.push(format!("txn {}", actor_name(*actor)));
                    lines.extend(ops.iter().map(op_text));
                    lines.push("end".to_owned());
                }
            }
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Parse a session written by [`to_text`].
///
/// # Errors
///
/// Returns the first line that does not parse, with its line number.
pub fn from_text(text: &str) -> Result<Session, ParseError> {
    let mut lines = text.lines().enumerate().filter_map(|(index, raw)| {
        let trimmed = raw.trim();
        (!trimmed.is_empty() && !trimmed.starts_with('#')).then_some((index + 1, trimmed))
    });
    let Some((header_number, header)) = lines.next() else {
        return Err(ParseError {
            line: 0,
            reason: "the file is empty".to_owned(),
        });
    };
    if header != "gnoblin-replay 1" {
        return Err(ParseError {
            line: header_number,
            reason: format!("expected `gnoblin-replay 1`, found `{header}`"),
        });
    }

    let mut frames: Vec<Frame> = Vec::new();
    let mut open: Option<(Actor, Vec<TreeOp>)> = None;
    let mut last_number = header_number;
    for (number, text_line) in lines {
        last_number = number;
        let mut line = Line::new(number, text_line);
        let keyword = line.word("a directive")?;
        if let Some((actor, mut ops)) = open.take() {
            match keyword {
                "end" => {
                    line.finish()?;
                    push_input(&mut frames, Input::Transaction { actor, ops }, &line)?;
                }
                "new" | "insert" | "destroy" | "call" | "set" => {
                    ops.push(parse_op(keyword, &mut line)?);
                    line.finish()?;
                    open = Some((actor, ops));
                }
                other => {
                    return Err(line.error(format!("`{other}` inside a transaction; expected an op or `end`")));
                }
            }
            continue;
        }
        match keyword {
            "frame" => {
                let now: u32 = line.number("a frame time")?;
                line.finish()?;
                if frames.last().is_some_and(|last| now < last.now) {
                    return Err(line.error("frame times must not decrease"));
                }
                frames.push(Frame {
                    now,
                    inputs: Vec::new(),
                });
            }
            "pointer" => {
                let x: f64 = line.number("the x position")?;
                let y: f64 = line.number("the y position")?;
                line.finish()?;
                push_input(&mut frames, Input::Pointer { x, y }, &line)?;
            }
            "txn" => {
                let actor = parse_actor(line.word("an actor")?)
                    .ok_or_else(|| line.error("expected `compositor` or `shell`"))?;
                line.finish()?;
                if frames.is_empty() {
                    return Err(line.error("input before the first `frame`"));
                }
                open = Some((actor, Vec::new()));
            }
            other => return Err(line.error(format!("unknown directive `{other}`"))),
        }
    }
    if open.is_some() {
        return Err(ParseError {
            line: last_number,
            reason: "a transaction is never closed with `end`".to_owned(),
        });
    }
    Ok(Session { frames })
}

fn push_input(frames: &mut [Frame], input: Input, line: &Line<'_>) -> Result<(), ParseError> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| line.error("input before the first `frame`"))?;
    frame.inputs.push(input);
    Ok(())
}

fn actor_name(actor: Actor) -> &'static str {
    match actor {
        Actor::Compositor => "compositor",
        Actor::Shell => "shell",
    }
}

fn parse_actor(word: &str) -> Option<Actor> {
    match word {
        "compositor" => Some(Actor::Compositor),
        "shell" => Some(Actor::Shell),
        _ => None,
    }
}

fn op_text(op: &TreeOp) -> String {
    match op {
        TreeOp::New { class, id } => format!("new {class} {}", id.get()),
        TreeOp::Insert { node, parent, index } => {
            let index = index.map_or_else(|| "last".to_owned(), |index| index.to_string());
            format!("insert {} {} {index}", node.get(), parent.get())
        }
        TreeOp::Destroy { node } => format!("destroy {}", node.get()),
        TreeOp::Input { node, seat, input } => format!("input {} {} {input:?}", node.get(), seat.get()),
        TreeOp::TreeCall { node, method, args, .. } => {
            let mut words = vec![format!("call {} {method}", node.get())];
            words.extend(args.iter().map(i64::to_string));
            words.join(" ")
        }
        TreeOp::Set {
            node,
            key,
            value,
            timing,
        } => {
            let mut text = format!("set {} {key} {}", node.get(), value_text(value));
            if let Some(timing) = timing {
                text.push_str(&format!(" over {:?}", timing.seconds));
                for sample in &timing.samples {
                    text.push_str(&format!(" {sample:?}"));
                }
            }
            text
        }
    }
}

fn value_text(value: &TreeValue) -> String {
    match value {
        TreeValue::Bool(flag) => format!("bool:{flag}"),
        TreeValue::Int(number) => format!("int:{number}"),
        TreeValue::Uint(number) => format!("uint:{number}"),
        TreeValue::Double(number) => format!("double:{number:?}"),
        TreeValue::Text(text) => format!("text:{text}"),
        TreeValue::Bytes(bytes) => format!("bytes:{bytes:?}"),
        TreeValue::ListDouble(items) => format!("list_double:{items:?}"),
        TreeValue::ListText(items) => format!("list_text:{items:?}"),
        TreeValue::Effects(passes) => format!("effects:{}", passes.len()),
        TreeValue::Node(node) => format!("node:{node:?}"),
    }
}

/// One line being parsed, split into words with a cursor.
struct Line<'a> {
    number: usize,
    words: Vec<&'a str>,
    at: usize,
}

impl<'a> Line<'a> {
    fn new(number: usize, text: &'a str) -> Self {
        Self {
            number,
            words: text.split_whitespace().collect(),
            at: 0,
        }
    }

    fn error(&self, reason: impl Into<String>) -> ParseError {
        ParseError {
            line: self.number,
            reason: reason.into(),
        }
    }

    fn word(&mut self, what: &str) -> Result<&'a str, ParseError> {
        let word = self
            .words
            .get(self.at)
            .copied()
            .ok_or_else(|| self.error(format!("missing {what}")))?;
        self.at += 1;
        Ok(word)
    }

    fn peek(&self) -> Option<&'a str> {
        self.words.get(self.at).copied()
    }

    fn number<T: FromStr>(&mut self, what: &str) -> Result<T, ParseError> {
        let word = self.word(what)?;
        word.parse()
            .map_err(|_| self.error(format!("{what} `{word}` is not a number")))
    }

    fn node(&mut self) -> Result<NodeId, ParseError> {
        Ok(NodeId::new(self.number("a node id")?))
    }

    /// The wire form carries the scalar kinds, with text written as one word. Byte strings,
    /// lists and node references are not written in this format.
    fn value(&mut self) -> Result<TreeValue, ParseError> {
        let word = self.word("a value")?;
        let (kind, text) = word
            .split_once(':')
            .ok_or_else(|| self.error(format!("value `{word}` needs a kind, as in `double:0.5`")))?;
        let parsed = match kind {
            "bool" => text.parse().map(TreeValue::Bool).ok(),
            "int" => text.parse().map(TreeValue::Int).ok(),
            "uint" => text.parse().map(TreeValue::Uint).ok(),
            "double" => text.parse().map(TreeValue::Double).ok(),
            "text" => Some(TreeValue::Text(text.to_owned())),
            _ => None,
        };
        parsed.ok_or_else(|| self.error(format!("value `{word}` is not a valid bool, int, uint, double or text")))
    }

    fn finish(&self) -> Result<(), ParseError> {
        match self.words.get(self.at) {
            None => Ok(()),
            Some(extra) => Err(self.error(format!("unexpected `{extra}`"))),
        }
    }
}

fn parse_op(keyword: &str, line: &mut Line<'_>) -> Result<TreeOp, ParseError> {
    match keyword {
        "new" => Ok(TreeOp::New {
            class: line.word("a class")?.to_owned(),
            id: line.node()?,
        }),
        "insert" => {
            let node = line.node()?;
            let parent = line.node()?;
            let index = match line.word("an index or `last`")? {
                "last" => None,
                other => Some(
                    other
                        .parse()
                        .map_err(|_| line.error(format!("index `{other}` is not `last` or a number")))?,
                ),
            };
            Ok(TreeOp::Insert { node, parent, index })
        }
        "destroy" => Ok(TreeOp::Destroy { node: line.node()? }),
        "call" => {
            let node = line.node()?;
            let method = line.word("a method")?.to_owned();
            let mut args = Vec::new();
            while line.peek().is_some() {
                args.push(line.number("an argument")?);
            }
            Ok(TreeOp::TreeCall {
                node,
                method,
                args,
                text: SecretText::default(),
            })
        }
        "set" => {
            let node = line.node()?;
            let key = line.word("a key")?.to_owned();
            let value = line.value()?;
            let timing = if line.peek() == Some("over") {
                line.word("over")?;
                let seconds = line.number("a duration in seconds")?;
                let mut samples = Vec::new();
                while line.peek().is_some() {
                    samples.push(line.number("a sample")?);
                }
                Some(Timing { seconds, samples })
            } else {
                None
            };
            Ok(TreeOp::Set {
                node,
                key,
                value,
                timing,
            })
        }
        other => Err(line.error(format!("unknown op `{other}`"))),
    }
}

// ---------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------

/// The schema the harness applies sessions against: the same window class as the
/// transaction tests.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        class == "Window"
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        if class != "Window" {
            return None;
        }
        let (kind, readonly, animated) = match key {
            "opacity" | "x" | "y" | "z" => (ValueType::Double, false, true),
            "width" | "height" => (ValueType::Double, false, false),
            "visible" | "clip" | "input" => (ValueType::Bool, false, false),
            "id" => (ValueType::Uint, true, false),
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly,
            animated,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        class == "Window" && method == "focus"
    }
}

const ROOT: u64 = COMPOSITOR_ID_BASE | 1;

/// The scene every session starts from: one compositor-owned root over a 1920x1080
/// output.
fn initial_scene() -> Scene {
    let mut scene = Scene::new();
    let root = NodeId::new(ROOT);
    scene
        .insert(root, NodeSpec::new(Rect::new(0, 0, 1920, 1080)))
        .expect("the root inserts into an empty scene");
    scene
        .set_class(root, Some("Window".into()))
        .expect("the root takes a class");
    scene.set_root(Some(root)).expect("the root is the scene root");
    scene
}

struct Player {
    scene: Scene,
    tween: TweenPlayer,
    /// The live track on each node and key, as the compositor would hold it.
    tracks: HashMap<(NodeId, String), TrackId>,
    last_now: Option<u32>,
}

impl Player {
    fn new() -> Self {
        Self {
            scene: initial_scene(),
            tween: TweenPlayer::new(),
            tracks: HashMap::new(),
            last_now: None,
        }
    }

    fn frame(&mut self, frame: &Frame) -> FrameTrace {
        let elapsed = self.last_now.map_or(0, |last| frame.now.saturating_sub(last));
        self.last_now = Some(frame.now);
        self.tween.advance(&mut self.scene, elapsed);
        self.reap();
        let outcomes = frame.inputs.iter().map(|input| self.input(input)).collect();
        FrameTrace {
            now: frame.now,
            hash: scene_hash(&self.scene),
            outcomes,
        }
    }

    fn input(&mut self, input: &Input) -> Outcome {
        match input {
            Input::Pointer { x, y } => {
                Outcome::Hit(self.scene.hit_test(PointF { x: *x, y: *y }).map(|hit| hit.node.get()))
            }
            Input::Transaction { actor, ops } => self.transaction(*actor, ops),
        }
    }

    fn transaction(&mut self, actor: Actor, ops: &[TreeOp]) -> Outcome {
        let transaction = TreeTransaction::new(ops.to_vec());
        match validate_and_apply(&mut self.scene, &Table, actor, &transaction) {
            Ok(applied) => {
                self.drive_tracks(ops, &applied);
                Outcome::Applied
            }
            Err(refusal) => Outcome::Refused(refusal),
        }
    }

    /// Start, retarget or cancel the tracks an applied transaction asks for. Only a
    /// committed transaction reaches here, so a refused one starts nothing.
    fn drive_tracks(&mut self, ops: &[TreeOp], applied: &TreeApplied) {
        let mut animations = applied.animations.iter();
        for op in ops {
            let TreeOp::Set { node, key, timing, .. } = op else {
                continue;
            };
            if timing.is_none() {
                if let Some(id) = self.tracks.remove(&(*node, key.clone())) {
                    self.tween
                        .cancel(id)
                        .expect("a track in the map is live until it is reaped");
                }
                continue;
            }
            let animation = animations
                .next()
                .expect("validate_and_apply hands over one animation per timed set");
            self.start_track(animation);
        }
        self.reap();
    }

    fn start_track(&mut self, animation: &gnoblin_render::transaction::Animation) {
        let property = visual_property(&animation.key).expect("a validated animated key names a visual property");
        let TreeValue::Double(target) = animation.value else {
            panic!("an animated key carries a double value");
        };
        #[allow(clippy::cast_possible_truncation)]
        let to = PropertyValue::number(target as f32);
        let curve = SampledCurve::new(animation.samples.clone()).expect("validated samples build a curve");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let duration_ms = (animation.seconds * 1000.0).round() as u32;
        let key = (animation.node, animation.key.clone());
        let id = match self.tracks.remove(&key) {
            Some(current) => self
                .tween
                .retarget_sampled(&mut self.scene, current, to, duration_ms, curve),
            None => {
                let mut spec = TrackSpec::new(animation.node, property, to);
                spec.duration_ms = duration_ms;
                self.tween.start_sampled(&mut self.scene, spec, curve)
            }
        }
        .expect("a validated animation starts a track");
        self.tracks.insert(key, id);
    }

    /// Forget every track the tween has ended, so the map only holds live tracks.
    fn reap(&mut self) {
        for end in self.tween.take_ended() {
            self.tracks.retain(|_, id| *id != end.track);
        }
    }
}

/// FNV-1a over a stream of `Debug` fields.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn field(&mut self, value: &impl fmt::Debug) {
        for byte in format!("{value:?};").as_bytes() {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// The tree hash after a frame: the shared [`tree_hash`] (ids, parents, children
/// and transforms), then every node's bounds, class, visibility, clip, input,
/// effects and the root list, which that hash does not cover.
fn scene_hash(scene: &Scene) -> u64 {
    let mut hash = Fnv::new();
    hash.field(&tree_hash(scene));
    let mut ids: Vec<NodeId> = scene.node_ids().collect();
    ids.sort();
    for node in ids {
        hash.field(&node.get());
        hash.field(&scene.class(node));
        hash.field(&scene.bounds(node));
        hash.field(&scene.visible(node));
        hash.field(&scene.clip(node));
        hash.field(&scene.input(node));
        hash.field(&scene.effects(node));
    }
    hash.field(&scene.root_ids());
    hash.0
}

// ---------------------------------------------------------------------------
// Generated sessions
// ---------------------------------------------------------------------------

/// A seeded xorshift64 generator: the same seed always makes the same session.
struct XorShift(u64);

impl XorShift {
    fn step(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    /// A value in `0..bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.step() % bound
    }

    /// True with `percent` in a hundred odds.
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const DOUBLE_KEYS: [&str; 4] = ["opacity", "x", "y", "z"];
const BOOL_KEYS: [&str; 3] = ["visible", "clip", "input"];
const SIZE_KEYS: [&str; 2] = ["width", "height"];
const SECONDS: [f64; 4] = [0.1, 0.25, 0.5, 1.0];
const CURVES: [&[f64]; 2] = [&[0.0, 0.5, 1.0], &[0.0, 0.25, 0.75, 1.0]];

/// Builds random compositor transactions over the windows it has created.
struct Generator {
    rng: XorShift,
    /// Every window created and not destroyed, in creation order.
    windows: Vec<NodeId>,
    next_id: u64,
}

impl Generator {
    fn pick(&mut self) -> Option<NodeId> {
        if self.windows.is_empty() {
            return None;
        }
        let index = self.rng.below(self.windows.len() as u64) as usize;
        Some(self.windows[index])
    }

    fn new_window(&mut self) -> TreeOp {
        let id = NodeId::new(self.next_id);
        self.next_id += 1;
        self.windows.push(id);
        TreeOp::New {
            class: "Window".into(),
            id,
        }
    }

    fn transaction(&mut self) -> Vec<TreeOp> {
        let count = 1 + self.rng.below(4);
        (0..count).map(|_| self.op()).collect()
    }

    fn op(&mut self) -> TreeOp {
        let roll = self.rng.below(100);
        let Some(node) = self.pick() else {
            return self.new_window();
        };
        match roll {
            0..=14 => self.new_window(),
            15..=29 => {
                let parent = if self.rng.chance(20) {
                    NodeId::new(ROOT)
                } else {
                    self.pick().unwrap_or(node)
                };
                let index = if self.rng.chance(70) {
                    None
                } else {
                    Some(self.rng.below(3) as usize)
                };
                TreeOp::Insert { node, parent, index }
            }
            30..=34 => {
                self.windows.retain(|window| *window != node);
                TreeOp::Destroy { node }
            }
            35..=59 => {
                let key = DOUBLE_KEYS[self.rng.below(DOUBLE_KEYS.len() as u64) as usize];
                let value = if key == "opacity" {
                    self.rng.below(1001) as f64 / 1000.0
                } else {
                    self.rng.below(801) as f64 - 400.0
                };
                let timing = self.rng.chance(50).then(|| Timing {
                    seconds: SECONDS[self.rng.below(SECONDS.len() as u64) as usize],
                    samples: CURVES[self.rng.below(CURVES.len() as u64) as usize].to_vec(),
                });
                TreeOp::Set {
                    node,
                    key: key.into(),
                    value: TreeValue::Double(value),
                    timing,
                }
            }
            60..=74 => {
                let key = SIZE_KEYS[self.rng.below(SIZE_KEYS.len() as u64) as usize];
                TreeOp::Set {
                    node,
                    key: key.into(),
                    value: TreeValue::Double((self.rng.below(1500) + 1) as f64),
                    timing: None,
                }
            }
            75..=84 => {
                let key = BOOL_KEYS[self.rng.below(BOOL_KEYS.len() as u64) as usize];
                TreeOp::Set {
                    node,
                    key: key.into(),
                    value: TreeValue::Bool(self.rng.chance(50)),
                    timing: None,
                }
            }
            85..=94 => TreeOp::Insert {
                node,
                parent: NodeId::new(ROOT),
                index: None,
            },
            _ => TreeOp::TreeCall {
                node,
                method: "focus".into(),
                args: Vec::new(),
                text: SecretText::default(),
            },
        }
    }
}

/// A seeded session of `frames` frames of compositor transactions and pointer
/// events. Clock steps are 1 to 40 ms.
fn generated_session(seed: u64, frames: usize) -> Session {
    let mut generator = Generator {
        rng: XorShift(seed),
        windows: Vec::new(),
        next_id: COMPOSITOR_ID_BASE | 2,
    };
    let mut now = 0u32;
    let frames = (0..frames)
        .map(|_| {
            now += 1 + generator.rng.below(40) as u32;
            let mut inputs = Vec::new();
            if generator.rng.chance(15) {
                inputs.push(Input::Pointer {
                    x: generator.rng.below(1920) as f64,
                    y: generator.rng.below(1080) as f64,
                });
            }
            if generator.rng.chance(85) {
                inputs.push(Input::Transaction {
                    actor: Actor::Compositor,
                    ops: generator.transaction(),
                });
            }
            Frame { now, inputs }
        })
        .collect();
    Session { frames }
}

const GENERATED_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

// ---------------------------------------------------------------------------
// Scripted sessions
// ---------------------------------------------------------------------------

const A: u64 = COMPOSITOR_ID_BASE | 2;
const B: u64 = COMPOSITOR_ID_BASE | 3;

fn compositor(ops: Vec<TreeOp>) -> Input {
    Input::Transaction {
        actor: Actor::Compositor,
        ops,
    }
}

fn set(node: u64, key: &str, value: TreeValue, timing: Option<Timing>) -> TreeOp {
    TreeOp::Set {
        node: NodeId::new(node),
        key: key.into(),
        value,
        timing,
    }
}

fn timed(seconds: f64, samples: &[f64]) -> Option<Timing> {
    Some(Timing {
        seconds,
        samples: samples.to_vec(),
    })
}

/// Six frames: two windows, an animated move, an instant opacity write (the op a
/// changed-value test edits), a retarget in flight, a reparent, a raise and a hide.
fn script(opacity: f64) -> Session {
    let root = NodeId::new(ROOT);
    let frame = |now: u32, inputs: Vec<Input>| Frame { now, inputs };
    Session {
        frames: vec![
            frame(
                0,
                vec![compositor(vec![
                    TreeOp::New {
                        class: "Window".into(),
                        id: NodeId::new(A),
                    },
                    TreeOp::Insert {
                        node: NodeId::new(A),
                        parent: root,
                        index: None,
                    },
                    TreeOp::New {
                        class: "Window".into(),
                        id: NodeId::new(B),
                    },
                    TreeOp::Insert {
                        node: NodeId::new(B),
                        parent: root,
                        index: None,
                    },
                    set(A, "width", TreeValue::Double(300.0), None),
                    set(A, "height", TreeValue::Double(200.0), None),
                ])],
            ),
            frame(
                16,
                vec![
                    Input::Pointer { x: 50.0, y: 50.0 },
                    compositor(vec![set(B, "x", TreeValue::Double(10.0), timed(0.5, &[0.0, 0.5, 1.0]))]),
                ],
            ),
            frame(
                32,
                vec![compositor(vec![set(A, "opacity", TreeValue::Double(opacity), None)])],
            ),
            frame(
                48,
                vec![compositor(vec![set(
                    B,
                    "x",
                    TreeValue::Double(20.0),
                    timed(0.25, &[0.0, 1.0]),
                )])],
            ),
            frame(
                400,
                vec![
                    Input::Pointer { x: 1000.0, y: 500.0 },
                    compositor(vec![TreeOp::Insert {
                        node: NodeId::new(A),
                        parent: root,
                        index: None,
                    }]),
                ],
            ),
            frame(
                700,
                vec![compositor(vec![
                    TreeOp::Insert {
                        node: NodeId::new(B),
                        parent: NodeId::new(A),
                        index: None,
                    },
                    set(B, "visible", TreeValue::Bool(false), None),
                ])],
            ),
        ],
    }
}

/// The op at index 1 of frame 3 is refused: its own animated write must not start a
/// track, and the frame must hash as if it had never been sent.
fn refused_frame_session() -> Session {
    let mut session = script(0.25);
    session.frames[3].inputs.push(compositor(vec![
        set(B, "x", TreeValue::Double(-300.0), timed(0.5, &[0.0, 1.0])),
        set(A, "width", TreeValue::Bool(true), None),
    ]));
    session
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn assert_replays(session: &Session) -> Trace {
    match replay_twice(session) {
        Ok(trace) => trace,
        Err(divergence) => panic!("the session diverges between two runs: {divergence:?}"),
    }
}

fn hashes(trace: &Trace) -> Vec<u64> {
    trace.frames.iter().map(|frame| frame.hash).collect()
}

#[test]
fn same_session_replays_to_identical_frame_hashes() {
    let session = script(0.25);
    let trace = assert_replays(&session);
    assert_eq!(trace.frames.len(), session.frames.len(), "one record per frame");
    check(&session, &trace).expect("a run matches its own recording");
    assert!(
        trace.frames.windows(2).any(|pair| pair[0].hash != pair[1].hash),
        "the scripted session must change the tree, or the hashes prove nothing"
    );
    assert_eq!(
        trace.frames[1].outcomes,
        vec![Outcome::Hit(Some(A)), Outcome::Applied],
        "the pointer hits the window it lands on, and the transaction commits"
    );
    assert_eq!(
        trace.frames[4].outcomes.last(),
        Some(&Outcome::Applied),
        "the raise at frame 4 commits"
    );
}

#[test]
fn changing_one_op_changes_hash_from_that_frame() {
    let base = run(&script(0.25));
    let changed = run(&script(0.5));
    let divergence = first_divergence(&base, &changed).expect("the changed op must diverge");
    assert_eq!(
        divergence.frame, 2,
        "the first differing frame is the one with the changed op"
    );
    for frame in 2..base.frames.len() {
        assert_ne!(
            base.frames[frame].hash, changed.frames[frame].hash,
            "frame {frame} must hash differently once the op changed"
        );
    }
    assert_eq!(
        base.frames[1].hash, changed.frames[1].hash,
        "frames before the change hash the same"
    );
}

#[test]
fn refused_transaction_leaves_hash_sequence_unchanged() {
    let plain = run(&script(0.25));
    let with_refusal = run(&refused_frame_session());
    assert_eq!(
        hashes(&with_refusal),
        hashes(&plain),
        "a refused transaction must leave every frame's hash as it was"
    );
    assert_eq!(
        with_refusal.frames[3].outcomes.last(),
        Some(&Outcome::Refused(TreeRefusal {
            failed_op: 1,
            code: Code::WrongType,
        })),
        "the second op is refused for its type"
    );
    assert_replays(&refused_frame_session());
}

#[test]
fn generated_600_frame_session_replays_twice() {
    let session = generated_session(GENERATED_SEED, 600);
    assert_eq!(session.frames.len(), 600);
    let trace = assert_replays(&session);
    let outcomes: Vec<&Outcome> = trace.frames.iter().flat_map(|frame| frame.outcomes.iter()).collect();
    assert!(
        outcomes.iter().any(|outcome| **outcome == Outcome::Applied),
        "the session must commit transactions"
    );
    assert!(
        outcomes.iter().any(|outcome| matches!(outcome, Outcome::Refused(_))),
        "the session must include refusals, so the replay covers them"
    );
    check(&session, &trace).expect("a second run matches the recording");
}

#[test]
fn replay_file_round_trips() {
    for session in [script(0.25), generated_session(GENERATED_SEED, 600)] {
        let text = to_text(&session);
        let parsed = from_text(&text).unwrap_or_else(|error| panic!("the written file parses: {error}"));
        assert_eq!(parsed, session, "parsing the written file gives back the session");
        assert_eq!(to_text(&parsed), text, "writing the parsed session gives back the file");
        let recorded = assert_replays(&session);
        check(&parsed, &recorded).expect("the parsed session replays to the recording");
    }
}

#[test]
fn malformed_file_is_refused_with_its_line() {
    let error = from_text("gnoblin-replay 1\nframe 0\nfrme 16\n").expect_err("a misspelt directive is refused");
    assert_eq!(error.line, 3);
    let error = from_text("gnoblin-replay 1\nframe 0\ntxn compositor\nset 1 opacity double:x\nend\n")
        .expect_err("a value that is not a number is refused");
    assert_eq!(error.line, 4);
}

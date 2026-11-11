//! Failure-mode tests for the PipeWire export. Each test runs a private `pipewire` daemon. A
//! second client of that daemon lists the nodes it holds, as `pw-dump` would, so a node the
//! export should not have created, or should have removed, is seen exactly as a user would see it.
//!
//! Every test grants or refuses through [`Table::open`], the same call the portal consent flow
//! makes, never by reaching into the table.

use std::time::Duration;

use gnoblin_capture::{Copied, Format, PixelFormat, SessionId, Stop, Table, Target};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, NodeId, OutputId, Rect, Size, Tick};

use super::testing::{Consumer, Daemon, Observer};
use super::{ExportError, PipewireExport, PlaneSource};

const SIZE: Size = Size::new(64, 48);
/// The colour the headless window paints, as red, green and blue.
const WINDOW_COLOUR: [u8; 3] = [0xc0, 0x80, 0x40];
/// The same colour as the BGRx bytes of one stream pixel.
const WINDOW_PIXEL: [u8; 4] = [0x40, 0x80, 0xc0, 0x00];

/// A source with nothing to copy. The lifecycle tests never reach a frame.
struct Nothing;

impl PlaneSource for Nothing {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(SIZE))
    }

    fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, _plane: &mut [u8]) -> Copied {
        Copied::Nothing
    }
}

/// A source that paints every pixel the window colour, standing in for a rendered output.
struct SolidSource;

impl PlaneSource for SolidSource {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(SIZE))
    }

    fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, plane: &mut [u8]) -> Copied {
        for pixel in plane.as_chunks_mut::<4>().0 {
            *pixel = WINDOW_PIXEL;
        }
        Copied::Copied
    }
}

fn format() -> Format {
    Format::new(SIZE, SIZE.width * 4, PixelFormat::Xrgb8888).expect("a valid format")
}

fn output() -> Target {
    Target::Output(OutputId::new(1))
}

fn owner() -> ClientId {
    ClientId::new(1)
}

fn intruder() -> ClientId {
    ClientId::new(2)
}

/// The `node.name` the export gives the stream of `session`.
fn stream_name(session: SessionId) -> String {
    format!("gnoblin-capture-{}", session.get())
}

#[test]
fn ungranted_session_is_refused_and_no_node_appears() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Requested, format());
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::NotGranted));
    assert_eq!(export.node_id(&table, session, owner()), Err(ExportError::NotExported));
    assert!(
        export.connection.is_none(),
        "a refused session must not connect to PipeWire"
    );
    observer.settle();
    assert_eq!(
        observer.node(&stream_name(session)),
        None,
        "a refused session must not create a node"
    );
}

#[test]
fn granted_session_gets_the_node_the_observer_sees() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());

    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    assert_eq!(export.node_id(&table, session, owner()), Ok(node));
    observer.wait_for(node);
}

#[test]
fn unknown_and_closed_sessions_are_refused() {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    assert!(table.close(session).is_some());
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::Unknown));
    assert_eq!(
        export.start(&table, SessionId::new(42), owner()),
        Err(ExportError::Unknown)
    );
    assert!(
        export.connection.is_none(),
        "a refused session must not connect to PipeWire"
    );
}

#[test]
fn stopped_session_is_refused() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    assert!(table.stop(session, Stop::TargetGone));
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::Stopped));
    assert!(
        export.connection.is_none(),
        "a stopped session must not connect to PipeWire"
    );
    observer.settle();
    assert_eq!(
        observer.node(&stream_name(session)),
        None,
        "a stopped session must not create a node"
    );
}

#[test]
fn only_the_owner_reads_the_node_id_and_starts_the_stream() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert_eq!(export.node_id(&table, session, intruder()), Err(ExportError::NotOwner));
    assert_eq!(export.start(&table, session, intruder()), Err(ExportError::NotOwner));
    assert_eq!(export.node_id(&table, session, owner()), Ok(node));
}

#[test]
fn a_second_caller_gets_its_own_node_and_never_another_callers() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let first = table.open(owner(), output(), CaptureStream::Granted, format());
    let second = table.open(intruder(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let first_node = export
        .start(&table, first, owner())
        .expect("the owner starts its stream");

    assert_eq!(export.node_id(&table, first, intruder()), Err(ExportError::NotOwner));
    let second_node = export
        .start(&table, second, intruder())
        .expect("the second caller starts its own stream");
    assert_ne!(first_node, second_node, "each caller's stream is a separate node");
    assert_eq!(export.node_id(&table, second, intruder()), Ok(second_node));
    assert_eq!(export.node_id(&table, second, owner()), Err(ExportError::NotOwner));
    assert_eq!(export.node_id(&table, first, owner()), Ok(first_node));
    observer.wait_for(second_node);
}

#[test]
fn owner_disconnect_tears_the_stream_down() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert_eq!(table.close_owner(owner()), 1);
    export.pump(&mut table, Tick::new(1), Capture::Desktop, &mut Nothing);

    assert!(!export.is_live(), "the stream ends with its owner");
    assert!(export.connection.is_none(), "no connection outlives the last stream");
    assert!(
        observer.wait_gone(&stream_name(session)),
        "the node leaves the registry"
    );
    assert_eq!(export.node_id(&table, session, owner()), Err(ExportError::Unknown));
}

/// The table has no revoke call: a grant is fixed when the session opens. Withdrawing consent
/// therefore reaches the export the only way the table allows, by stopping the session (here
/// the lock embargo), and the export must end the stream on that stop.
#[test]
fn revoked_grant_tears_the_stream_down() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert!(table.stop(session, Stop::Embargoed));
    export.pump(&mut table, Tick::new(1), Capture::Desktop, &mut Nothing);

    assert!(!export.is_live(), "a stopped session's stream ends");
    assert!(
        observer.wait_gone(&stream_name(session)),
        "the node leaves the registry"
    );
    assert_eq!(
        export.start(&table, session, owner()),
        Err(ExportError::Stopped),
        "a stopped session cannot be restarted"
    );
}

/// Drives `export` once per turn of a wait, on a tick of its own.
fn pump_of<'a>(
    export: &'a mut PipewireExport,
    table: &'a mut Table,
    source: &'a mut dyn PlaneSource,
) -> impl FnMut() + 'a {
    let mut tick = 0;
    move || {
        tick += 1;
        export.pump(table, Tick::new(tick), Capture::Desktop, source);
    }
}

/// A consumer of the export's node receives frames of the window colour, not empty or stale buffers.
#[test]
fn consumer_receives_frames_of_the_window_colour() {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    let consumer = Consumer::connect(&daemon.socket(), node);

    let frame = consumer.wait_for_frame_pumping(
        "of the window colour",
        pump_of(&mut export, &mut table, &mut SolidSource),
        |frame| frame.is_solid(WINDOW_COLOUR),
    );
    assert_eq!((frame.width, frame.height), (SIZE.width, SIZE.height));
    assert!(
        consumer.frames().iter().all(|frame| frame.is_solid(WINDOW_COLOUR)),
        "every frame is the window colour"
    );
}

/// A source whose target is drawn at a size the test changes, painting every pixel one colour.
struct Resizable {
    size: Size,
    colour: [u8; 4],
}

impl PlaneSource for Resizable {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(self.size))
    }

    fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, plane: &mut [u8]) -> Copied {
        for pixel in plane.as_chunks_mut::<4>().0 {
            *pixel = self.colour;
        }
        Copied::Copied
    }
}

/// A stream of a View is the View at the size it is drawn at: when the View grows and shrinks, the consumer is
/// renegotiated to the new size and receives whole pictures of it, not a crop of the size the stream started at.
#[test]
fn a_view_stream_follows_the_size_the_view_is_drawn_at() {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let view = Target::View(NodeId::new(5));
    let session = table.open(owner(), view, CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    let consumer = Consumer::connect(&daemon.socket(), node);
    let mut source = Resizable {
        size: SIZE,
        colour: WINDOW_PIXEL,
    };

    consumer.wait_for_frame_pumping(
        "of the View at its first size",
        pump_of(&mut export, &mut table, &mut source),
        |frame| frame.is_solid(WINDOW_COLOUR) && (frame.width, frame.height) == (SIZE.width, SIZE.height),
    );
    for grown in [Size::new(96, 80), Size::new(40, 24)] {
        source.size = grown;
        let frame = consumer.wait_for_frame_pumping(
            "of the View at its new size",
            pump_of(&mut export, &mut table, &mut source),
            |frame| frame.is_solid(WINDOW_COLOUR) && (frame.width, frame.height) == (grown.width, grown.height),
        );
        assert!(frame.is_whole(), "the picture is exactly the View's size");
        assert_eq!(
            table.get(session).map(|entry| entry.format().size()),
            Some(grown),
            "the session's shape followed the View"
        );
    }
}

/// The size of an output is the output's, not the stream's to follow: a stream of one keeps the shape it was
/// negotiated in, and the picture is cut or padded to it.
#[test]
fn an_output_stream_keeps_the_size_it_started_with() {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    let consumer = Consumer::connect(&daemon.socket(), node);
    let mut source = Resizable {
        size: Size::new(96, 80),
        colour: WINDOW_PIXEL,
    };
    consumer.wait_for_frame_pumping(
        "of the output at the size the stream started with",
        pump_of(&mut export, &mut table, &mut source),
        |frame| frame.is_solid(WINDOW_COLOUR) && (frame.width, frame.height) == (SIZE.width, SIZE.height),
    );
    consumer.listen_pumping(
        Duration::from_millis(500),
        pump_of(&mut export, &mut table, &mut source),
    );
    assert!(
        consumer
            .frames()
            .iter()
            .all(|frame| (frame.width, frame.height) == (SIZE.width, SIZE.height)),
        "no frame of another size arrived"
    );
}

/// The lock ends the stream of a View whether or not anyone reads it: the node leaves the daemon without a frame
/// ever being asked for, and a stream of an output the lock scene may show goes on.
#[test]
fn the_lock_ends_a_view_stream_nobody_reads_and_leaves_an_output_stream_alone() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let view = table.open(owner(), Target::View(NodeId::new(5)), CaptureStream::Granted, format());
    let screen = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let view_node = export.start(&table, view, owner()).expect("a View stream starts");
    let screen_node = export.start(&table, screen, owner()).expect("an output stream starts");
    observer.wait_for(view_node);
    observer.wait_for(screen_node);

    // Several turns while the lock scene is up: a stream the lock refuses is stopped on the first.
    for tick in 1..=5 {
        export.pump(&mut table, Tick::new(tick), Capture::LockScene, &mut Nothing);
    }
    assert!(
        observer.wait_gone(&stream_name(view)),
        "the lock removed the View's node although nobody read it"
    );
    assert_eq!(
        table.get(view).and_then(|entry| entry.stopped()),
        Some(Stop::Embargoed),
        "the View's session is stopped for the lock"
    );
    assert_eq!(
        observer.node(&stream_name(screen)),
        Some(screen_node),
        "an output the lock scene may show keeps its stream"
    );
    assert_eq!(
        export.start(&table, view, owner()),
        Err(ExportError::Stopped),
        "a View stream does not come back when the lock is released"
    );
}

//! Failure-mode tests for the PipeWire export. Each test runs a private `pipewire` daemon. A
//! second client of that daemon lists the nodes it holds, as `pw-dump` would, so a node the
//! export should not have created, or should have removed, is seen exactly as a user would see it.
//!
//! Every test grants or refuses through [`Table::open`], the same call the portal consent flow
//! makes, never by reaching into the table.

use std::time::{Duration, Instant};

use gnoblin_capture::{Copied, Format, PixelFormat, SessionId, Stop, Table, Target};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{ClientId, NodeId, OutputId, Rect, Size, Tick, WindowId};

use super::testing::{Daemon, FrameConsumer, Observer};
use super::{ExportError, PipewireExport, PlaneSource, write_region};

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

/// Scrubbing zeroes the pixels of every pool buffer the stream holds, and the stream lives on: the producer's own
/// mapping of each buffer is the one the scrub writes, so a buffer a consumer later takes carries no picture.
#[test]
fn scrub_zeroes_every_pool_buffer_and_the_stream_lives_on() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    observer.wait_for(node);
    let consumer = FrameConsumer::connect(&daemon.socket(), node);
    consumer.wait_for_frame_pumping(
        "of the window colour",
        pump_of(&mut export, &mut table, &mut SolidSource),
        |frame| frame.is_solid(WINDOW_COLOUR),
    );

    let before = export.exports[&session].known_buffers.borrow().planes();
    assert!(
        before.iter().any(|plane| plane.iter().any(|byte| *byte != 0)),
        "the pool holds the window's pixels before the scrub"
    );

    export.scrub();

    assert!(export.is_live(), "a stream whose buffers were zeroed lives on");
    let after = export.exports[&session].known_buffers.borrow().planes();
    assert!(!after.is_empty(), "the pool still holds its buffers");
    assert!(
        after.iter().all(|plane| plane.iter().all(|byte| *byte == 0)),
        "every pool buffer is zeroed"
    );
}

/// Starts a granted stream, puts `buffer` in its pool as if PipeWire had added it, and scrubs. The scrub must end the
/// stream: a buffer it cannot zero is one a consumer could still take the picture from.
fn scrub_ends_a_stream_holding(buffer: *mut pipewire::sys::pw_buffer) {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    observer.wait_for(node);

    export.exports[&session].known_buffers.borrow_mut().add(buffer);
    export.scrub();

    assert!(!export.is_live(), "a stream that cannot zero its buffers ends");
    assert!(export.connection.is_none(), "no connection outlives the last stream");
    assert!(
        observer.wait_gone(&stream_name(session)),
        "the node leaves the registry"
    );
    assert_eq!(
        export.node_id(&table, session, owner()),
        Err(ExportError::NotExported),
        "an ended stream has no node"
    );
}

/// A null buffer pointer cannot be zeroed, so the stream ends.
#[test]
fn scrub_ends_the_stream_when_a_buffer_pointer_is_null() {
    scrub_ends_a_stream_holding(std::ptr::null_mut());
}

/// A buffer with no mapping cannot be zeroed, so the stream ends. The buffer is test memory, built with a plane the
/// producer never mapped, and it outlives the scrub.
#[test]
fn scrub_ends_the_stream_when_a_buffer_has_no_mapping() {
    let mut unmapped = crate::pool::Fake::boxed(8, false);
    scrub_ends_a_stream_holding(unmapped.pointer());
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
    let consumer = FrameConsumer::connect(&daemon.socket(), node);

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

/// The longest one `pump` may take while its consumer reads nothing. A pump is one non-blocking turn of the
/// connection plus at most one frame copy, a few milliseconds; the bound is far above that so a loaded machine does
/// not trip it, and far below a blocked pump, which is as long as the stall.
const PUMP_BOUND: Duration = Duration::from_millis(250);

/// A consumer that has stopped reading does not hold the producer: every pump (the call the compositor's frame loop
/// makes) returns at once however long the consumer stays silent, the buffers a silent consumer holds are not
/// waited for, and when the consumer reads again frames flow again without a new stream.
fn a_stalled_consumer_never_blocks_the_pump(target: Target) {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let session = table.open(owner(), target, CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    let consumer = FrameConsumer::connect(&daemon.socket(), node);
    consumer.wait_for_frame_pumping(
        "before the stall",
        pump_of(&mut export, &mut table, &mut SolidSource),
        |frame| frame.is_solid(WINDOW_COLOUR),
    );

    // The consumer's loop is not iterated from here: its process callback never runs and it returns no buffer.
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let mut pumps = 0u32;
    let mut tick = 10_000;
    while started.elapsed() < Duration::from_millis(2000) {
        tick += 1;
        let before = Instant::now();
        export.pump(&mut table, Tick::new(tick), Capture::Desktop, &mut SolidSource);
        slowest = slowest.max(before.elapsed());
        pumps += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        slowest < PUMP_BOUND,
        "the slowest of {pumps} pumps took {slowest:?} while the consumer was silent"
    );
    assert!(pumps > 50, "the loop kept turning for the whole stall: {pumps} pumps");
    assert!(export.is_live(), "a silent consumer does not end its stream");
    assert_eq!(
        table.get(session).map(|entry| entry.stopped()),
        Some(None),
        "and does not stop the session"
    );

    consumer.forget();
    consumer.wait_for_frame_pumping(
        "after the consumer reads again",
        pump_of(&mut export, &mut table, &mut SolidSource),
        |frame| frame.is_solid(WINDOW_COLOUR),
    );
}

#[test]
fn a_stalled_consumer_of_an_output_never_blocks_the_pump() {
    a_stalled_consumer_never_blocks_the_pump(output());
}

#[test]
fn a_stalled_consumer_of_a_view_never_blocks_the_pump() {
    a_stalled_consumer_never_blocks_the_pump(Target::View(NodeId::new(5)));
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

/// A stream of a View or a window is the target at the size it is drawn at: when it grows and shrinks, the consumer
/// is renegotiated to the new size and receives whole pictures of it, not a crop of the size the stream started at.
fn a_stream_follows_the_size_its_target_is_drawn_at(target: Target) {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let view = target;
    let session = table.open(owner(), view, CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    let consumer = FrameConsumer::connect(&daemon.socket(), node);
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

#[test]
fn a_view_stream_follows_the_size_the_view_is_drawn_at() {
    a_stream_follows_the_size_its_target_is_drawn_at(Target::View(NodeId::new(5)));
}

#[test]
fn a_window_stream_follows_the_size_the_window_is_drawn_at() {
    a_stream_follows_the_size_its_target_is_drawn_at(Target::Window(WindowId::new(5)));
}

/// A frame of a window that shrank below the size the stream negotiated holds none of the pixels around the window:
/// what lies past the window's own rectangle is black, never the desktop beside it, even before the renegotiation.
#[test]
fn a_frame_never_carries_pixels_past_the_targets_own_rectangle() {
    let red = [0xff, 0x00, 0x00, 0xff].repeat(8 * 8);
    let image = Image::new(Size::new(8, 8), 8 * 4, red).expect("an image of red pixels");
    // The window is the 3 by 2 rectangle at (2, 1), and the stream still has its old 6 by 4 shape.
    let negotiated = Format::new(Size::new(6, 4), 24, PixelFormat::Xrgb8888).expect("a valid format");
    let mut plane = vec![0x55_u8; 6 * 4 * 4];
    assert_eq!(
        write_region(&image, Rect::new(2, 1, 3, 2), negotiated, &mut plane),
        Copied::Copied
    );
    let pixel = |x: usize, y: usize| -> [u8; 4] { plane[(y * 6 + x) * 4..][..4].try_into().expect("four bytes") };
    for y in 0..4 {
        for x in 0..6 {
            let inside = x < 3 && y < 2;
            let want = if inside {
                [0x00, 0x00, 0xff, 0xff]
            } else {
                [0, 0, 0, 0xff]
            };
            assert_eq!(pixel(x, y), want, "pixel ({x}, {y})");
        }
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
    let consumer = FrameConsumer::connect(&daemon.socket(), node);
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

/// A window is desktop content and the lock scene is not a window: the lock ends a window's stream although nobody
/// reads it, and the stream does not come back when the lock is released.
#[test]
fn the_lock_ends_a_window_stream_nobody_reads() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let window = table.open(
        owner(),
        Target::Window(WindowId::new(6)),
        CaptureStream::Granted,
        format(),
    );
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export.start(&table, window, owner()).expect("a window stream starts");
    observer.wait_for(node);

    for tick in 1..=5 {
        export.pump(&mut table, Tick::new(tick), Capture::LockScene, &mut Nothing);
    }
    assert!(
        observer.wait_gone(&stream_name(window)),
        "the lock removed the window's node although nobody read it"
    );
    assert_eq!(
        table.get(window).and_then(|entry| entry.stopped()),
        Some(Stop::Embargoed)
    );
    assert_eq!(
        export.start(&table, window, owner()),
        Err(ExportError::Stopped),
        "a window stream does not come back when the lock is released"
    );
}

/// A window that is gone ends its stream cleanly: a stream being read whose window has no bounds any more is stopped
/// by the table, and the next pump drops the node. The stream is not restarted.
#[test]
fn a_window_stream_ends_when_the_window_is_gone() {
    /// A window that is there until the test takes it away.
    struct Switch {
        present: bool,
    }
    impl PlaneSource for Switch {
        fn bounds(&self, _target: Target) -> Option<Rect> {
            self.present.then(|| Rect::from_size(SIZE))
        }
        fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, plane: &mut [u8]) -> Copied {
            for pixel in plane.as_chunks_mut::<4>().0 {
                *pixel = WINDOW_PIXEL;
            }
            Copied::Copied
        }
    }
    let daemon = Daemon::start();
    let mut table = Table::new();
    let window = table.open(
        owner(),
        Target::Window(WindowId::new(6)),
        CaptureStream::Granted,
        format(),
    );
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export.start(&table, window, owner()).expect("a window stream starts");
    let consumer = FrameConsumer::connect(&daemon.socket(), node);
    let mut source = Switch { present: true };
    consumer.wait_for_frame_pumping(
        "of the window",
        pump_of(&mut export, &mut table, &mut source),
        |frame| frame.is_solid(WINDOW_COLOUR),
    );

    source.present = false;
    consumer.listen_pumping(
        Duration::from_millis(500),
        pump_of(&mut export, &mut table, &mut source),
    );
    assert!(
        consumer.observer.wait_gone(&stream_name(window)),
        "the window's node left the daemon with the window"
    );
    assert_eq!(
        table.get(window).and_then(|entry| entry.stopped()),
        Some(Stop::TargetGone)
    );
}

/// The test helpers and the export open connections from parallel test threads. `pw_init` is not safe
/// to call from two threads at once, and a second `Once` beside the export's own made it so: the
/// workspace test run died with SIGSEGV in this binary. Every caller goes through one `Once`.
#[test]
fn concurrent_callers_initialise_pipewire_through_one_once() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let threads: Vec<_> = (0..16)
        .map(|_| {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                super::init_pipewire();
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("no caller crashes or panics");
    }
}

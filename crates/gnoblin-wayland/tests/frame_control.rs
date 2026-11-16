//! A shell that holds the tree controls two things the compositor does for each window: when the window's client is
//! told to draw its next frame (`frame_policy`), and what the shell hears of the frame that showed it (`presented`).
//! Every test runs a real client over a real socket, asks for frame callbacks and presentation feedback as a toolkit
//! does, and reads what the client was answered and what the shell was told.

mod session;
mod stock;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_render::transaction::TreeValue;
use session::shell::Shell;
use session::{App, Client, Session, Window};
use wayland_client::protocol::wl_callback;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};

/// How many frame callbacks a client has been answered.
type Answers = Arc<AtomicU32>;

/// The times, in milliseconds on the compositor's monotonic clock, a client's feedback said its frames were presented.
type Presented = Arc<Mutex<Vec<u64>>>;

impl Dispatch<wl_callback::WlCallback, Answers> for App {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        answers: &Answers,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            answers.fetch_add(1, Ordering::SeqCst);
        }
    }
}

wayland_client::delegate_noop!(App: ignore wp_presentation::WpPresentation);

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, Presented> for App {
    fn event(
        _: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        presented: &Presented,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation_feedback::Event::Presented {
            tv_sec_hi,
            tv_sec_lo,
            tv_nsec,
            ..
        } = event
        {
            let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
            presented
                .lock()
                .expect("the lock is not poisoned")
                .push(seconds * 1000 + u64::from(tv_nsec) / 1_000_000);
        }
    }
}

fn text(value: &str) -> TreeValue {
    TreeValue::Text(value.to_owned())
}

fn answered(answers: &Answers) -> u32 {
    answers.load(Ordering::SeqCst)
}

/// Asks for a frame callback on the window and draws it, as a toolkit does for each frame.
fn ask_and_draw(client: &mut Client, window: &Window, answers: &Answers) {
    window.surface.frame(&client.qh, Arc::clone(answers));
    client.attach_buffer(&window.surface, 200, 150);
}

/// Waits long enough for an answer the compositor means to send to have arrived, and checks that none has.
#[track_caller]
fn assert_still_unanswered(client: &mut Client, answers: &Answers, expected: u32, why: &str) {
    for _ in 0..24 {
        thread::sleep(Duration::from_millis(25));
        client.roundtrip();
        assert_eq!(answered(answers), expected, "{why}");
    }
}

/// How many callbacks a client that draws a frame whenever it is answered gets in `span`.
fn answers_within(client: &mut Client, window: &Window, span: Duration) -> u32 {
    let answers: Answers = Arc::default();
    let deadline = Instant::now() + span;
    while Instant::now() < deadline {
        let before = answered(&answers);
        ask_and_draw(client, window, &answers);
        client.wait_until("a frame callback", |_| answered(&answers) > before);
    }
    answered(&answers)
}

#[test]
fn a_held_window_is_answered_once_the_shell_sets_it_back_to_auto() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.paced", "Paced");
        let node = shell.wait_window_node();
        let answers: Answers = Arc::default();

        ask_and_draw(&mut client, &window, &answers);
        client.wait_until("the first frame callback", |_| answered(&answers) == 1);

        shell.write(node, &[("frame_policy", text("hold"))]);
        ask_and_draw(&mut client, &window, &answers);
        assert_still_unanswered(
            &mut client,
            &answers,
            1,
            "a held window is drawn, and its client is left waiting",
        );

        shell.write(node, &[("frame_policy", text("auto"))]);
        client.wait_until("the held callback", |_| answered(&answers) == 2);
    });
}

#[test]
fn a_window_the_shell_never_paced_is_answered_after_every_frame() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.plain", "Plain");
        let answers: Answers = Arc::default();
        for expected in 1..=3 {
            ask_and_draw(&mut client, &window, &answers);
            client.wait_until("a frame callback", |_| answered(&answers) == expected);
        }
    });
}

#[test]
fn a_throttled_window_is_answered_no_faster_than_its_rate() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.paced", "Paced");
        let node = shell.wait_window_node();

        let free = answers_within(&mut client, &window, Duration::from_millis(500));
        shell.write(node, &[("frame_policy", text("throttle")), ("frame_rate", TreeValue::Double(50.0))]);
        let throttled = answers_within(&mut client, &window, Duration::from_secs(1));
        assert!(
            (20..=60).contains(&throttled),
            "50 Hz over a second answers about 50 times, and neither stalls nor runs free: {throttled}"
        );
        assert!(
            free > throttled,
            "an unpaced window of the same client is answered far more often, even in half the time: {free} against {throttled}"
        );
    });
}

#[test]
fn a_held_window_is_answered_again_when_the_shell_lets_go_of_the_tree() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.paced", "Paced");
        let node = shell.wait_window_node();
        let seat = shell.wait_seat_node();
        shell.write(
            node,
            &[
                ("frame_policy", text("hold")),
                ("report_presented", TreeValue::Bool(true)),
            ],
        );
        shell.write(seat, &[("builtin_motion", TreeValue::Bool(false))]);
        let answers: Answers = Arc::default();
        ask_and_draw(&mut client, &window, &answers);
        assert_still_unanswered(&mut client, &answers, 0, "held while the shell holds the tree");

        shell.detach();
        client.wait_until("the callback of a window nobody holds", |_| answered(&answers) == 1);
        shell.wait_published(node, "frame_policy", &text("auto"));
        shell.wait_published(node, "report_presented", &TreeValue::Bool(false));
        shell.wait_published(seat, "builtin_motion", &TreeValue::Bool(true));
    });
}

#[test]
fn the_shell_hears_the_instant_the_window_reached_the_screen_that_its_client_was_told() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let seat = shell.wait_seat_node();
        shell.write(seat, &[("builtin_motion", TreeValue::Bool(false))]);
        let mut client = Client::connect(session.socket());
        let presentation: wp_presentation::WpPresentation = client
            .globals
            .bind(&client.qh, 1..=2, ())
            .expect("the session advertises wp_presentation");
        let window = client.map_window("org.example.timed", "Timed");
        let node = shell.wait_window_node();
        let answers: Answers = Arc::default();
        ask_and_draw(&mut client, &window, &answers);
        client.wait_until("a frame callback", |_| answered(&answers) == 1);
        assert_eq!(
            shell.published(node, "presented"),
            None,
            "a window the shell did not ask about is not stamped"
        );

        shell.write(node, &[("report_presented", TreeValue::Bool(true))]);
        let presented: Presented = Arc::default();
        let _feedback = presentation.feedback(&window.surface, &client.qh, Arc::clone(&presented));
        client.attach_buffer(&window.surface, 200, 150);
        client.wait_until("a presentation feedback", |_| {
            !presented.lock().expect("the lock is not poisoned").is_empty()
        });
        let told = presented.lock().expect("the lock is not poisoned")[0];
        assert!(told > 0, "the client was told a real time");
        let deadline = Instant::now() + session::TIMEOUT;
        loop {
            let heard = shell.published_all(node, "presented");
            if heard.contains(&TreeValue::Uint(told)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the shell was never told {told}, the same reading of the same clock the client was told: {heard:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
}

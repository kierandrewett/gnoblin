//! What the parent's protocol events leave behind, kept free of the wire.
//!
//! The `Dispatch` implementations in [`super::handler`] do one thing each: they
//! turn a protocol event into one call on this table. That split is what makes the
//! failure modes with teeth testable without a socket — a configure that arrives a
//! hundred times, a close that arrives twice, input that arrives before there is an
//! output to attribute it to.
//!
//! # Failure modes
//!
//! 1. **A resize storm leaves the session on a size that is already gone.** The
//!    table keeps one pending configure per output and the last one wins, so a
//!    hundred configures between two dispatches become one change of the final size:
//!    `tests::a_resize_storm_reports_the_latest_size_once`.
//! 2. **A configure that changes nothing is reported as a change.** The applied size
//!    and scale are remembered, so a parent that repeats itself costs no resize:
//!    `tests::a_configure_of_the_applied_size_is_not_a_change`.
//! 3. **A close that arrives twice takes an output that has already gone.** A close
//!    is reported once and the record stops answering configures:
//!    `tests::a_close_is_reported_once` and
//!    `tests::a_closed_output_ignores_later_configures`.
//! 4. **A buffer release is credited to the wrong pool.** A release carries the
//!    generation of the pool that made the buffer, so a release from a pool that was
//!    replaced by a resize is reported with its own generation and the target can
//!    ignore it: `tests::a_release_keeps_the_generation_of_its_pool`.
//! 5. **Input is attributed to an output that does not exist yet.** A surface has an
//!    output only once it has been configured, so the table refuses to name one
//!    before: `tests::a_surface_has_no_output_before_its_first_configure`, and the
//!    seat's own refusal is `input::tests::a_motion_without_an_output_is_dropped`.
//!
//! # State tables
//!
//! Each output is an [`OutputState`] and the connection is a [`ParentConnection`]. Every
//! event either takes a row below or is refused, and each row has a named test.
//!
//! Output (`configure` and `prefer_scale` take a size or scale only while `Open`):
//!
//! | From      | Event         | To        | Test                                                       |
//! |-----------|---------------|-----------|------------------------------------------------------------|
//! | `Open`    | `configure`   | `Open`    | `tests::an_open_output_takes_a_configure`                  |
//! | `Open`    | `prefer_scale`| `Open`    | `tests::an_open_output_takes_a_scale_preference`           |
//! | `Open`    | `close`       | `Closing` | `tests::a_close_request_marks_the_output_closing`          |
//! | `Closing` | `configure`   | refused   | `tests::a_closing_output_refuses_a_configure`              |
//! | `Closing` | `prefer_scale`| refused   | `tests::a_closing_output_refuses_a_scale_preference`       |
//! | `Closing` | `close`       | refused   | `tests::a_repeated_close_while_closing_is_refused`         |
//! | `Closing` | `take_changes`| `Closed`  | `tests::a_closing_output_is_reported_closed_once`          |
//! | `Closed`  | `configure`   | refused   | `tests::a_closed_output_ignores_later_configures`          |
//! | `Closed`  | `prefer_scale`| refused   | `tests::a_closed_output_refuses_a_scale_preference`        |
//! | `Closed`  | `close`       | refused   | `tests::a_closed_output_refuses_a_close`                   |
//!
//! ParentConnection:
//!
//! | From       | Event         | To         | Test                                                      |
//! |------------|---------------|------------|-----------------------------------------------------------|
//! | `Open`     | `disconnected`| `Lost`     | `tests::a_lost_connection_is_not_connected`               |
//! | `Lost`     | `disconnected`| refused    | `tests::a_repeated_loss_before_the_report_is_one_loss`    |
//! | `Lost`     | `take_changes`| `Reported` | `tests::a_lost_connection_is_reported_once`               |
//! | `Reported` | `disconnected`| refused    | `tests::a_reported_loss_is_not_reported_again`            |
//! | `Reported` | `take_changes`| `Reported` | `tests::a_reported_loss_stays_a_loss`                     |

use gnoblin_types::{OutputId, Scale, Size};

use super::input::Seat;

/// What the table has to tell the backend after a batch of parent events.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    /// Outputs whose applied size or scale changed: index, size and scale.
    pub configured: Vec<(usize, Size, Scale)>,
    /// Outputs the parent asked to close.
    pub closed: Vec<usize>,
    /// Outputs the parent finished a frame on.
    pub frame_done: Vec<usize>,
    /// Buffers the parent released: index, pool generation and slot.
    pub released: Vec<(usize, u32, u8)>,
    /// The connection to the parent ended.
    pub disconnected: bool,
}

/// Whether an output is still wanted, and what the backend has been told about closing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputState {
    /// The parent has not asked for the output to close.
    Open,
    /// The parent asked for it to close, and the backend has not been told yet.
    Closing,
    /// The backend has been told. Later events for the output are ignored.
    Closed,
}

/// Whether the connection to the parent is up, and whether its end has been reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentConnection {
    /// The parent is connected.
    Open,
    /// The parent is gone, and the backend has not been told yet.
    Lost,
    /// The parent is gone, and the backend has been told once. It is not told again.
    Reported,
}

/// One nested output's protocol state.
#[derive(Debug)]
struct Record {
    output: OutputId,
    connector: String,
    /// The size and scale the backend has been told about.
    applied: Option<(Size, Scale)>,
    /// The size of the latest configure the backend has not been told about yet.
    size: Option<Size>,
    /// The scale the parent last preferred, from `wp_fractional_scale_v1`.
    scale: Scale,
    /// Where the parent's request to close the output has got to.
    state: OutputState,
    frame_done: bool,
    released: Vec<(u32, u8)>,
}

/// The parent-facing state of every nested output, plus the seat.
#[derive(Debug)]
pub(crate) struct Table {
    records: Vec<Record>,
    seat: Seat,
    connection: ParentConnection,
}

impl Table {
    /// A table with no outputs.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            seat: Seat::new(),
            connection: ParentConnection::Open,
        }
    }

    /// Adds an output the parent has not configured yet.
    pub fn add(&mut self, output: OutputId, connector: impl Into<String>) -> usize {
        self.records.push(Record {
            output,
            connector: connector.into(),
            applied: None,
            size: None,
            scale: Scale::IDENTITY,
            state: OutputState::Open,
            frame_done: false,
            released: Vec::new(),
        });
        self.records.len() - 1
    }

    /// An output's identity, if the index is one this table handed out.
    pub fn output(&self, index: usize) -> Option<OutputId> {
        self.records.get(index).map(|record| record.output)
    }

    /// An output's connector name.
    pub fn connector(&self, index: usize) -> Option<&str> {
        self.records.get(index).map(|record| record.connector.as_str())
    }

    /// The size and scale the backend has been told about, if it has been told.
    pub fn applied(&self, index: usize) -> Option<(Size, Scale)> {
        self.records.get(index).and_then(|record| record.applied)
    }

    /// The parent asked for `size`; the latest configure wins.
    pub fn configure(&mut self, index: usize, size: Size) {
        if let Some(record) = self.records.get_mut(index)
            && record.state == OutputState::Open
        {
            record.size = Some(size);
        }
    }

    /// The parent preferred `scale`, which `wp_fractional_scale_v1` states before
    /// the configure it belongs to.
    pub fn prefer_scale(&mut self, index: usize, scale: Scale) {
        if let Some(record) = self.records.get_mut(index)
            && record.state == OutputState::Open
        {
            record.scale = scale;
        }
    }

    /// The parent asked the output to close.
    pub fn close(&mut self, index: usize) {
        if let Some(record) = self.records.get_mut(index)
            && record.state == OutputState::Open
        {
            record.state = OutputState::Closing;
        }
    }

    /// The parent finished drawing the frame it was asked for.
    pub fn frame_done(&mut self, index: usize) {
        if let Some(record) = self.records.get_mut(index) {
            record.frame_done = true;
        }
    }

    /// The parent released a buffer this backend attached.
    pub fn released(&mut self, index: usize, generation: u32, slot: u8) {
        if let Some(record) = self.records.get_mut(index) {
            record.released.push((generation, slot));
        }
    }

    /// The connection to the parent ended.
    pub fn disconnected(&mut self) {
        if self.connection == ParentConnection::Open {
            self.connection = ParentConnection::Lost;
        }
    }

    /// Whether the connection to the parent is still up.
    pub fn is_connected(&self) -> bool {
        self.connection == ParentConnection::Open
    }

    /// The seat the parent's input arrives on.
    pub fn seat(&mut self) -> &mut Seat {
        &mut self.seat
    }

    /// How many input events the seat could not attribute to an output.
    pub fn dropped(&self) -> u64 {
        self.seat.dropped()
    }

    /// Takes everything that happened since the last call.
    ///
    /// A configure is only reported when it changes what the backend already knows,
    /// and the last configure of a storm is the one reported, because it overwrote
    /// the earlier ones in the record.
    pub fn take_changes(&mut self) -> Changes {
        let mut changes = Changes::default();
        if self.connection == ParentConnection::Lost {
            self.connection = ParentConnection::Reported;
            changes.disconnected = true;
        }
        for (index, record) in self.records.iter_mut().enumerate() {
            let size = record.size.or_else(|| record.applied.map(|(size, _)| size));
            if let Some(size) = size {
                let wanted = (size, record.scale);
                if record.applied != Some(wanted) {
                    record.applied = Some(wanted);
                    changes.configured.push((index, size, record.scale));
                }
            }
            if record.state == OutputState::Closing {
                record.state = OutputState::Closed;
                changes.closed.push(index);
            }
            if record.frame_done {
                record.frame_done = false;
                changes.frame_done.push(index);
            }
            changes.released.extend(
                record
                    .released
                    .drain(..)
                    .map(|(generation, slot)| (index, generation, slot)),
            );
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: OutputId = OutputId::new(1);

    fn table() -> (Table, usize) {
        let mut table = Table::new();
        let index = table.add(OUTPUT, "NESTED-1");
        (table, index)
    }

    #[test]
    fn a_resize_storm_reports_the_latest_size_once() {
        let (mut table, index) = table();
        for height in 1..=100 {
            table.configure(index, Size::new(1024, height));
        }
        let changes = table.take_changes();
        assert_eq!(
            changes.configured,
            vec![(index, Size::new(1024, 100), Scale::IDENTITY)],
            "one change, at the size the storm ended on"
        );
        assert!(
            table.take_changes().configured.is_empty(),
            "and nothing is repeated on the next dispatch"
        );
    }

    #[test]
    fn a_configure_of_the_applied_size_is_not_a_change() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        assert_eq!(
            table.take_changes().configured,
            vec![(index, Size::new(800, 600), Scale::IDENTITY)]
        );
        table.configure(index, Size::new(800, 600));
        assert!(table.take_changes().configured.is_empty(), "the parent repeated itself");
        assert_eq!(table.applied(index), Some((Size::new(800, 600), Scale::IDENTITY)));
    }

    #[test]
    fn a_scale_change_is_a_configure_of_its_own() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.prefer_scale(index, Scale::new(3, 2));
        assert_eq!(
            table.take_changes().configured,
            vec![(index, Size::new(800, 600), Scale::new(3, 2))],
            "a preferred scale with no new size still changes what the output composes at"
        );
    }

    #[test]
    fn a_close_is_reported_once() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.close(index);
        table.close(index);
        assert_eq!(table.take_changes().closed, vec![index]);
        assert!(
            table.take_changes().closed.is_empty(),
            "the output has already gone; it cannot go twice"
        );
    }

    #[test]
    fn a_closed_output_ignores_later_configures() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.close(index);
        table.configure(index, Size::new(640, 480));
        let changes = table.take_changes();
        assert_eq!(changes.closed, vec![index]);
        assert!(
            changes.configured.is_empty(),
            "a parent that configures a toplevel it just closed does not resize it on the way out"
        );
    }

    #[test]
    fn a_release_keeps_the_generation_of_its_pool() {
        let (mut table, index) = table();
        table.released(index, 7, 1);
        table.released(index, 8, 0);
        assert_eq!(table.take_changes().released, vec![(index, 7, 1), (index, 8, 0)]);
        assert!(table.take_changes().released.is_empty(), "releases are reported once");
    }

    #[test]
    fn a_surface_has_no_output_before_its_first_configure() {
        let (table, index) = table();
        assert_eq!(
            table.applied(index),
            None,
            "no size means no output to attribute input to"
        );
        assert_eq!(table.output(index), Some(OUTPUT), "the identity exists from the start");
        assert_eq!(
            table.output(index + 1),
            None,
            "an index the table never handed out is not an output"
        );
        assert_eq!(table.connector(index), Some("NESTED-1"));
    }

    #[test]
    fn a_disconnect_is_reported_once() {
        let (mut table, _) = table();
        table.disconnected();
        assert!(table.take_changes().disconnected);
        assert!(
            !table.take_changes().disconnected,
            "a disconnect is one event, not a state to poll"
        );
    }

    #[test]
    fn a_lost_connection_is_not_connected() {
        let (mut table, _) = table();
        assert!(table.is_connected());
        table.disconnected();
        assert!(!table.is_connected());
    }

    #[test]
    fn a_repeated_loss_before_the_report_is_one_loss() {
        let (mut table, _) = table();
        table.disconnected();
        table.disconnected();
        assert!(table.take_changes().disconnected);
        assert!(
            !table.take_changes().disconnected,
            "two losses before the report are one event"
        );
    }

    #[test]
    fn a_lost_connection_is_reported_once() {
        let (mut table, _) = table();
        table.disconnected();
        assert!(table.take_changes().disconnected);
        assert!(!table.is_connected(), "a reported loss stays a loss");
    }

    #[test]
    fn a_reported_loss_is_not_reported_again() {
        let (mut table, _) = table();
        table.disconnected();
        assert!(table.take_changes().disconnected);
        table.disconnected();
        assert!(
            !table.take_changes().disconnected,
            "a loss after the report is refused, not a second event"
        );
    }

    #[test]
    fn a_reported_loss_stays_a_loss() {
        let (mut table, _) = table();
        table.disconnected();
        let _ = table.take_changes();
        let _ = table.take_changes();
        assert!(!table.is_connected());
    }

    #[test]
    fn an_open_output_takes_a_configure() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        assert_eq!(
            table.take_changes().configured,
            vec![(index, Size::new(800, 600), Scale::IDENTITY)]
        );
    }

    #[test]
    fn an_open_output_takes_a_scale_preference() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        table.prefer_scale(index, Scale::new(3, 2));
        assert_eq!(
            table.take_changes().configured,
            vec![(index, Size::new(800, 600), Scale::new(3, 2))]
        );
    }

    #[test]
    fn a_close_request_marks_the_output_closing() {
        let (mut table, index) = table();
        table.close(index);
        assert_eq!(table.records[index].state, OutputState::Closing);
    }

    #[test]
    fn a_closing_output_refuses_a_configure() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.close(index);
        table.configure(index, Size::new(640, 480));
        let changes = table.take_changes();
        assert!(changes.configured.is_empty(), "a closing output keeps its size");
        assert_eq!(changes.closed, vec![index]);
    }

    #[test]
    fn a_closing_output_refuses_a_scale_preference() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.close(index);
        table.prefer_scale(index, Scale::new(3, 2));
        assert!(
            table.take_changes().configured.is_empty(),
            "a closing output takes no scale preference"
        );
    }

    #[test]
    fn a_repeated_close_while_closing_is_refused() {
        let (mut table, index) = table();
        table.close(index);
        table.close(index);
        assert_eq!(table.take_changes().closed, vec![index]);
        assert_eq!(table.records[index].state, OutputState::Closed);
    }

    #[test]
    fn a_closing_output_is_reported_closed_once() {
        let (mut table, index) = table();
        table.close(index);
        assert_eq!(table.take_changes().closed, vec![index]);
        assert!(
            table.take_changes().closed.is_empty(),
            "the output has already gone; it cannot go twice"
        );
    }

    #[test]
    fn a_closed_output_refuses_a_scale_preference() {
        let (mut table, index) = table();
        table.configure(index, Size::new(800, 600));
        let _ = table.take_changes();
        table.close(index);
        let _ = table.take_changes();
        table.prefer_scale(index, Scale::new(3, 2));
        assert!(
            table.take_changes().configured.is_empty(),
            "a closed output takes no scale preference"
        );
    }

    #[test]
    fn a_closed_output_refuses_a_close() {
        let (mut table, index) = table();
        table.close(index);
        let _ = table.take_changes();
        table.close(index);
        assert!(
            table.take_changes().closed.is_empty(),
            "closed is terminal, not closed again"
        );
    }
}

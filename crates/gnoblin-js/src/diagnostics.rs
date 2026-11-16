//! The diagnostics a role reports, kept bounded.
//!
//! A role reports a diagnostic for a failed reload, a listener that threw, a refused transaction and the like, and a
//! shell that runs for weeks reports as many as it fails. The list a test or a status reader asks for must not grow
//! with them, so it keeps the newest [`DIAGNOSTICS_KEPT`] lines, each cut at [`DIAGNOSTIC_LINE_BYTES`], and counts
//! what it let go. Once it has let one go, the first line of the list says how many: one summary line, rewritten as the
//! count grows, so a reader of the list sees at a glance that it is not the whole story.
//!
//! A line is named by how many were reported before it, not by its place in the list, so a caller that wants what
//! arrived since it last looked asks [`Diagnostics::since`] with the [`Diagnostics::reported`] it took then, and the
//! answer does not shift when the list drops its oldest.

/// The most lines the list keeps, not counting the summary line.
pub const DIAGNOSTICS_KEPT: usize = 256;

/// The longest line the list keeps, in bytes. A longer one is cut at a character boundary and ends in `...`.
pub const DIAGNOSTIC_LINE_BYTES: usize = 4096;

/// The newest diagnostics of a role, and how many it has let go.
#[derive(Debug)]
pub(crate) struct Diagnostics {
    /// The summary line when anything was dropped, then the kept lines, oldest first.
    lines: Vec<String>,
    /// How many lines were ever reported, the dropped ones included.
    reported: u64,
    /// How many were let go to stay within `limit`.
    dropped: u64,
    /// The most lines kept, not counting the summary.
    limit: usize,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new(DIAGNOSTICS_KEPT)
    }
}

impl Diagnostics {
    /// An empty list that keeps the newest `limit` lines, and at least one.
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            lines: Vec::new(),
            reported: 0,
            dropped: 0,
            limit: limit.max(1),
        }
    }

    /// Keeps `line`, and lets the oldest go when the list is full.
    pub(crate) fn push(&mut self, line: String) {
        self.reported += 1;
        self.lines.push(cut(line));
        if self.kept() > self.limit {
            // The oldest kept line goes; the summary, when there is one, stays first.
            self.lines.remove(usize::from(self.dropped > 0));
            self.dropped += 1;
            let summary = format!(
                "{} earlier diagnostics were dropped: the list keeps the newest {}",
                self.dropped, self.limit
            );
            if self.dropped == 1 {
                self.lines.insert(0, summary);
            } else {
                self.lines[0] = summary;
            }
        }
    }

    /// The list: the summary line first when anything was dropped, then the kept lines, oldest first.
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    /// How many lines were ever reported, the dropped ones included.
    pub(crate) fn reported(&self) -> u64 {
        self.reported
    }

    /// How many lines were let go.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The kept lines reported after the first `reported`: what arrived since a caller that took
    /// [`Self::reported`] then last looked. A line that was dropped since is not in it.
    pub(crate) fn since(&self, reported: u64) -> &[String] {
        let kept = self.kept() as u64;
        // The kept lines are the last `kept` of those reported.
        let first_kept = self.reported - kept;
        let first_new = reported.max(first_kept);
        let skip = usize::from(self.dropped > 0) + usize::try_from(first_new - first_kept).unwrap_or(usize::MAX);
        self.lines.get(skip..).unwrap_or_default()
    }

    /// How many real lines are in the list: all but the summary.
    fn kept(&self) -> usize {
        self.lines.len() - usize::from(self.dropped > 0)
    }
}

/// `line`, cut at [`DIAGNOSTIC_LINE_BYTES`] on a character boundary.
fn cut(mut line: String) -> String {
    if line.len() <= DIAGNOSTIC_LINE_BYTES {
        return line;
    }
    let mut end = DIAGNOSTIC_LINE_BYTES - 3;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    line.push_str("...");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(count: usize) -> Diagnostics {
        let mut list = Diagnostics::default();
        for n in 0..count {
            list.push(format!("line {n}"));
        }
        list
    }

    #[test]
    fn a_list_within_its_bound_is_every_line_and_no_summary() {
        let list = filled(DIAGNOSTICS_KEPT);
        assert_eq!(list.lines().len(), DIAGNOSTICS_KEPT);
        assert_eq!(list.lines()[0], "line 0");
        assert_eq!((list.reported(), list.dropped()), (DIAGNOSTICS_KEPT as u64, 0));
    }

    #[test]
    fn a_flood_keeps_the_newest_lines_and_one_summary_that_counts_what_it_let_go() {
        let list = filled(DIAGNOSTICS_KEPT + 1_000);
        assert_eq!(
            list.lines().len(),
            DIAGNOSTICS_KEPT + 1,
            "the kept lines and one summary"
        );
        assert_eq!(list.dropped(), 1_000);
        assert!(
            list.lines()[0].starts_with("1000 earlier diagnostics were dropped"),
            "{:?}",
            list.lines()[0]
        );
        assert_eq!(
            list.lines()[1],
            "line 1000",
            "the oldest kept line is the 1001st reported"
        );
        assert_eq!(
            list.lines().last().map(String::as_str),
            Some(format!("line {}", DIAGNOSTICS_KEPT + 999).as_str())
        );
        assert_eq!(
            list.lines().iter().filter(|line| line.contains("dropped")).count(),
            1,
            "one summary line, not one per drop"
        );
        assert_eq!(list.reported(), (DIAGNOSTICS_KEPT + 1_000) as u64);
    }

    #[test]
    fn what_arrived_since_a_look_does_not_shift_when_the_list_drops_its_oldest() {
        let mut list = filled(10);
        let mark = list.reported();
        assert!(list.since(mark).is_empty());
        list.push("new".to_owned());
        assert_eq!(list.since(mark), ["new".to_owned()]);
        // A flood after the look: what is asked for is only what is still kept, and never the summary.
        for n in 0..DIAGNOSTICS_KEPT * 2 {
            list.push(format!("flood {n}"));
        }
        let later = list.since(mark);
        assert_eq!(later.len(), DIAGNOSTICS_KEPT);
        assert_eq!(later[0], format!("flood {}", DIAGNOSTICS_KEPT));
        assert!(later.iter().all(|line| !line.contains("dropped")));
        assert!(list.since(list.reported()).is_empty());
    }

    #[test]
    fn a_line_is_cut_at_a_character_boundary() {
        let mut list = Diagnostics::default();
        list.push("\u{1F600}".repeat(DIAGNOSTIC_LINE_BYTES));
        let kept = &list.lines()[0];
        assert!(kept.len() <= DIAGNOSTIC_LINE_BYTES, "{}", kept.len());
        assert!(kept.ends_with("..."));
        assert!(kept.starts_with('\u{1F600}'));
    }
}

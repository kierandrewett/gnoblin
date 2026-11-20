//! Keyboard selection across a menu and its open submenu. Each card keeps its selection when the submenu closes.
//!
//! The navigator holds rows (canvases of the toolkit, by name) and a place in each card; what a row says (its label, whether it can be
//! run) is read through the functions each call is given, so the same code serves a card and a navigator a shell made.

use crate::ui::Cid;

/// What the navigator asks of a row.
pub(crate) struct NavRow {
    pub(crate) label: String,
    pub(crate) enabled: bool,
}

/// The selection across the main card and its submenu.
#[derive(Clone, Debug)]
pub(crate) struct Nav {
    pub(crate) rows: Vec<Cid>,
    pub(crate) sub: Vec<Cid>,
    depth: u8,
    picked: [i64; 2],
}

fn at_of(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}

impl Nav {
    pub(crate) fn new(rows: Vec<Cid>) -> Nav {
        Nav {
            rows,
            sub: Vec::new(),
            depth: 1,
            picked: [-1, -1],
        }
    }

    /// Whether the selection is in the main card (1) or the submenu (2).
    pub(crate) fn depth(&self) -> u8 {
        self.depth
    }

    /// The rows of the active card.
    pub(crate) fn active(&self) -> &[Cid] {
        if self.depth == 2 { &self.sub } else { &self.rows }
    }

    /// The selected row's place in the active card, or -1.
    pub(crate) fn index(&self) -> i64 {
        self.picked[usize::from(self.depth) - 1]
    }

    fn set_index(&mut self, at: i64) {
        self.picked[usize::from(self.depth) - 1] = at;
    }

    /// The row the keyboard is on.
    pub(crate) fn selected(&self) -> Option<Cid> {
        usize::try_from(self.index())
            .ok()
            .and_then(|at| self.active().get(at).copied())
    }

    /// Whether `row` is selected, in either card.
    pub(crate) fn is_selected(&self, row: Cid) -> bool {
        let held = |rows: &[Cid], at: i64| usize::try_from(at).ok().and_then(|at| rows.get(at).copied()) == Some(row);
        held(&self.rows, self.picked[0]) || held(&self.sub, self.picked[1])
    }

    /// Selects `row`; selecting a row outside the submenu closes it.
    pub(crate) fn select(&mut self, row: Cid) {
        if self.depth == 2 && !self.sub.contains(&row) {
            self.close_submenu();
        }
        if let Some(at) = self.active().iter().position(|held| *held == row) {
            self.set_index(at_of(at));
        }
    }

    /// Moves the selection by `step` over the rows that can be run, wrapping at the ends. With nothing selected a step forward lands on
    /// the first row and a step back on the last.
    pub(crate) fn step(&mut self, step: i64, row: &dyn Fn(Cid) -> NavRow) {
        let count = at_of(self.active().len());
        let mut at = self.index();
        if at < 0 {
            at = if step > 0 { -1 } else { count };
        }
        for _ in 0..count {
            at = (at + step).rem_euclid(count);
            let held = usize::try_from(at).ok().and_then(|at| self.active().get(at).copied());
            if held.is_some_and(|held| row(held).enabled) {
                self.set_index(at);
                return;
            }
        }
    }

    /// Selects the first row that can be run.
    pub(crate) fn home(&mut self, row: &dyn Fn(Cid) -> NavRow) {
        self.set_index(-1);
        self.step(1, row);
    }

    /// Selects the last row that can be run.
    pub(crate) fn end(&mut self, row: &dyn Fn(Cid) -> NavRow) {
        self.set_index(-1);
        self.step(-1, row);
    }

    /// Selects the next row that can be run whose label starts with `letter`, ignoring case.
    pub(crate) fn jump(&mut self, letter: &str, row: &dyn Fn(Cid) -> NavRow) {
        let count = at_of(self.active().len());
        let want = letter.to_lowercase();
        for step in 1..=count {
            let at = (self.index() + step + count).rem_euclid(count);
            let Some(held) = usize::try_from(at).ok().and_then(|at| self.active().get(at).copied()) else {
                continue;
            };
            let held = row(held);
            let first: String = held.label.chars().take(1).collect();
            if held.enabled && first.to_lowercase() == want {
                self.set_index(at);
                return;
            }
        }
    }

    /// Opens a submenu and selects its first row that can be run.
    pub(crate) fn open_submenu(&mut self, rows: Vec<Cid>, row: &dyn Fn(Cid) -> NavRow) {
        self.close_submenu();
        self.sub = rows;
        self.depth = 2;
        self.picked[1] = -1;
        self.step(1, row);
    }

    /// Closes the submenu and gives the main card back its selection.
    pub(crate) fn close_submenu(&mut self) {
        if self.depth != 2 {
            return;
        }
        self.sub.clear();
        self.depth = 1;
        self.picked[1] = -1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Vec<&'static str> {
        vec![
            "Maximise",
            "Minimise",
            "Move",
            "Resize",
            "Always on top",
            "Move to workspace",
            "Close",
        ]
    }

    fn info(cid: Cid) -> NavRow {
        NavRow {
            label: labels()[usize::try_from(cid).unwrap_or(0) - 1].to_owned(),
            enabled: true,
        }
    }

    fn nav() -> Nav {
        Nav::new((1..=7).collect())
    }

    fn at(nav: &Nav) -> &'static str {
        nav.selected()
            .map_or("", |cid| labels()[usize::try_from(cid).unwrap_or(0) - 1])
    }

    #[test]
    fn a_step_back_with_nothing_selected_lands_on_the_last_row() {
        let mut nav = nav();
        nav.step(-1, &info);
        assert_eq!(at(&nav), "Close");
        nav.home(&info);
        assert_eq!(at(&nav), "Maximise");
        nav.end(&info);
        nav.step(1, &info);
        assert_eq!(at(&nav), "Maximise");
    }

    #[test]
    fn a_letter_jumps_to_the_next_row_with_it_and_wraps() {
        let mut nav = nav();
        nav.jump("c", &info);
        assert_eq!(at(&nav), "Close");
        nav.jump("m", &info);
        assert_eq!(at(&nav), "Maximise");
        nav.jump("M", &info);
        assert_eq!(at(&nav), "Minimise");
    }

    #[test]
    fn a_disabled_row_is_skipped() {
        let mut nav = nav();
        let skip = |cid: Cid| NavRow {
            label: info(cid).label,
            enabled: cid != 2,
        };
        nav.step(1, &skip);
        nav.step(1, &skip);
        assert_eq!(nav.selected(), Some(3));
    }

    #[test]
    fn the_submenu_keeps_its_own_selection_and_gives_the_card_back_its_own() {
        let mut nav = nav();
        nav.step(1, &info);
        nav.open_submenu(vec![3, 4], &info);
        assert_eq!((nav.depth(), nav.selected()), (2, Some(3)));
        assert!(nav.is_selected(1) && nav.is_selected(3));
        nav.close_submenu();
        assert_eq!((nav.depth(), nav.selected()), (1, Some(1)));
        assert!(!nav.is_selected(3));
    }

    #[test]
    fn selecting_a_row_outside_the_submenu_closes_it() {
        let mut nav = nav();
        nav.open_submenu(vec![3, 4], &info);
        nav.select(6);
        assert_eq!((nav.depth(), nav.selected()), (1, Some(6)));
    }
}

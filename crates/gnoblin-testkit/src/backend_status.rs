//! The per-backend status table, read from the committed markdown so a test can hold the document to what the code
//! does.
//!
//! `design/backend-parity.md` carries one row per capability and one cell per backend. A cell is a status word, an
//! optional colon and a note: `supported`, `refused: the host owns the window size`, `n/a: no window`. The
//! conformance tests measure the same capability on each backend and compare the status word with the cell, so the
//! table cannot say something the backend does not do, and a capability nobody measures cannot sit in it. The user
//! page repeats the table in consumer words; [`StatusTable::parse`] reads either, and a test compares them.
//!
//! The grammar is small on purpose: a table between two marker comments, cells split on `|`, no escapes. A note that
//! needs a bar is reworded.

use std::collections::BTreeMap;
use std::fmt;

/// The backends a cell exists for, in column order.
pub const BACKENDS: [&str; 3] = ["headless", "nested", "drm"];

/// What a backend does with a capability.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellStatus {
    /// The backend does it.
    Supported,
    /// The backend refuses it on purpose, with the same typed answer on every backend that refuses.
    Refused,
    /// The capability does not exist for this kind of backend: a window has no DRM master.
    NotApplicable,
}

impl CellStatus {
    /// The word a cell starts with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            CellStatus::Supported => "supported",
            CellStatus::Refused => "refused",
            CellStatus::NotApplicable => "n/a",
        }
    }

    fn of(word: &str) -> Option<CellStatus> {
        [CellStatus::Supported, CellStatus::Refused, CellStatus::NotApplicable]
            .into_iter()
            .find(|status| status.word() == word)
    }
}

impl fmt::Display for CellStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.word())
    }
}

/// One cell: what a backend does and the note that says how or why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The status word.
    pub status: CellStatus,
    /// The text after the colon; empty when the cell has none. A refusal and a not-applicable cell must have one.
    pub note: String,
}

/// One capability and what each backend does with it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// The stable name tests use, `area.thing`.
    pub id: String,
    /// The words the user page uses for it.
    pub capability: String,
    /// The cells, in [`BACKENDS`] order.
    pub cells: [Cell; 3],
}

impl Row {
    /// The cell of `backend`.
    ///
    /// # Panics
    ///
    /// For a name that is not in [`BACKENDS`].
    #[must_use]
    pub fn cell(&self, backend: &str) -> &Cell {
        let index = BACKENDS
            .iter()
            .position(|name| *name == backend)
            .unwrap_or_else(|| panic!("`{backend}` is not a backend: {BACKENDS:?}"));
        &self.cells[index]
    }
}

/// The table: its rows in document order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusTable {
    /// Every row.
    pub rows: Vec<Row>,
}

const START: &str = "<!-- backend-status:start -->";
const END: &str = "<!-- backend-status:end -->";

/// The lines between the status markers of `markdown`.
fn between(markdown: &str) -> Result<Vec<&str>, String> {
    let mut lines = markdown.lines();
    lines
        .by_ref()
        .find(|line| line.trim() == START)
        .ok_or_else(|| format!("no `{START}` marker"))?;
    let mut body = Vec::new();
    for line in lines {
        if line.trim() == END {
            return Ok(body);
        }
        body.push(line);
    }
    Err(format!("no `{END}` marker"))
}

/// The cells of one table line, trimmed, without the outer bars.
fn cells_of(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    line.split('|').map(|cell| cell.trim().to_owned()).collect()
}

fn parse_cell(text: &str) -> Result<Cell, String> {
    let (word, note) = match text.split_once(':') {
        Some((word, note)) => (word.trim(), note.trim()),
        None => (text.trim(), ""),
    };
    let status = CellStatus::of(&word.to_lowercase())
        .ok_or_else(|| format!("`{text}` does not start with supported, refused or n/a"))?;
    if status != CellStatus::Supported && note.is_empty() {
        return Err(format!("`{text}`: a {status} cell says why"));
    }
    Ok(Cell {
        status,
        note: note.to_owned(),
    })
}

impl StatusTable {
    /// Reads the table between the status markers of `markdown`. Two layouts are accepted: the design's, with an
    /// `Id` column before the capability, and the user page's, without one (its rows get the capability as id).
    ///
    /// # Errors
    ///
    /// A message naming the line that is not a row, a cell that is not a status, or a missing marker.
    pub fn parse(markdown: &str) -> Result<StatusTable, String> {
        let body = between(markdown)?;
        let mut rows = Vec::new();
        let mut header: Option<Vec<String>> = None;
        for line in body.into_iter().filter(|line| line.trim_start().starts_with('|')) {
            let cells = cells_of(line);
            if cells
                .iter()
                .all(|cell| cell.chars().all(|c| matches!(c, '-' | ':' | ' ')))
            {
                continue;
            }
            let Some(header) = &header else {
                header = Some(cells);
                continue;
            };
            let with_id = header.first().is_some_and(|first| first.eq_ignore_ascii_case("id"));
            let leading = usize::from(with_id) + 1;
            if cells.len() != leading + BACKENDS.len() {
                return Err(format!(
                    "`{line}` has {} cells, expected {}",
                    cells.len(),
                    leading + BACKENDS.len()
                ));
            }
            let (id, capability) = if with_id {
                (cells[0].trim_matches('`').to_owned(), cells[1].clone())
            } else {
                (cells[0].clone(), cells[0].clone())
            };
            let mut parsed = Vec::new();
            for (backend, text) in BACKENDS.iter().zip(&cells[leading..]) {
                parsed.push(parse_cell(text).map_err(|error| format!("{id}, {backend}: {error}"))?);
            }
            let cells: [Cell; 3] = parsed.try_into().map_err(|_| "three cells".to_owned())?;
            rows.push(Row { id, capability, cells });
        }
        if rows.is_empty() {
            return Err("the table has no rows".to_owned());
        }
        let mut seen = BTreeMap::new();
        for row in &rows {
            if let Some(first) = seen.insert(row.id.clone(), row.capability.clone()) {
                return Err(format!(
                    "`{}` appears twice ({first:?} and {:?})",
                    row.id, row.capability
                ));
            }
        }
        Ok(StatusTable { rows })
    }

    /// The row with `id`.
    ///
    /// # Panics
    ///
    /// When the table has no such row: a test that measures a capability the table does not list has found the table
    /// out of date.
    #[must_use]
    pub fn row(&self, id: &str) -> &Row {
        self.rows
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("the status table has no row `{id}`"))
    }

    /// Every row id, in document order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|row| row.id.as_str())
    }
}

/// Records what a test measured per backend, and checks it against the table.
#[derive(Debug)]
pub struct ColumnCheck<'a> {
    table: &'a StatusTable,
    backend: &'static str,
    seen: BTreeMap<String, CellStatus>,
}

impl<'a> ColumnCheck<'a> {
    /// A recorder for one backend's column.
    ///
    /// # Panics
    ///
    /// For a name that is not in [`BACKENDS`].
    #[must_use]
    pub fn new(table: &'a StatusTable, backend: &'static str) -> Self {
        assert!(BACKENDS.contains(&backend), "`{backend}` is not a backend");
        Self {
            table,
            backend,
            seen: BTreeMap::new(),
        }
    }

    /// Records that `id` was measured as `status` on this backend, and fails when the table says otherwise.
    ///
    /// # Panics
    ///
    /// When the cell's status differs from what was measured, or the row is missing.
    pub fn is(&mut self, id: &str, status: CellStatus) {
        let cell = self.table.row(id).cell(self.backend);
        assert_eq!(
            cell.status, status,
            "the status table says `{id}` is {} on {}, and it measured {status}",
            cell.status, self.backend
        );
        self.seen.insert(id.to_owned(), status);
    }

    /// The backend this recorder measures.
    #[must_use]
    pub fn backend(&self) -> &'static str {
        self.backend
    }

    /// Records that `id` is `status` on this backend because the test `test` in `source` (the text of the test file)
    /// proves it. The test must still exist, so a rename or a deletion fails here instead of leaving the table
    /// vouching for nothing.
    ///
    /// # Panics
    ///
    /// When `source` has no test of that name, or the table disagrees.
    pub fn covered(&mut self, id: &str, status: CellStatus, test: &str, source: &str) {
        assert!(
            source.contains(&format!("fn {test}(")),
            "`{id}` on {} is covered by `{test}`, which no longer exists",
            self.backend
        );
        self.is(id, status);
    }

    /// Records that `id` could not be measured on this machine for `why` (no GPU renderer, say), loudly, so a machine
    /// that lacks a driver still runs the rest of the table. The table's cell is not compared.
    pub fn skipped(&mut self, id: &str, why: &str) {
        println!("skipping `{id}` on {}: {why}", self.backend);
        let status = self.table.row(id).cell(self.backend).status;
        self.seen.insert(id.to_owned(), status);
    }

    /// [`ColumnCheck::skipped`] for each of `ids` that nothing measured, for a machine that lacks what measuring them
    /// needs.
    pub fn skip_unseen(&mut self, ids: &[&str], why: &str) {
        for id in ids {
            if !self.seen.contains_key(*id) {
                self.skipped(id, why);
            }
        }
    }

    /// Records a capability this crate cannot measure and the table marks with `marker` in its note (for example
    /// `unproven on hardware`), so what is unproven is named in the table and not silently assumed.
    ///
    /// # Panics
    ///
    /// When the cell does not carry `marker`.
    pub fn declared(&mut self, id: &str, marker: &str) {
        let cell = self.table.row(id).cell(self.backend);
        assert!(
            cell.note.contains(marker),
            "`{id}` on {} is not measured here, so its table note must say `{marker}`: `{}`",
            self.backend,
            cell.note
        );
        self.seen.insert(id.to_owned(), cell.status);
    }

    /// Fails unless every row of the table was either measured or declared for this backend.
    ///
    /// # Panics
    ///
    /// Naming the rows nobody measured.
    pub fn finish(self) {
        let missing: Vec<&str> = self.table.ids().filter(|id| !self.seen.contains_key(*id)).collect();
        assert!(
            missing.is_empty(),
            "the status table lists capabilities nothing measures on {}: {missing:?}",
            self.backend
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "intro\n<!-- backend-status:start -->\n\
| Id | Capability | Headless | Nested | DRM |\n| --- | --- | --- | --- | --- |\n\
| `a.b` | Thing | supported | refused: the host decides | n/a: no window |\n\
| `a.c` | Other | supported: virtual | supported | supported: fake kernel |\n\
<!-- backend-status:end -->\ntrailer";

    #[test]
    fn a_table_is_read_with_its_statuses_and_notes() {
        let table = StatusTable::parse(TABLE).expect("a table");
        assert_eq!(table.ids().collect::<Vec<_>>(), ["a.b", "a.c"]);
        let cell = table.row("a.b").cell("nested");
        assert_eq!(cell.status, CellStatus::Refused);
        assert_eq!(cell.note, "the host decides");
        assert_eq!(table.row("a.c").cell("drm").note, "fake kernel");
    }

    #[test]
    fn a_refusal_without_a_reason_is_not_a_cell() {
        let text = TABLE.replace("refused: the host decides", "refused");
        assert!(StatusTable::parse(&text).expect_err("no reason").contains("says why"));
    }

    #[test]
    fn a_word_that_is_no_status_is_named() {
        let text = TABLE.replace("supported: virtual", "maybe");
        assert!(StatusTable::parse(&text).expect_err("no status").contains("maybe"));
    }

    #[test]
    fn a_missing_marker_is_an_error_and_so_is_an_empty_table() {
        assert!(StatusTable::parse("nothing").is_err());
        assert!(StatusTable::parse("<!-- backend-status:start -->\n<!-- backend-status:end -->").is_err());
    }

    #[test]
    fn a_measurement_that_disagrees_with_the_table_fails() {
        let table = StatusTable::parse(TABLE).expect("a table");
        let outcome = std::panic::catch_unwind(|| {
            let mut measured = ColumnCheck::new(&table, "nested");
            measured.is("a.b", CellStatus::Supported);
        });
        assert!(outcome.is_err());
    }

    #[test]
    fn a_row_nobody_measured_fails_the_finish() {
        let table = StatusTable::parse(TABLE).expect("a table");
        let mut measured = ColumnCheck::new(&table, "headless");
        measured.is("a.b", CellStatus::Supported);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measured.finish()));
        assert!(outcome.is_err(), "`a.c` was never measured");
    }
}

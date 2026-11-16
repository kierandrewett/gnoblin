//! The list of diagnostics a role keeps, under a flood.
//!
//! A role reports a diagnostic for each failed reload, listener that threw and refused transaction, and the list a status
//! reader asks for must not grow with them: a shell that runs for weeks and fails the same reload every time the user
//! saves a file would otherwise hold every line it ever said. The list keeps the newest [`DIAGNOSTICS_KEPT`] lines, each
//! cut at [`DIAGNOSTIC_LINE_BYTES`], counts what it let go and says so in one summary line at its head.
//!
//! A reload boots a generation of its own, which costs about a second in a debug build, so these tests lower the list's
//! bound (`JsRoleConfig::diagnostics_kept`) to flood it with a few reloads. The bound's arithmetic at the real size is
//! `diagnostics.rs`'s unit tests'.

use std::fs;

use gnoblin_js::{DIAGNOSTIC_LINE_BYTES, DIAGNOSTICS_KEPT, JsRole, JsRoleLinks};

mod common;
use common::{FakeCompositor, honest_role_config};

/// How many lines the role of these tests keeps.
const KEPT: usize = 4;

/// A role over the shared fake compositor, and the directory of its `init.ts`. A reload of a file that throws reports why
/// it was refused, and the role keeps the running generation.
fn role_keeping_four() -> (JsRole, std::path::PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), "export {};\n").expect("init.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    config.diagnostics_kept = KEPT;
    let role = JsRole::over_link(Box::new(FakeCompositor::new()), JsRoleLinks::default(), config).expect("a role");
    (role, root, dir)
}

/// Rewrites `init.ts` to throw `what` and reloads it: the role keeps the running generation and reports why.
fn refuse_a_reload(role: &mut JsRole, root: &std::path::Path, what: &str) {
    fs::write(root.join("init.ts"), format!("throw new Error(\"{what}\");\n")).expect("init.ts");
    let report = role.reload_now();
    assert!(!report.ok, "a file that throws is refused: {report:?}");
}

#[test]
fn a_flood_of_diagnostics_keeps_the_newest_lines_and_counts_the_rest() {
    let (mut role, root, _dir) = role_keeping_four();
    let before = role.diagnostics_reported();
    let flood = 3 * KEPT;
    for number in 0..flood {
        refuse_a_reload(&mut role, &root, &format!("flood {number}"));
    }

    let reported = role.diagnostics_reported() - before;
    assert!(
        reported >= flood as u64,
        "each reload reported at least one line: {reported}"
    );
    let lines = role.diagnostics();
    assert_eq!(
        lines.len(),
        KEPT + 1,
        "the newest lines and one summary line, however many were reported: {lines:?}"
    );
    assert_eq!(role.diagnostics_dropped(), role.diagnostics_reported() - KEPT as u64);
    assert!(
        lines[0].contains("earlier diagnostics were dropped"),
        "the list says it is not the whole story: {:?}",
        lines[0]
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("earlier diagnostics were dropped"))
            .count(),
        1,
        "one summary line, not one per drop"
    );
    // What is kept is the newest: the last file that was refused is named in the last line.
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(&format!("flood {}", flood - 1))),
        "the newest line is the newest failure: {:?}",
        lines.last()
    );
    assert!(
        lines.iter().all(|line| line.len() <= DIAGNOSTIC_LINE_BYTES),
        "no line is longer than the bound"
    );
}

#[test]
fn what_arrived_since_a_look_is_asked_for_by_count_and_does_not_shift_with_the_drops() {
    let (mut role, root, _dir) = role_keeping_four();
    refuse_a_reload(&mut role, &root, "marker first");
    let mark = role.diagnostics_reported();
    assert!(
        role.diagnostics_since(mark).is_empty(),
        "nothing has arrived since the look"
    );

    refuse_a_reload(&mut role, &root, "marker second");
    let arrived = role.diagnostics_since(mark);
    assert!(
        arrived.iter().any(|line| line.contains("marker second"))
            && arrived.iter().all(|line| !line.contains("marker first")),
        "only what the second reload reported: {arrived:?}"
    );

    // A flood after the look pushes the old lines out of the list, and what is asked for stays what arrived after it.
    for number in 0..KEPT * 2 {
        refuse_a_reload(&mut role, &root, &format!("flood {number}"));
    }
    let later = role.diagnostics_since(mark);
    assert!(later.len() <= KEPT, "never more than the list holds: {}", later.len());
    assert!(
        later
            .iter()
            .all(|line| !line.contains("earlier diagnostics were dropped"))
    );
    assert!(
        later.iter().all(|line| !line.contains("marker first")),
        "the first reload came before the look"
    );
}

//! Exact output of every component, with colour on and off, Unicode and ASCII
//! glyphs, and widths 80 and 40, plus properties that must hold for all of them.

use gnoblin_cli_ui::{
    ColourToken, ErrorReport, LineOutcome, OutputTheme, badge, clear_waiting, definition_list, error_block, event_line,
    identifier, kv_list, section, status_line, strip_ansi, table, waiting_line, width,
};

fn plain(width: usize) -> OutputTheme {
    OutputTheme::new(false, false, width)
}

fn unicode(width: usize) -> OutputTheme {
    OutputTheme::new(false, true, width)
}

fn coloured(width: usize) -> OutputTheme {
    OutputTheme::new(true, true, width)
}

fn rows(data: &[&[&str]]) -> Vec<Vec<String>> {
    data.iter()
        .map(|row| row.iter().map(|cell| cell.to_string()).collect())
        .collect()
}

#[test]
fn table_aligns_columns_under_a_header() {
    let out = table(
        &plain(80),
        &["id", "state", "title"],
        &rows(&[&["12", "focused", "Terminal"], &["7", "normal", "Notes"]]),
    );
    assert_eq!(
        out,
        "id  state    title\n--  -------  --------\n12  focused  Terminal\n7   normal   Notes"
    );
}

#[test]
fn table_counts_wide_characters_as_two_columns() {
    let out = table(&plain(80), &["name", "size"], &rows(&[&["東京", "1"]]));
    assert_eq!(out, "name  size\n----  ----\n東京  1");
}

#[test]
fn table_shrinks_the_widest_column_with_an_ellipsis() {
    let long = "a window title that is far too long for a forty column terminal";
    let out = table(&plain(40), &["id", "title"], &rows(&[&["12", long]]));
    for line in out.lines() {
        assert!(width(line) <= 40, "line too wide: {line:?}");
    }
    let row = out.lines().nth(2).expect("row");
    assert!(row.starts_with("12  "), "{row:?}");
    assert!(row.ends_with("..."), "{row:?}");
}

#[test]
fn kv_list_aligns_keys_and_wraps_values() {
    let out = kv_list(
        &plain(80),
        &[("id", "12"), ("title", "Terminal"), ("geometry", "0,0 1280x720")],
    );
    assert_eq!(out, "id        12\ntitle     Terminal\ngeometry  0,0 1280x720");
}

#[test]
fn status_line_uses_glyph_in_unicode_and_word_in_ascii() {
    assert_eq!(
        status_line(&unicode(80), LineOutcome::Ok, "window 12  focused"),
        "\u{2714} ok  window 12  focused"
    );
    assert_eq!(
        status_line(&plain(80), LineOutcome::Ok, "window 12  focused"),
        "ok  window 12  focused"
    );
    assert_eq!(
        status_line(&plain(80), LineOutcome::Fail, "socket missing"),
        "fail  socket missing"
    );
}

#[test]
fn error_block_labels_each_line() {
    let report = ErrorReport {
        summary: "operation failed".to_string(),
        why: Some("the compositor closed the socket".to_string()),
        hints: vec!["run `gnoblinctl status` to check the connection".to_string()],
    };
    assert_eq!(
        error_block(&plain(80), &report),
        "error: operation failed\n\
         why:   the compositor closed the socket\n\
         hint:  run `gnoblinctl status` to check the connection"
    );
}

#[test]
fn event_line_shows_the_change() {
    assert_eq!(
        event_line(&plain(80), "00:00:01.500", "window.focus", Some(("12", "13"))),
        "00:00:01.500  window.focus  12 -> 13"
    );
    assert_eq!(
        event_line(&plain(80), "00:00:01.500", "window.focus", None),
        "00:00:01.500  window.focus"
    );
}

#[test]
fn section_underlines_the_title() {
    assert_eq!(section(&plain(80), "Windows"), "Windows\n-------");
    assert_eq!(
        section(&unicode(80), "Windows"),
        "Windows\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
}

#[test]
fn definition_list_indents_descriptions() {
    assert_eq!(
        definition_list(&plain(80), &[("list", "Show every window.")]),
        "list\n    Show every window."
    );
}

#[test]
fn waiting_is_plain_text_off_a_terminal() {
    assert_eq!(
        waiting_line(&plain(80), 3, "calling window.focus 12"),
        "calling window.focus 12..."
    );
    assert_eq!(clear_waiting(&plain(80)), "");
}

#[test]
fn colour_wraps_only_the_styled_text() {
    assert_eq!(
        status_line(&coloured(80), LineOutcome::Ok, "window 12"),
        "\x1b[32m\u{2714} ok\x1b[0m  window 12"
    );
    assert_eq!(identifier(&coloured(80), "12"), "\x1b[36m12\x1b[0m");
    assert_eq!(
        badge(&coloured(80), ColourToken::Warning, "stale"),
        "\x1b[33m[stale]\x1b[0m"
    );
}

/// Every sample output, rendered at one theme. Includes wide characters, so
/// callers must not assume ASCII.
fn samples(theme: &OutputTheme) -> Vec<String> {
    let table_rows = rows(&[
        &[
            "12",
            "a window title that is quite long and keeps going well past the edge",
            "x",
        ],
        &["東京都の長いタイトルです", "normal", "y"],
    ]);
    let report = ErrorReport {
        summary: "the compositor did not answer within the timeout of five seconds".to_string(),
        why: Some("it may be busy or the socket belongs to a different session".to_string()),
        hints: vec![
            "run `gnoblinctl status` to see the connection".to_string(),
            "check that XDG_RUNTIME_DIR points at this session".to_string(),
        ],
    };
    vec![
        section(theme, "Windows and their current focus state"),
        table(theme, &["id", "title", "detail"], &table_rows),
        kv_list(
            theme,
            &[(
                "title",
                "a very long value that must wrap across several lines of output",
            )],
        ),
        definition_list(
            theme,
            &[(
                "window list",
                "List every window the compositor knows about, with its focus state and geometry.",
            )],
        ),
        status_line(
            theme,
            LineOutcome::Warn,
            "a long status detail that the terminal width has to wrap",
        ),
        error_block(theme, &report),
        event_line(
            theme,
            "00:00:01.500",
            "window.title",
            Some(("an old title that is long", "a new title that is also long")),
        ),
        waiting_line(theme, 2, "waiting for the compositor"),
        badge(theme, ColourToken::Danger, "failed"),
        identifier(theme, "window.focus"),
    ]
}

#[test]
fn no_line_is_wider_than_the_terminal() {
    for columns in [40, 80] {
        for colour in [false, true] {
            for unicode_glyphs in [false, true] {
                let theme = OutputTheme::new(colour, unicode_glyphs, columns);
                for block in samples(&theme) {
                    for line in block.lines() {
                        assert!(
                            width(line) <= columns,
                            "{columns} colour={colour} unicode={unicode_glyphs}: {line:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn stripping_colour_gives_the_plain_output() {
    for columns in [40, 80] {
        for unicode_glyphs in [false, true] {
            let styled = samples(&OutputTheme::new(true, unicode_glyphs, columns));
            let bare = samples(&OutputTheme::new(false, unicode_glyphs, columns));
            assert_eq!(styled.len(), bare.len());
            for (with, without) in styled.iter().zip(&bare) {
                assert_eq!(strip_ansi(with), *without);
            }
        }
    }
}

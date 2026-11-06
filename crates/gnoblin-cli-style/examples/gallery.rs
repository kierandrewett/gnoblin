//! Prints every component for the current terminal.
//!
//! `cargo run -p gnoblin-cli-style --example gallery` prints to the terminal.
//! `cargo run -p gnoblin-cli-style --example gallery -- --html` prints the same
//! components as an HTML page instead: forced colour, 80 and 40 columns, Unicode
//! and ASCII glyphs. Redirect it to `design/cli-gallery.html` to screenshot it.

use std::fmt::Write as _;

use gnoblin_cli_style::{
    ColorChoice, ColourToken, ErrorReport, LineOutcome, OutputEnvironment, OutputTheme, badge, definition_list,
    error_block, event_line, identifier, kv_list, section, status_line, table, waiting_line,
};

/// The 16 ANSI colours as the HTML page draws them. This is presentation only;
/// in a terminal the user's own theme decides the shades.
const PALETTE: [&str; 16] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5", "#7f7f7f", "#ff0000",
    "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
];

/// Every component, rendered at one theme.
fn blocks(theme: &OutputTheme) -> Vec<(&'static str, String)> {
    let report = ErrorReport {
        summary: "the compositor did not answer within the timeout of five seconds".to_string(),
        why: Some("it may be busy or the socket belongs to a different session".to_string()),
        hints: vec![
            "run `gnoblinctl status` to see the connection".to_string(),
            "check that XDG_RUNTIME_DIR points at this session".to_string(),
        ],
    };
    let windows = vec![
        vec!["12".to_string(), "focused".to_string(), "Terminal".to_string()],
        vec![
            "7".to_string(),
            "normal".to_string(),
            "a long document title that needs cutting".to_string(),
        ],
        vec!["東京".to_string(), "normal".to_string(), "Notes".to_string()],
    ];
    vec![
        ("section", section(theme, "Windows")),
        ("table", table(theme, &["id", "state", "title"], &windows)),
        (
            "key-value list",
            kv_list(
                theme,
                &[
                    ("id", "12"),
                    ("title", "Terminal"),
                    ("geometry", "0,0 1280x720 at scale 1.5"),
                ],
            ),
        ),
        (
            "definition list",
            definition_list(
                theme,
                &[
                    ("window list", "List every window the compositor knows about."),
                    ("window get ID", "Show one window in full."),
                ],
            ),
        ),
        (
            "status lines",
            [
                status_line(theme, LineOutcome::Ok, "window 12  focused"),
                status_line(theme, LineOutcome::Warn, "output DP-2 is disabled"),
                status_line(theme, LineOutcome::Fail, "socket missing"),
                status_line(theme, LineOutcome::Info, "using the default socket"),
            ]
            .join("\n"),
        ),
        ("error block", error_block(theme, &report)),
        (
            "event lines",
            [
                event_line(theme, "00:00:01.500", "window.focus", Some(("7", "12"))),
                event_line(theme, "00:00:02.114", "window.title", None),
            ]
            .join("\n"),
        ),
        (
            "waiting",
            format!(
                "{}\n{}",
                waiting_line(theme, 0, "calling window.focus 12"),
                status_line(theme, LineOutcome::Ok, "window.focus 12 done")
            ),
        ),
        (
            "badges and identifiers",
            format!(
                "{} {} {} {}",
                badge(theme, ColourToken::Success, "live"),
                badge(theme, ColourToken::Warning, "stale"),
                badge(theme, ColourToken::Danger, "failed"),
                identifier(theme, "window.focus")
            ),
        ),
    ]
}

/// Converts the 16-colour SGR sequences in `text` to HTML spans. Other escape
/// sequences are dropped.
fn ansi_to_html(text: &str) -> String {
    let mut out = String::new();
    let mut fg: Option<u8> = None;
    let mut bold = false;
    let mut rest = text;
    while !rest.is_empty() {
        let (chunk, tail) = match rest.find('\x1b') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, ""),
        };
        if !chunk.is_empty() {
            let mut classes = Vec::new();
            if bold {
                classes.push("b".to_string());
            }
            if let Some(colour) = fg {
                classes.push(format!("c{colour}"));
            }
            let escaped = escape_html(chunk);
            if classes.is_empty() {
                out.push_str(&escaped);
            } else {
                let _ = write!(out, "<span class=\"{}\">{escaped}</span>", classes.join(" "));
            }
        }
        if tail.is_empty() {
            break;
        }
        let Some(body) = tail.strip_prefix("\x1b[") else {
            rest = &tail[1..];
            continue;
        };
        let Some(end) = body.find('m') else {
            break;
        };
        for code in body[..end].split(';') {
            let value = code.parse::<u8>().unwrap_or(0);
            match value {
                0 => {
                    fg = None;
                    bold = false;
                }
                1 => bold = true,
                22 => bold = false,
                30..=37 => fg = Some(value - 30),
                39 => fg = None,
                90..=97 => fg = Some(value - 90 + 8),
                _ => {}
            }
        }
        rest = &body[end + 1..];
    }
    out
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The HTML page: one heading and one preformatted block per section.
fn html_page(sections: &[(String, String)]) -> String {
    let mut css = String::from(
        "body{background:#1e1e1e;color:#d4d4d4;font-family:monospace;margin:24px}\
         h1{font-family:sans-serif;font-weight:normal}\
         h2{font-family:sans-serif;font-weight:normal;color:#9cdcfe;margin-top:32px}\
         pre{font-size:14px;line-height:1.35;margin:0}\
         .b{font-weight:bold}",
    );
    for (index, colour) in PALETTE.iter().enumerate() {
        let _ = write!(css, ".c{index}{{color:{colour}}}");
    }
    let mut body = String::new();
    for (title, text) in sections {
        let _ = write!(
            body,
            "<h2>{}</h2>\n<pre>{}</pre>\n",
            escape_html(title),
            ansi_to_html(text)
        );
    }
    format!(
        "<!doctype html>\n<html lang=\"en-GB\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>gnoblin-cli-style gallery</title>\n<style>{css}</style>\n</head>\n\
         <body>\n<h1>gnoblin-cli-style gallery</h1>\n{body}</body>\n</html>\n"
    )
}

/// Every component at 80 and 40 columns, with Unicode and ASCII glyphs.
fn html_sections() -> Vec<(String, String)> {
    let mut sections = Vec::new();
    for (columns, unicode) in [(80, true), (40, true), (80, false), (40, false)] {
        let theme = OutputTheme::new(true, unicode, columns);
        let glyphs = if unicode { "Unicode" } else { "ASCII" };
        for (name, block) in blocks(&theme) {
            sections.push((format!("{name}: {columns} columns, {glyphs}"), block));
        }
    }
    sections
}

fn main() {
    if std::env::args().skip(1).any(|arg| arg == "--html") {
        print!("{}", html_page(&html_sections()));
        return;
    }
    let theme = OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect_stdout());
    for (_, block) in blocks(&theme) {
        println!("{block}\n");
    }
}

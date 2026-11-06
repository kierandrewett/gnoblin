//! Colour, glyph and width decisions shared by every Gnoblin command-line tool.
//!
//! An `OutputTheme` is resolved once per process from the `--color` flag and the
//! environment. Components take a `&OutputTheme` and never read the environment
//! themselves, so tests can build any combination with `OutputTheme::new`.

use std::io::IsTerminal;

/// Width used when the terminal does not report its size.
pub const DEFAULT_WIDTH: usize = 80;
/// Narrowest layout any component produces; narrower terminals get this.
pub const MIN_WIDTH: usize = 40;

/// The value of `--color`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorChoice {
    /// Colour when the stream is a terminal (or `CLICOLOR_FORCE` is set).
    Auto,
    /// Colour even when the stream is not a terminal.
    Always,
    /// Never colour.
    Never,
}

impl ColorChoice {
    /// Parses the value of `--color`: `auto`, `always` or `never`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// The facts about the process that decide how output looks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputEnvironment {
    /// Whether the stream the output goes to is a terminal.
    pub is_terminal: bool,
    /// `NO_COLOR` is set to a non-empty value.
    pub no_color: bool,
    /// `CLICOLOR_FORCE` is set and is not `0`.
    pub clicolor_force: bool,
    /// The value of `TERM`, when set.
    pub term: Option<String>,
    /// The first of `LC_ALL`, `LC_CTYPE` and `LANG` that is non-empty.
    pub locale: Option<String>,
    /// The value of `COLUMNS`, when it is a number.
    pub columns: Option<usize>,
}

impl OutputEnvironment {
    /// Reads this process's environment. `is_terminal` says whether the stream
    /// the output goes to is a terminal.
    pub fn detect(is_terminal: bool) -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            is_terminal,
            no_color: var("NO_COLOR").is_some(),
            clicolor_force: var("CLICOLOR_FORCE").is_some_and(|value| value != "0"),
            term: var("TERM"),
            locale: ["LC_ALL", "LC_CTYPE", "LANG"].into_iter().find_map(var),
            columns: var("COLUMNS").and_then(|value| value.parse().ok()),
        }
    }

    /// Reads this process's environment for standard output.
    pub fn detect_stdout() -> Self {
        Self::detect(std::io::stdout().is_terminal())
    }
}

/// A semantic colour. Each maps to one of the 16 ANSI colours, so the user's
/// terminal theme decides the actual shade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColourToken {
    /// Identifiers and the names of things: window ids, property names.
    Accent,
    /// Good news: an operation finished, a value is as wanted.
    Success,
    /// Something to look at, but not a failure.
    Warning,
    /// A failure.
    Danger,
    /// Neutral information.
    Info,
    /// Secondary text: timestamps, rules, labels.
    Muted,
    /// Headings and column headers.
    Strong,
}

impl ColourToken {
    /// The SGR parameter for this colour.
    fn code(self) -> &'static str {
        match self {
            Self::Accent => "36",
            Self::Success => "32",
            Self::Warning => "33",
            Self::Danger => "31",
            Self::Info => "34",
            Self::Muted => "90",
            Self::Strong => "1",
        }
    }
}

/// A symbol with a Unicode form and an ASCII fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Glyph {
    /// Success.
    Ok,
    /// Warning.
    Warn,
    /// Failure.
    Fail,
    /// Information.
    Info,
    /// A list item.
    Bullet,
    /// A change from one value to another.
    Arrow,
    /// Text cut short.
    Ellipsis,
    /// A horizontal rule under a header.
    Rule,
}

impl Glyph {
    /// The Unicode form and the ASCII fallback.
    fn pair(self) -> (&'static str, &'static str) {
        match self {
            Self::Ok => ("\u{2714}", "ok"),
            Self::Warn => ("\u{26a0}", "warn"),
            Self::Fail => ("\u{2718}", "fail"),
            Self::Info => ("\u{2139}", "info"),
            Self::Bullet => ("\u{2022}", "*"),
            Self::Arrow => ("\u{2192}", "->"),
            Self::Ellipsis => ("\u{2026}", "..."),
            Self::Rule => ("\u{2500}", "-"),
        }
    }
}

/// The spinner frames for a waiting operation.
const SPINNER_UNICODE: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}",
    "\u{280f}",
];
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

/// What the output looks like: colour, Unicode glyphs, animation and width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputTheme {
    colour: bool,
    unicode: bool,
    animate: bool,
    width: usize,
}

impl OutputTheme {
    /// Decides the theme from `--color` and the environment.
    ///
    /// Colour is off when the output is `TERM=dumb`, when `NO_COLOR` is set
    /// (unless `--color=always`), and when the stream is not a terminal (unless
    /// `--color=always` or `CLICOLOR_FORCE` is set). Unicode glyphs need a UTF-8
    /// locale, a stream that is a terminal (or `--color=always`), and a
    /// `TERM` other than `dumb`. Animation needs a terminal.
    pub fn resolve(choice: ColorChoice, env: &OutputEnvironment) -> Self {
        let dumb = env.term.as_deref() == Some("dumb");
        let colour = !dumb
            && match choice {
                ColorChoice::Always => true,
                ColorChoice::Never => false,
                ColorChoice::Auto => !env.no_color && (env.clicolor_force || env.is_terminal),
            };
        let utf8 = env.locale.as_deref().is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        });
        let on_stream = env.is_terminal || choice == ColorChoice::Always;
        Self {
            colour,
            unicode: !dumb && on_stream && utf8,
            animate: !dumb && env.is_terminal && choice != ColorChoice::Never,
            width: env.columns.unwrap_or(DEFAULT_WIDTH).max(MIN_WIDTH),
        }
    }

    /// A theme with the given choices and no animation. The width is raised to
    /// `MIN_WIDTH` when smaller.
    pub const fn new(colour: bool, unicode: bool, width: usize) -> Self {
        Self {
            colour,
            unicode,
            animate: false,
            width: if width < MIN_WIDTH { MIN_WIDTH } else { width },
        }
    }

    /// Whether colour escapes are written.
    pub fn colour(&self) -> bool {
        self.colour
    }

    /// Whether Unicode glyphs are written instead of ASCII fallbacks.
    pub fn unicode(&self) -> bool {
        self.unicode
    }

    /// Whether waiting operations animate (a spinner and carriage returns).
    pub fn animate(&self) -> bool {
        self.animate
    }

    /// The number of columns output may use.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Wraps `text` in the colour for `token`, or returns it unchanged when
    /// colour is off.
    pub fn paint(&self, token: ColourToken, text: &str) -> String {
        if !self.colour || text.is_empty() {
            return text.to_string();
        }
        format!("\x1b[{}m{text}\x1b[0m", token.code())
    }

    /// The glyph in the form this theme uses.
    pub fn glyph(&self, glyph: Glyph) -> &'static str {
        let (unicode, ascii) = glyph.pair();
        if self.unicode { unicode } else { ascii }
    }

    /// The spinner frame for `tick`.
    pub fn spinner(&self, tick: usize) -> &'static str {
        if self.unicode {
            SPINNER_UNICODE[tick % SPINNER_UNICODE.len()]
        } else {
            SPINNER_ASCII[tick % SPINNER_ASCII.len()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(is_terminal: bool) -> OutputEnvironment {
        OutputEnvironment {
            is_terminal,
            locale: Some("en_GB.UTF-8".to_string()),
            ..OutputEnvironment::default()
        }
    }

    #[test]
    fn colour_follows_the_stream_under_auto() {
        assert!(!OutputTheme::resolve(ColorChoice::Auto, &env(false)).colour());
        assert!(OutputTheme::resolve(ColorChoice::Auto, &env(true)).colour());
    }

    #[test]
    fn no_color_and_dumb_terminals_turn_colour_off() {
        let mut no_color = env(true);
        no_color.no_color = true;
        assert!(!OutputTheme::resolve(ColorChoice::Auto, &no_color).colour());
        let mut dumb = env(true);
        dumb.term = Some("dumb".to_string());
        let theme = OutputTheme::resolve(ColorChoice::Always, &dumb);
        assert!(!theme.colour());
        assert!(!theme.unicode());
        assert!(!theme.animate());
    }

    #[test]
    fn always_forces_colour_and_unicode_through_a_pipe() {
        let theme = OutputTheme::resolve(ColorChoice::Always, &env(false));
        assert!(theme.colour());
        assert!(theme.unicode());
        assert!(!theme.animate());
    }

    #[test]
    fn ascii_locale_gets_ascii_glyphs() {
        let mut ascii = env(true);
        ascii.locale = Some("C".to_string());
        let theme = OutputTheme::resolve(ColorChoice::Auto, &ascii);
        assert_eq!(theme.glyph(Glyph::Ellipsis), "...");
    }

    #[test]
    fn width_defaults_and_has_a_floor() {
        assert_eq!(OutputTheme::resolve(ColorChoice::Auto, &env(false)).width(), 80);
        let mut narrow = env(false);
        narrow.columns = Some(20);
        assert_eq!(OutputTheme::resolve(ColorChoice::Auto, &narrow).width(), 40);
        assert_eq!(OutputTheme::new(false, false, 10).width(), 40);
    }

    #[test]
    fn paint_is_identity_without_colour() {
        assert_eq!(OutputTheme::new(false, false, 80).paint(ColourToken::Danger, "x"), "x");
        assert_eq!(
            OutputTheme::new(true, false, 80).paint(ColourToken::Danger, "x"),
            "\x1b[31mx\x1b[0m"
        );
    }

    #[test]
    fn colour_choice_parses_three_values() {
        assert_eq!(ColorChoice::parse("always"), Some(ColorChoice::Always));
        assert_eq!(ColorChoice::parse("sometimes"), None);
    }
}

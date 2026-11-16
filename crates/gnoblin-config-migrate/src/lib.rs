//! Converts a Lua `init.lua` to a TypeScript `shell.ts`.
//!
//! `gnoblinctl config migrate` is the user of this crate. The conversion is a pure function of the text of the Lua
//! files: nothing here reads a file, runs Lua or looks at the environment. A file that a `gnoblin.load` names is
//! fetched through a [`ConfigSourceLoader`], which the command implements over the directory and a test implements
//! over a map.
//!
//! # What the output is
//!
//! Statements against the specification's objects, in the style of the owner's reference `shell.js`, not a config
//! object: `new Workspace({ id, name, layout })` and `gnoblin.workspaces.add` for the `workspaces`,
//! `gnoblin.input.primary.bind(trigger, () => gnoblin.spawn([...]))` for the `shortcuts` and `keybindings`, and for a
//! window rule of a shell file a `gnoblin.clients.where({ appId }).style` (opacity on an exact match) or a
//! `CollectionListener` (workspace, maximised, fullscreen and opacity, on an app id or a title). A Lua pattern in a rule
//! becomes a comparison or a RegExp when it has an exact equivalent ([`translate_lua_pattern`]); a chord becomes a
//! trigger ([`translate_chord`]). The seat's `alerts` switch decides whether the not-responding alert is started.
//!
//! A `shell.ts` replaces the stock default instead of layering over it, so the output also writes the stock desktop
//! out, minus the keys and workspaces the Lua config set itself.
//!
//! # What is reported
//!
//! Everything else. A statement the converter cannot evaluate, and every setting of a section the specification has no
//! home for (`autostart`, `touchpad-gestures`, `workspace_sets`, the `edge_tiling` and `drag_constraint` of the seat,
//! and the settings of the document's other sections), is left out of the TypeScript and written there as an
//! `UNMAPPED (file:line)` comment with its source text, and is listed in the [`MigrationReport`] with its line, why and
//! where the migration page says what to do. A setting with no home says what a home would need. The first line of the
//! output is `// INCOMPLETE: N unmapped settings, see comments` whenever N is not zero. A thing that converts to
//! something the shell would refuse (a duplicate trigger, a rule for a workspace the config does not declare) is left
//! out the same way, so the output loads.
//!
//! # Failure modes first
//!
//! 1. **A construct is dropped without a word.** Every path that does not convert calls the one reporting function,
//!    which records the item and returns the comment that marks its place; `tests/convert.rs`
//!    `every_unmapped_construct_is_in_the_output_and_the_report` counts both.
//! 2. **A rule matches more than the author wrote.** A match key or pattern with no exact equivalent leaves the whole
//!    rule out; it is never loosened (`a_rule_with_a_pattern_that_has_no_equivalent_is_left_out_whole`).
//! 3. **A key is bound twice.** `input.bind` refuses a second binding of one trigger, so the stock keys the config
//!    rebinds are not written, and a trigger the config binds twice is left out and reported.
//! 4. **The output does not load.** A duplicate trigger, a rule that names a workspace the config does not declare,
//!    and a rule with no query or no action are each left out and reported; the `gnoblinctl` tests load the converted
//!    starter and a config of every construct in the shell's dry-run host.
//! 5. **A glob loads files the Lua host would not.** The command expands `gnoblin.load` against the directory of the
//!    file that wrote it, sorted, as the Lua host did ([`resolve_load_path`], [`glob_matches`]).

mod chord;
mod emit;
mod engine;
mod lua;
mod names;
mod pattern;

pub use chord::{ChordTranslation, translate_chord};
pub use pattern::{PatternTranslation, translate_lua_pattern};

/// One Lua file: the name the user sees for it (a path relative to the config directory) and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateSource {
    /// The name in comments and reports, such as `config/10-windows.lua`.
    pub name: String,
    /// The text of the file.
    pub text: String,
}

/// Where the files named by `gnoblin.load` come from.
pub trait ConfigSourceLoader {
    /// The files that `pattern` names when `from` wrote it, in load order. `from` is the name of the file that called
    /// `gnoblin.load`; a relative `pattern` is relative to that file's directory. A pattern that names no file is an
    /// empty list, as it is in Lua. An explicit path that does not exist is an error with the message to show.
    fn expand(&self, from: &str, pattern: &str) -> Result<Vec<MigrateSource>, String>;
}

/// How much of the config was converted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConvertedCounts {
    /// Workspaces, now `gnoblin.workspaces.add` statements.
    pub workspaces: usize,
    /// Shortcuts, now `input.bind` statements.
    pub keys: usize,
    /// Window rules that became a style or a listener.
    pub rules: usize,
    /// The seat's `alerts` switch, which decides whether the not-responding alert is started.
    pub options: usize,
}

/// One construct the converter left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnmappedItem {
    /// The file, as named in the sources.
    pub file: String,
    /// The line it starts on, from 1.
    pub line: usize,
    /// Its first line of source, cut to fit a report line.
    pub summary: String,
    /// Why it was left out, in a sentence.
    pub reason: String,
    /// Where in the migration page to read what to do, as `docs/migrating-from-lua.md#anchor`.
    pub pointer: &'static str,
}

/// What a conversion did, besides the TypeScript.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// The files read, in load order.
    pub sources: Vec<String>,
    /// How much was converted.
    pub converted: ConvertedCounts,
    /// What was left out, in the order it was met.
    pub unmapped: Vec<UnmappedItem>,
    /// Things the author may want to know that are not a loss: a disabled shortcut left out, a glob with no match.
    pub notes: Vec<String>,
}

/// The TypeScript and the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationOutput {
    /// The text of `shell.ts`.
    pub typescript: String,
    /// What was converted and what was not.
    pub report: MigrationReport,
}

/// Why nothing was converted.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    /// The entry file is not Lua.
    #[error("{file}:{line}: this does not parse as Lua: {message}")]
    Syntax {
        /// The file.
        file: String,
        /// The line the parser gave up on.
        line: usize,
        /// What the parser said.
        message: String,
    },
}

/// Converts `entry` and everything it loads, then `followers` (the shell files) in order.
///
/// A syntax error in `entry` is an error. One in any other file is one left-out construct, so the rest converts.
///
/// # Errors
///
/// [`MigrateError::Syntax`] when `entry` does not parse as Lua.
pub fn migrate(
    entry: &MigrateSource,
    followers: &[MigrateSource],
    loader: &dyn ConfigSourceLoader,
) -> Result<MigrationOutput, MigrateError> {
    let mut engine = engine::Engine::new(loader);
    engine.run_source(entry, true)?;
    for follower in followers {
        engine.run_source(follower, false)?;
    }
    Ok(engine.finish(&entry.name))
}

/// Whether `pattern` names more than one file: it has a `*` or a `?`.
#[must_use]
pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// The path a `gnoblin.load(pattern)` in the file `from` means, relative to the config directory: the pattern joined
/// to the directory of `from`, with `.` and `..` resolved. An absolute pattern is returned as it is.
#[must_use]
pub fn resolve_load_path(from: &str, pattern: &str) -> String {
    if pattern.starts_with('/') {
        return pattern.to_owned();
    }
    let directory = from.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut parts: Vec<&str> = Vec::new();
    for part in directory.split('/').chain(pattern.split('/')) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Whether `path` matches `pattern`: `/`-separated segments, `*` any run of characters and `?` one character within a
/// segment, and a `**` segment any number of segments, none included.
#[must_use]
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments_match(&pattern, &path)
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments_match(rest, &path[skip..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(head, tail)| wildcard_match(first, head) && segments_match(rest, tail)),
    }
}

/// `*` and `?` within one path segment.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star, matched)) = backtrack {
            p = star + 1;
            t = matched + 1;
            backtrack = Some((star, matched + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_path_is_relative_to_the_file_that_wrote_it() {
        assert_eq!(resolve_load_path("init.lua", "config/*.lua"), "config/*.lua");
        assert_eq!(
            resolve_load_path("config/10-a.lua", "parts/x.lua"),
            "config/parts/x.lua"
        );
        assert_eq!(resolve_load_path("config/10-a.lua", "../extra.lua"), "extra.lua");
        assert_eq!(resolve_load_path("config/10-a.lua", "./x.lua"), "config/x.lua");
        assert_eq!(
            resolve_load_path("init.lua", "/etc/gnoblin/x.lua"),
            "/etc/gnoblin/x.lua"
        );
    }

    #[test]
    fn a_glob_matches_by_segment() {
        assert!(glob_matches("config/*.lua", "config/10-windows.lua"));
        assert!(!glob_matches("config/*.lua", "config/sub/10.lua"));
        assert!(!glob_matches("config/*.lua", "config/readme.md"));
        assert!(glob_matches("conf.d/**/*.lua", "conf.d/a.lua"));
        assert!(glob_matches("conf.d/**/*.lua", "conf.d/x/y/a.lua"));
        assert!(glob_matches("a?c.lua", "abc.lua"));
        assert!(!glob_matches("a?c.lua", "ac.lua"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*-x*", "a-x-b"));
        assert!(glob_matches("exact.lua", "exact.lua"));
        assert!(!glob_matches("exact.lua", "other.lua"));
    }

    #[test]
    fn only_a_star_or_a_question_mark_makes_a_glob() {
        assert!(is_glob("config/*.lua"));
        assert!(is_glob("a?.lua"));
        assert!(!is_glob("config/a.lua"));
    }
}

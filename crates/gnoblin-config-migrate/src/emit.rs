//! The TypeScript the converter writes.
//!
//! Statements against the specification's own objects, in the style of the owner's reference `shell.js`: workspaces are
//! `new Workspace({ id, name, layout })` and `gnoblin.workspaces.add`, a shortcut is `input.bind(trigger, () =>
//! gnoblin.spawn([...]))`, a rule that only styles windows of an exact app id is `gnoblin.clients.where({ appId
//! }).style`, and every other simple rule is a `CollectionListener`. Nothing here reads Lua: the engine hands over what
//! it found, and this module prints it.
//!
//! A `shell.ts` replaces the stock default instead of layering over it, so [`stock`] writes the stock desktop out in
//! full, minus what the Lua config set itself. That is what keeps a migrated config the desktop the Lua shell was.

use std::collections::BTreeSet;

use crate::chord::canonical_trigger;
use crate::names;
use crate::pattern::PatternTranslation;

/// The widest an `import` is written on one line.
const IMPORT_WIDTH: usize = 120;

/// What the file needs from the `gnoblin-core` and `gnoblin-std` modules, and whether it binds on the seat.
#[derive(Default)]
pub(crate) struct Needs {
    names: BTreeSet<&'static str>,
    client_type: bool,
    input: bool,
}

impl Needs {
    fn import(&mut self, name: &'static str) {
        self.names.insert(name);
    }
}

/// A thing of a group, or the comment that stands where one was left out.
pub(crate) enum Placed<T> {
    Def(T),
    Note(Vec<String>),
}

/// One workspace the Lua config declared.
pub(crate) struct WorkspaceDef {
    pub id: String,
    pub name: Option<String>,
    /// The comments for the fields of the workspace that were left out, printed after it.
    pub notes: Vec<Vec<String>>,
}

/// One shortcut that converts: its trigger and the program to start.
pub(crate) struct KeyDef {
    pub trigger: String,
    pub argv: Vec<String>,
}

/// How a rule compares one property of a window with what the Lua pattern said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Test {
    Equals(String),
    StartsWith(String),
    EndsWith(String),
    Includes(String),
    RegExp { source: String, flags: String },
}

impl Test {
    /// The test a translated pattern is, or `None` when the pattern matches every value and so tests nothing.
    pub(crate) fn of(translation: &PatternTranslation) -> Option<Test> {
        match translation {
            PatternTranslation::Glob(glob) if glob == "*" => None,
            // A glob the translator writes has a star only at an end, and never as a literal character.
            PatternTranslation::Glob(glob) if !glob.contains('*') => Some(Test::Equals(glob.clone())),
            PatternTranslation::Glob(glob) => {
                let inner = glob.trim_start_matches('*').trim_end_matches('*').to_owned();
                Some(match (glob.starts_with('*'), glob.ends_with('*')) {
                    (true, true) => Test::Includes(inner),
                    (true, false) => Test::EndsWith(inner),
                    _ => Test::StartsWith(inner),
                })
            }
            PatternTranslation::RegExp { source, flags } => Some(Test::RegExp {
                source: source.clone(),
                flags: flags.clone(),
            }),
            PatternTranslation::Unmapped(_) => None,
        }
    }

    /// The condition on `client.<property>`.
    fn condition(&self, property: &str) -> String {
        match self {
            Test::Equals(text) => format!("client.{property} === {}", names::string(text)),
            Test::StartsWith(text) => format!("client.{property}.startsWith({})", names::string(text)),
            Test::EndsWith(text) => format!("client.{property}.endsWith({})", names::string(text)),
            Test::Includes(text) => format!("client.{property}.includes({})", names::string(text)),
            Test::RegExp { source, flags } => format!("/{source}/{flags}.test(client.{property})"),
        }
    }
}

/// One window rule that converts.
pub(crate) struct RuleDef {
    /// Where the rule stood, as `shell.lua:12`.
    pub origin: String,
    /// The properties of the window the rule matches and how: `appId` and `title`.
    pub conditions: Vec<(&'static str, Test)>,
    pub workspace: Option<String>,
    pub maximised: Option<bool>,
    pub fullscreen: Option<bool>,
    pub opacity: Option<f64>,
    /// The comments for the fields of the rule that were left out, printed with it.
    pub notes: Vec<Vec<String>>,
}

impl RuleDef {
    /// Whether a `where()` selection can be the rule's whole query: every condition is an exact match.
    fn selects_exactly(&self) -> bool {
        !self.conditions.is_empty() && self.conditions.iter().all(|(_, test)| matches!(test, Test::Equals(_)))
    }
}

/// The lines of a comment: an empty line stands for a blank comment line.
pub(crate) fn comment(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            if line.is_empty() {
                "//".to_owned()
            } else {
                format!("// {line}")
            }
        })
        .collect()
}

/// The statements that declare `items`, one `gnoblin.workspaces.add` each, in the order the Lua config listed them.
pub(crate) fn workspaces(items: &[Placed<WorkspaceDef>], needs: &mut Needs) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Placed::Note(lines) => out.extend(comment(lines)),
            Placed::Def(def) => {
                needs.import("Workspace");
                needs.import("FloatingLayout");
                let name = def
                    .name
                    .as_ref()
                    .map(|name| format!(", name: {}", names::string(name)))
                    .unwrap_or_default();
                out.push(format!(
                    "gnoblin.workspaces.add(new Workspace({{ id: {}{name}, layout: new FloatingLayout() }}));",
                    names::string(&def.id)
                ));
                for note in &def.notes {
                    out.extend(comment(note));
                }
            }
        }
    }
    out
}

/// The program of a shortcut as the argument of `gnoblin.spawn`.
fn argv(argv: &[String]) -> String {
    let parts: Vec<String> = argv.iter().map(|argument| names::string(argument)).collect();
    format!("[{}]", parts.join(", "))
}

/// The statements that bind `items` on the primary seat.
pub(crate) fn keys(items: &[Placed<KeyDef>], needs: &mut Needs) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Placed::Note(lines) => out.extend(comment(lines)),
            Placed::Def(def) => {
                needs.input = true;
                out.push(format!("input.bind({}, () => {{", names::string(&def.trigger)));
                out.push(format!("    gnoblin.spawn({});", argv(&def.argv)));
                out.push("});".to_owned());
            }
        }
    }
    out
}

/// The statements a rule runs on a window it matches, without the opacity when a `where()` selection carries it.
fn actions(def: &RuleDef, with_opacity: bool) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(id) = &def.workspace {
        out.push(format!(
            "gnoblin.workspaces.get({})?.clients.add(client);",
            names::string(id)
        ));
    }
    if let Some(on) = def.maximised {
        out.push(format!("client.maximised = {on};"));
    }
    if let Some(on) = def.fullscreen {
        out.push(format!("client.fullscreen = {on};"));
    }
    if with_opacity && let Some(opacity) = def.opacity {
        out.push(format!("client.style.opacity = {opacity};"));
    }
    out
}

/// The statements for the rules that convert, and the notes between them.
///
/// A rule that only sets opacity, on windows of an exact app id and title, is a `where()` selection's style: the shell
/// keeps it up to date as windows come and go. That is only used when every opacity rule can be one, because a
/// selection's style and a window's own style are two layers and a later rule must win over an earlier one in both.
/// Everything else runs in one `CollectionListener`, in the order the Lua listed the rules.
pub(crate) fn rules(items: &[Placed<RuleDef>], needs: &mut Needs) -> Vec<String> {
    let defs: Vec<&RuleDef> = items
        .iter()
        .filter_map(|item| match item {
            Placed::Def(def) => Some(def),
            Placed::Note(_) => None,
        })
        .collect();
    let selections = defs
        .iter()
        .filter(|def| def.opacity.is_some())
        .all(|def| def.selects_exactly());
    let mut out: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for item in items {
        let def = match item {
            Placed::Note(lines) => {
                // A note stands between the statements it was met between.
                out.extend(comment(lines));
                continue;
            }
            Placed::Def(def) => def,
        };
        let mut noted = false;
        if selections && let Some(opacity) = def.opacity {
            let query: Vec<String> = def
                .conditions
                .iter()
                .filter_map(|(property, test)| match test {
                    Test::Equals(text) => Some(format!("{property}: {}", names::string(text))),
                    _ => None,
                })
                .collect();
            out.push(format!("// {}", def.origin));
            out.extend(def.notes.iter().flat_map(|note| comment(note)));
            noted = true;
            out.push(format!(
                "gnoblin.clients.where({{ {} }}).style.opacity = {opacity};",
                query.join(", ")
            ));
        }
        let statements = actions(def, !selections);
        if statements.is_empty() {
            continue;
        }
        blocks.push(format!("            // {}", def.origin));
        if !noted {
            blocks.extend(
                def.notes
                    .iter()
                    .flat_map(|note| comment(note))
                    .map(|line| format!("            {line}")),
            );
        }
        let conditions: Vec<String> = def
            .conditions
            .iter()
            .map(|(property, test)| test.condition(property))
            .collect();
        if conditions.is_empty() {
            blocks.extend(statements.iter().map(|line| format!("            {line}")));
        } else {
            blocks.push(format!("            if ({}) {{", conditions.join(" && ")));
            blocks.extend(statements.iter().map(|line| format!("                {line}")));
            blocks.push("            }".to_owned());
        }
    }
    if !blocks.is_empty() {
        needs.import("CollectionListener");
        needs.client_type = true;
        out.push(
            "// Each rule below runs when a window is added. A Lua rule also ran again when the window's title or its focus changed."
                .to_owned(),
        );
        out.push("gnoblin.clients.addListener(".to_owned());
        out.push("    new (class extends CollectionListener {".to_owned());
        out.push("        onAdded(client: Client): void {".to_owned());
        out.extend(blocks);
        out.push("        }".to_owned());
        out.push("    })(),".to_owned());
        out.push(");".to_owned());
    }
    out
}

/// The stock desktop's one description: the starter, `shells/default/shell.ts`.
const STARTER: &str = include_str!("../../../shells/default/shell.ts");

/// The first line of the starter's statement that binds `Super+1` to `Super+9`.
const POSITION_LOOP: &str = "for (let position = 1; position <= 9; position++) {";

/// The starter, split for [`stock`]: the names it imports, and its blocks of statements. A block ends at a blank line
/// and keeps the comment above it. A block of comments only is an example the starter offers, and `const input` is the
/// document's own line, so neither is a block.
struct Starter {
    names: Vec<&'static str>,
    blocks: Vec<Vec<&'static str>>,
}

impl Starter {
    fn read() -> Starter {
        let lines: Vec<&'static str> = STARTER.lines().collect();
        let open = lines
            .iter()
            .position(|line| *line == "import {")
            .expect("the starter imports the library");
        let close = lines
            .iter()
            .position(|line| line.starts_with("} from "))
            .expect("the starter's import ends");
        let names = lines[open + 1..close]
            .iter()
            .map(|line| line.trim().trim_end_matches(','))
            .collect();
        let blocks = lines[close + 1..]
            .split(|line| line.is_empty())
            .filter(|block| block.iter().any(|line| !line.starts_with("//")))
            .filter(|block| !block.iter().any(|line| line.starts_with("const input =")))
            .map(<[&'static str]>::to_vec)
            .collect();
        Starter { names, blocks }
    }
}

/// The stock desktop: what `shells/default/shell.ts` does, written out for a config that replaces it.
///
/// The text is the starter's own, so it cannot drift from it. Each of its blocks is kept whole except for what the Lua
/// config said: `own_workspaces` (the Lua config declared workspaces, which then stand where the stock four would) drops
/// the workspaces; `alerts` false turns the not-responding alert into a comment; `taken` holds the canonical triggers
/// the Lua config binds itself, and a stock key on one of them is left out, as the Lua config's key took its place when
/// the two were layered. A block whose keys are all left out goes with its comment and helpers. The names the kept
/// statements use are the ones the file imports.
///
/// A test (`crates/gnoblinctl/tests/config_migrate.rs`) boots the converted Lua starter and the starter and compares the
/// worlds.
pub(crate) fn stock(needs: &mut Needs, own_workspaces: bool, alerts: bool, taken: &BTreeSet<String>) -> Vec<String> {
    let free = |trigger: &str| !taken.contains(&canonical_trigger(trigger));
    let starter = Starter::read();
    let mut blocks: Vec<Vec<String>> = vec![comment(&[
        "The stock desktop. A shell.ts replaces the stock default instead of layering over it, so what the Lua shell"
            .to_owned(),
        "kept on its own is written out here. Delete a statement and that part of the desktop is gone.".to_owned(),
    ])];
    blocks.extend(
        starter
            .blocks
            .iter()
            .filter_map(|block| kept(block, own_workspaces, alerts, &free)),
    );
    for line in blocks.iter().skip(1).flatten().filter(|line| !line.starts_with("//")) {
        for &name in &starter.names {
            if mentions(line, name) {
                needs.import(name);
            }
        }
        needs.input |= line.contains("input.");
    }
    join(blocks)
}

/// One block of the starter, less what the config says; `None` when nothing is left of it. A key is a literal
/// `input.bind("Trigger", ...)` statement, a media entry `Name: [...]` and the loop over `Super+1` to `Super+9`.
fn kept(block: &[&str], own_workspaces: bool, alerts: bool, free: &dyn Fn(&str) -> bool) -> Option<Vec<String>> {
    if own_workspaces
        && block
            .iter()
            .any(|line| line.trim_start().starts_with("gnoblin.workspaces.add("))
    {
        return None;
    }
    let (mut seen, mut left) = (0, 0);
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while at < block.len() {
        let line = block[at];
        at += 1;
        if let Some(trigger) = line
            .strip_prefix("input.bind(\"")
            .and_then(|rest| rest.split('"').next())
        {
            seen += 1;
            let end = statement_end(block, at - 1);
            if free(trigger) {
                left += 1;
                out.extend(block[at - 1..end].iter().map(|statement| (*statement).to_owned()));
            }
            at = end;
        } else if let Some(key) = line
            .strip_prefix("    ")
            .and_then(|rest| rest.split_once(": ["))
            .map(|(key, _)| key)
            .filter(|key| key.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            seen += 1;
            if free(key) {
                left += 1;
                out.push(line.to_owned());
            }
        } else if line == POSITION_LOOP {
            seen += 1;
            let positions: Vec<usize> = (1..=9).filter(|position| free(&format!("Super+{position}"))).collect();
            if positions.is_empty() {
                at = statement_end(block, at - 1);
            } else {
                left += 1;
                out.push(if positions.len() == 9 {
                    line.to_owned()
                } else {
                    let list: Vec<String> = positions.iter().map(ToString::to_string).collect();
                    format!("for (const position of [{}]) {{", list.join(", "))
                });
            }
        } else if line == "startNotRespondingAlert();" && !alerts {
            out.push("// startNotRespondingAlert() is left out: the Lua config switched alerts off.".to_owned());
        } else {
            out.push(line.to_owned());
        }
    }
    (seen == 0 || left > 0).then_some(out)
}

/// The index one past the statement that starts at `start`: its own line when that ends in `;`, else the line that
/// closes it, which starts in the first column.
fn statement_end(block: &[&str], start: usize) -> usize {
    if block[start].ends_with(';') {
        return start + 1;
    }
    block[start + 1..]
        .iter()
        .position(|line| line.starts_with('}'))
        .map_or(block.len(), |close| start + close + 2)
}

/// Whether `line` names `name` as a whole identifier.
fn mentions(line: &str, name: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    line.match_indices(name).any(|(at, _)| {
        !line[..at].chars().next_back().is_some_and(ident) && !line[at + name.len()..].chars().next().is_some_and(ident)
    })
}

/// Blocks of lines with a blank line between them.
pub(crate) fn join(blocks: Vec<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for block in blocks.into_iter().filter(|block| !block.is_empty()) {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend(block);
    }
    out
}

/// What the head of the file says about where it came from and how complete it is.
pub(crate) struct Head<'a> {
    pub entry: &'a str,
    pub sources: &'a [String],
    pub unmapped: usize,
}

/// The whole file: the head, the imports, the seat, the stock desktop and then what the Lua files said.
pub(crate) fn document(head: &Head<'_>, needs: &Needs, stock: Vec<String>, body: Vec<Vec<String>>) -> String {
    let mut lines: Vec<String> = Vec::new();
    if head.unmapped > 0 {
        lines.push(format!(
            "// INCOMPLETE: {} unmapped settings, see comments",
            head.unmapped
        ));
    }
    lines.push(format!(
        "// {} converted to TypeScript by `gnoblinctl config migrate`.",
        head.entry
    ));
    lines.push(
        "// Review it before you rely on it: the converter writes what has a TypeScript equivalent and marks the rest."
            .to_owned(),
    );
    lines.push(format!("// Sources, in load order: {}.", head.sources.join(", ")));
    if head.unmapped > 0 {
        lines.push(
            "// Each setting it could not carry over is a comment that starts with UNMAPPED, where the Lua put it. The command"
                .to_owned(),
        );
        lines.push(
            "// lists them with a pointer into docs/migrating-from-lua.md, which says what each one needs.".to_owned(),
        );
    }
    let mut blocks: Vec<Vec<String>> = Vec::new();
    blocks.push(import_lines(needs));
    if needs.input {
        blocks.push(vec!["const input = gnoblin.input.primary;".to_owned()]);
    }
    blocks.push(stock);
    blocks.extend(body);
    let mut out = lines.join("\n");
    out.push_str("\n\n");
    let rest = join(blocks);
    if !rest.is_empty() {
        out.push_str(&rest.join("\n"));
        out.push('\n');
    }
    out
}

/// The names the converter imports that `gnoblin-core` exports; every other name it imports is `gnoblin-std`'s.
const CORE_NAMES: &[&str] = &["CollectionListener"];

/// The imports from `gnoblin-core` and `gnoblin-std`: for each package one line when it fits, one name a line when it
/// does not.
fn import_lines(needs: &Needs) -> Vec<String> {
    let mut lines = Vec::new();
    let (core, std): (Vec<&str>, Vec<&str>) = needs.names.iter().partition(|name| CORE_NAMES.contains(*name));
    for (package, names) in [("gnoblin-core", core), ("gnoblin-std", std)] {
        if names.is_empty() {
            continue;
        }
        let one = format!("import {{ {} }} from \"{package}\";", names.join(", "));
        if one.len() <= IMPORT_WIDTH {
            lines.push(one);
        } else {
            lines.push("import {".to_owned());
            lines.extend(names.iter().map(|name| format!("    {name},")));
            lines.push(format!("}} from \"{package}\";"));
        }
    }
    if needs.client_type {
        lines.push("import type { Client } from \"gnoblin-core\";".to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(text: &str) -> PatternTranslation {
        PatternTranslation::Glob(text.to_owned())
    }

    #[test]
    fn a_glob_becomes_the_comparison_it_says() {
        assert_eq!(Test::of(&glob("foot")), Some(Test::Equals("foot".to_owned())));
        assert_eq!(
            Test::of(&glob("org.mozilla.*")),
            Some(Test::StartsWith("org.mozilla.".to_owned()))
        );
        assert_eq!(Test::of(&glob("*Notes")), Some(Test::EndsWith("Notes".to_owned())));
        assert_eq!(Test::of(&glob("*fire*")), Some(Test::Includes("fire".to_owned())));
        assert_eq!(Test::of(&glob("*")), None);
        assert_eq!(Test::of(&glob("")), Some(Test::Equals(String::new())));
    }

    #[test]
    fn a_comparison_is_a_condition_on_the_clients_property() {
        assert_eq!(
            Test::Equals("a\"b".to_owned()).condition("appId"),
            "client.appId === \"a\\\"b\""
        );
        assert_eq!(
            Test::StartsWith("x".to_owned()).condition("title"),
            "client.title.startsWith(\"x\")"
        );
        assert_eq!(
            Test::RegExp {
                source: "^a.b$".to_owned(),
                flags: "s".to_owned()
            }
            .condition("appId"),
            "/^a.b$/s.test(client.appId)"
        );
    }

    #[test]
    fn imports_break_over_lines_only_when_they_would_run_long() {
        let mut needs = Needs::default();
        needs.import("Workspace");
        needs.import("FloatingLayout");
        assert_eq!(
            import_lines(&needs),
            ["import { FloatingLayout, Workspace } from \"gnoblin-std\";"]
        );
        for name in [
            "StripLayout",
            "WorkspaceSlideBinding",
            "openLauncher",
            "startDecorations",
            "startNotRespondingAlert",
            "startWindowMenu",
            "CollectionListener",
        ] {
            needs.import(name);
        }
        let lines = import_lines(&needs);
        assert_eq!(
            lines.first().map(String::as_str),
            Some("import { CollectionListener } from \"gnoblin-core\";"),
            "a name of the core package is imported from it"
        );
        assert_eq!(lines[1], "import {");
        assert_eq!(lines[2], "    FloatingLayout,");
        assert_eq!(lines.last().map(String::as_str), Some("} from \"gnoblin-std\";"));
    }

    #[test]
    fn a_stock_key_the_config_binds_itself_is_not_written_twice() {
        let mut needs = Needs::default();
        let taken: BTreeSet<String> = ["XF86AudioMute", "Super+2", "Super+space"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let text = stock(&mut needs, false, true, &taken).join("\n");
        assert!(!text.contains("XF86AudioMute"), "{text}");
        assert!(!text.contains("openLauncher"), "{text}");
        assert!(
            text.contains("for (const position of [1, 3, 4, 5, 6, 7, 8, 9]) {"),
            "{text}"
        );
        assert!(text.contains("XF86AudioMicMute"), "{text}");
    }

    #[test]
    fn the_stock_desktop_is_the_starter_less_what_the_config_declares() {
        let mut needs = Needs::default();
        let text = stock(&mut needs, true, false, &BTreeSet::new()).join("\n");
        assert!(
            !text.contains("gnoblin.workspaces.add("),
            "the Lua config's workspaces stand in place: {text}"
        );
        assert!(text.contains("// startNotRespondingAlert() is left out"), "{text}");
        assert!(!text.contains("startNotRespondingAlert();"), "{text}");
        assert!(
            text.contains("for (let position = 1; position <= 9; position++) {"),
            "{text}"
        );
        assert!(text.contains("openLauncher();"), "{text}");
        let imported = import_lines(&needs).join("\n");
        assert!(imported.contains("openLauncher"), "{imported}");
        assert!(!imported.contains("startNotRespondingAlert"), "{imported}");
        assert!(
            !imported.contains("FloatingLayout"),
            "the workspaces are the Lua config's: {imported}"
        );
    }

    #[test]
    fn a_block_whose_keys_are_all_taken_goes_with_its_comment_and_helper() {
        let mut needs = Needs::default();
        let taken: BTreeSet<String> = ["Super+Left", "Super+Right"].into_iter().map(str::to_owned).collect();
        let text = stock(&mut needs, false, true, &taken).join("\n");
        assert!(!text.contains("moveColumn"), "{text}");
        assert!(!text.contains("Super+Left and Super+Right"), "{text}");
        assert!(!import_lines(&needs).join("\n").contains("StripLayout"));
    }
}

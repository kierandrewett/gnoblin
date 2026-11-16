//! The TypeScript the converter writes.
//!
//! Statements against the specification's own objects, in the style of the owner's reference `init.js`: workspaces are
//! `new Workspace({ id, name, layout })` and `gnoblin.workspaces.add`, a shortcut is `input.bind(trigger, () =>
//! gnoblin.spawn([...]))`, a rule that only styles windows of an exact app id is `gnoblin.clients.where({ appId
//! }).style`, and every other simple rule is a `CollectionListener`. Nothing here reads Lua: the engine hands over what
//! it found, and this module prints it.
//!
//! An `init.ts` replaces the stock default instead of layering over it, so [`stock`] writes the stock desktop out in
//! full, minus what the Lua config set itself. That is what keeps a migrated config the desktop the Lua shell was.

use std::collections::BTreeSet;

use crate::chord::canonical_trigger;
use crate::names;
use crate::pattern::PatternTranslation;

/// The widest an `import` is written on one line.
const IMPORT_WIDTH: usize = 120;

/// What the file needs from the `gnoblin` module, and whether it binds on the seat.
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

/// The stock keys that carry a command: the trigger and the program.
const MEDIA_KEYS: &[(&str, &[&str])] = &[
    (
        "XF86AudioRaiseVolume",
        &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
    ),
    (
        "XF86AudioLowerVolume",
        &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
    ),
    (
        "XF86AudioMute",
        &["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
    ),
    (
        "XF86AudioMicMute",
        &["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
    ),
    ("XF86AudioPlay", &["playerctl", "play-pause"]),
    ("XF86AudioNext", &["playerctl", "next"]),
    ("XF86AudioPrev", &["playerctl", "previous"]),
];

/// The stock desktop: what `data/js/starter/init.ts` does, written out for a config that replaces it.
///
/// `own_workspaces` says the Lua config declared workspaces, which then stand where the stock four would. `alerts` is
/// whether the not-responding alert is on. `taken` holds the canonical triggers the Lua config binds itself: a stock
/// key on one of them is left out, as the Lua config's key took its place when the two were layered.
///
/// A test (`crates/gnoblinctl/tests/config_migrate.rs`) boots the converted Lua starter and the starter and compares the
/// worlds, so this text cannot drift from the starter unseen.
pub(crate) fn stock(needs: &mut Needs, own_workspaces: bool, alerts: bool, taken: &BTreeSet<String>) -> Vec<String> {
    let free = |trigger: &str| !taken.contains(&canonical_trigger(trigger));
    let mut blocks: Vec<Vec<String>> = Vec::new();
    blocks.push(comment(&[
        "The stock desktop. An init.ts replaces the stock default instead of layering over it, so what the Lua shell"
            .to_owned(),
        "kept on its own is written out here. Delete a statement and that part of the desktop is gone.".to_owned(),
    ]));
    if !own_workspaces {
        needs.import("Workspace");
        needs.import("FloatingLayout");
        blocks.push(
            [
                "// Four floating workspaces. The first is shown until a key shows another.",
                "for (const id of [\"1\", \"2\", \"3\", \"4\"]) {",
                "    gnoblin.workspaces.add(new Workspace({ id, name: `Workspace ${id}`, layout: new FloatingLayout() }));",
                "}",
            ]
            .map(str::to_owned)
            .to_vec(),
        );
    }
    needs.import("startDecorations");
    needs.import("startWindowMenu");
    let mut services = vec![
        "// A titlebar on every window, the menu the compositor asks for, and the dialog for a window that stops answering."
            .to_owned(),
        "startDecorations();".to_owned(),
        "startWindowMenu();".to_owned(),
    ];
    if alerts {
        needs.import("startNotRespondingAlert");
        services.push("startNotRespondingAlert();".to_owned());
    } else {
        services.push("// startNotRespondingAlert() is left out: the Lua config switched alerts off.".to_owned());
    }
    blocks.push(services);

    needs.input = true;
    let positions: Vec<usize> = (1..=9).filter(|position| free(&format!("Super+{position}"))).collect();
    if !positions.is_empty() {
        let head = if positions.len() == 9 {
            "for (let position = 1; position <= 9; position++) {".to_owned()
        } else {
            let list: Vec<String> = positions.iter().map(ToString::to_string).collect();
            format!("for (const position of [{}]) {{", list.join(", "))
        };
        let mut block = comment(&[
            "Super+1 .. Super+9 show the workspace at that position on the display the pointer is on. A key past the last"
                .to_owned(),
            "workspace does nothing.".to_owned(),
        ]);
        block.extend(
            [
                "    input.bind(`Super+${position}`, () => {",
                "        const target = gnoblin.workspaces.at(position - 1);",
                "        if (target !== undefined) {",
                "            gnoblin.workspaces.show(target, { duration: 250 });",
                "        }",
                "    });",
                "}",
            ]
            .map(str::to_owned),
        );
        block.insert(2, head);
        blocks.push(block);
    }

    let strip: Vec<(&str, &str)> = [("Super+Left", "focusLeft"), ("Super+Right", "focusRight")]
        .into_iter()
        .filter(|(trigger, _)| free(trigger))
        .collect();
    if !strip.is_empty() {
        needs.import("StripLayout");
        needs.import("Workspace");
        let mut block = comment(&[
            "Super+Left and Super+Right move the focus between the columns of a StripLayout, and do nothing on other layouts."
                .to_owned(),
        ]);
        block.extend(
            [
                "const moveColumn = (side: \"focusLeft\" | \"focusRight\") => {",
                "    const shown = (input.activeDisplay as { viewport: { content: Workspace | null } } | null)?.viewport.content;",
                "    if (shown?.layout instanceof StripLayout) {",
                "        shown.layout[side](shown);",
                "    }",
                "};",
            ]
            .map(str::to_owned),
        );
        for (trigger, side) in strip {
            block.push(format!(
                "input.bind({}, () => moveColumn({}));",
                names::string(trigger),
                names::string(side)
            ));
        }
        blocks.push(block);
    }

    needs.import("WorkspaceSlideBinding");
    let mut swipe = comment(&["Three fingers sliding sideways slide between workspaces.".to_owned()]);
    swipe.push("input.bind(\"Swipe3\", new WorkspaceSlideBinding());".to_owned());
    blocks.push(swipe);

    let media: Vec<&(&str, &[&str])> = MEDIA_KEYS.iter().filter(|(trigger, _)| free(trigger)).collect();
    if !media.is_empty() {
        let mut block = comment(&["Volume, microphone and media player keys: each key runs one command.".to_owned()]);
        block.push("const mediaKeys: Record<string, string[]> = {".to_owned());
        for (trigger, program) in media {
            let program: Vec<String> = program.iter().map(|part| (*part).to_owned()).collect();
            block.push(format!("    {trigger}: {},", argv(&program)));
        }
        block.extend(
            [
                "};",
                "for (const [key, command] of Object.entries(mediaKeys)) {",
                "    input.bind(key, () => {",
                "        gnoblin.spawn(command);",
                "    });",
                "}",
            ]
            .map(str::to_owned),
        );
        blocks.push(block);
    }

    if free("Super+space") {
        needs.import("openLauncher");
        let mut block =
            comment(&["Super+space opens the launcher: a command entry in the middle of the screen.".to_owned()]);
        block.extend(["input.bind(\"Super+space\", () => {", "    openLauncher();", "});"].map(str::to_owned));
        blocks.push(block);
    }
    join(blocks)
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

/// The imports from `gnoblin`: one line when it fits, one name a line when it does not.
fn import_lines(needs: &Needs) -> Vec<String> {
    let mut lines = Vec::new();
    if !needs.names.is_empty() {
        let names: Vec<&str> = needs.names.iter().copied().collect();
        let one = format!("import {{ {} }} from \"gnoblin\";", names.join(", "));
        if one.len() <= IMPORT_WIDTH {
            lines.push(one);
        } else {
            lines.push("import {".to_owned());
            lines.extend(names.iter().map(|name| format!("    {name},")));
            lines.push("} from \"gnoblin\";".to_owned());
        }
    }
    if needs.client_type {
        lines.push("import type { Client } from \"gnoblin\";".to_owned());
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
            ["import { FloatingLayout, Workspace } from \"gnoblin\";"]
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
        assert_eq!(lines.first().map(String::as_str), Some("import {"));
        assert_eq!(lines[1], "    CollectionListener,");
        assert_eq!(lines.last().map(String::as_str), Some("} from \"gnoblin\";"));
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
}

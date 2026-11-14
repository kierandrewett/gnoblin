//! The docs-parity gate: a name that a shell-api or events page documents must be implemented.
//!
//! The implemented surface is the published `gnoblin.d.ts`: the rendered schema and the stock library
//! (`crates/gnoblin-js-types/tests/golden/gnoblin.d.ts`, which a golden test keeps equal to what
//! `gnoblinctl types init` writes). A dotted `gnoblin.` name is followed member by member from the
//! `Gnoblin` interface through the types it names. A `gnoblin.<object>.<signal>` event is one of the keys
//! of `GnoblinSignals`. A `.method(` call is matched by name across every class and interface, because a
//! page does not say which object the receiver is.
//!
//! A documented name that is not implemented must be listed in `design/docs-parity/allow.toml` with the
//! slice that will implement it, or `docs wrong`. The gate fails on an unlisted name, and on a listed name
//! that is now implemented or no longer documented, so the list cannot go stale.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde::Deserialize;

const TYPES: &str = "crates/gnoblin-js-types/tests/golden/gnoblin.d.ts";
const ALLOW: &str = "design/docs-parity/allow.toml";
const DOC_DIRS: &[&str] = &["docs/shell-api", "docs/events"];
const DOC_FILES: &[&str] = &["docs/user-shell.md", "docs/user-scripts.md"];
/// The interface every dotted `gnoblin.` name starts from.
const ROOT_TYPE: &str = "Gnoblin";
/// The interface whose quoted keys are the signal names `gnoblin.on` accepts.
const SIGNALS_TYPE: &str = "GnoblinSignals";
/// Methods of the JavaScript built-ins. A documented `list.map(` is language code, not a gnoblin name.
const BUILTIN_METHODS: &[&str] = &[
    "map",
    "filter",
    "forEach",
    "find",
    "findIndex",
    "some",
    "every",
    "reduce",
    "push",
    "pop",
    "shift",
    "unshift",
    "slice",
    "splice",
    "concat",
    "join",
    "includes",
    "indexOf",
    "sort",
    "reverse",
    "flat",
    "keys",
    "values",
    "entries",
    "toString",
    "toFixed",
    "toLowerCase",
    "toUpperCase",
    "trim",
    "startsWith",
    "endsWith",
    "split",
    "replace",
    "padStart",
    "padEnd",
    "test",
    "exec",
    "then",
    "catch",
    "finally",
    "log",
    "warn",
    "error",
    "stringify",
    "parse",
    "assign",
    "isArray",
    "call",
    "apply",
    "getTime",
    "getHours",
    "getMinutes",
    "toISOString",
];
/// Objects of the language whose method calls are never a gnoblin name; `this` is the user's own class.
const BUILTIN_RECEIVERS: &[&str] = &[
    "console", "Math", "JSON", "Object", "Array", "Promise", "Date", "Number", "String", "Symbol", "Reflect", "this",
];
/// A token that ends like this is a file name (`gnoblin.d.ts`), not a name of the API.
const FILE_SUFFIXES: &[&str] = &[".ts", ".js", ".md", ".toml", ".json"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

/// A reviewed exception: a documented name that is not implemented yet.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowEntry {
    /// `gnoblin.a.b` for a dotted name, or `.method` for an object method.
    pub name: String,
    /// The documented page, relative to the repository root.
    pub doc: String,
    pub reason: String,
    #[serde(rename = "owner-slice")]
    pub owner_slice: String,
}

/// A class or interface of the declarations: what it extends, and its members by name. A member that
/// names a declared type carries that type, so a dotted path can be followed through it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TypeDecl {
    bases: Vec<String>,
    members: BTreeMap<String, Option<String>>,
}

/// The implemented names, read from the declarations.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Implemented {
    types: BTreeMap<String, TypeDecl>,
    /// `gnoblin.<object>.<signal>`, with underscores.
    pub events: BTreeSet<String>,
    /// `.name` for every method any class or interface declares.
    pub methods: BTreeSet<String>,
}

impl Implemented {
    /// A name is provided when it is a declared method (`.name`), an event or a namespace that holds
    /// one (`gnoblin.window` for `gnoblin.window.created`), or a path of members from the root type.
    fn provides(&self, name: &str) -> bool {
        if name.starts_with('.') {
            return self.methods.contains(name);
        }
        // A hyphen spelling names the underscored signal, as `gnoblin.on` accepts it.
        let name = name.replace('-', "_");
        let prefix = format!("{name}.");
        self.events.contains(&name) || self.events.iter().any(|event| event.starts_with(&prefix)) || self.reaches(&name)
    }

    /// Follow `gnoblin.a.b.c` from the root type. A member whose type is not declared here (a primitive, a
    /// union, a type of the language) ends the check: nothing is known beyond it, so the rest is accepted.
    fn reaches(&self, name: &str) -> bool {
        let mut current = ROOT_TYPE.to_owned();
        for segment in name.split('.').skip(1) {
            match self.member(&current, segment, 0) {
                None => return false,
                Some(Some(next)) if self.types.contains_key(&next) => current = next,
                Some(_) => return true,
            }
        }
        true
    }

    /// The member `name` of a type or of a type it extends: `None` when there is none, else its declared type.
    fn member(&self, ty: &str, name: &str, depth: usize) -> Option<Option<String>> {
        let decl = self.types.get(ty)?;
        if let Some(found) = decl.members.get(name) {
            return Some(found.clone());
        }
        if depth > 8 {
            return None;
        }
        decl.bases.iter().find_map(|base| self.member(base, name, depth + 1))
    }
}

/// One documented occurrence of a name, with the page and line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub name: String,
    pub doc: String,
    pub line: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DocsReport {
    /// Distinct documented names that the surface does not provide.
    pub missing: usize,
    /// Of those, how many an allow entry covers.
    pub allowed: usize,
    /// Documented names not implemented and not listed; each one fails the gate.
    pub unlisted: Vec<Mention>,
    /// Allow entries that are implemented now or no longer documented; each one fails the gate.
    pub stale: Vec<String>,
    /// Distinct `(name, doc)` pairs in the documentation.
    pub documented: usize,
}

/// Read the classes and interfaces a `.d.ts` declares. A declaration is a line that opens `class` or
/// `interface`; its members are the lines one indent level deeper, up to the closing brace. Lines at any
/// other depth (parameters, object types, comments) are not members.
pub fn parse_declarations(text: &str) -> Implemented {
    let mut implemented = Implemented::default();
    let mut open: Option<(usize, String)> = None;
    for line in text.lines() {
        let body = line.trim();
        let indent = line.len() - line.trim_start().len();
        if let Some((declared_at, owner)) = &open {
            if indent == *declared_at && body.starts_with('}') {
                open = None;
            } else if indent == declared_at + 4 {
                read_member(owner, body, &mut implemented);
            }
        } else if let Some((name, bases)) = declaration(body) {
            let decl = implemented.types.entry(name.clone()).or_default();
            decl.bases.extend(bases);
            if body.ends_with('{') {
                open = Some((indent, name));
            }
        }
    }
    implemented
}

/// The name and base types of a `class` or `interface` line, or `None` for any other line.
fn declaration(body: &str) -> Option<(String, Vec<String>)> {
    let mut rest = body;
    for prefix in ["export ", "declare ", "abstract "] {
        rest = rest.strip_prefix(prefix).unwrap_or(rest);
    }
    let rest = rest
        .strip_prefix("interface ")
        .or_else(|| rest.strip_prefix("class "))?;
    let name: String = rest.chars().take_while(is_ident).collect();
    if name.is_empty() {
        return None;
    }
    let after_name = skip_generics(&rest[name.len()..]);
    let heritage = after_name.trim_start().strip_prefix("extends ");
    let bases = heritage
        .map(|list| {
            let list = list.split('{').next().unwrap_or(list);
            let list = list.split(" implements ").next().unwrap_or(list);
            split_top_level(list)
                .iter()
                .map(|base| base.trim().chars().take_while(is_ident).collect::<String>())
                .filter(|base| !base.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some((name, bases))
}

/// `text` without a leading `<...>` generic list.
fn skip_generics(text: &str) -> &str {
    if !text.starts_with('<') {
        return text;
    }
    let mut depth = 0usize;
    for (index, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &text[index + 1..];
                }
            }
            _ => {}
        }
    }
    text
}

/// Split at commas that are not inside `<...>`.
fn split_top_level(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in list.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&list[start..]);
    parts
}

/// Record one member line of the type `owner`: a method adds `.name` to the methods, a property or accessor
/// adds the type it names. In the signals interface a quoted key is an event instead.
fn read_member(owner: &str, body: &str, implemented: &mut Implemented) {
    if owner == SIGNALS_TYPE {
        if let Some(key) = body.strip_prefix('"').and_then(|rest| rest.split('"').next()) {
            implemented.events.insert(key.replace('-', "_"));
        }
        return;
    }
    let mut rest = body;
    let mut accessor = false;
    loop {
        let before = rest;
        for modifier in [
            "readonly ",
            "static ",
            "abstract ",
            "public ",
            "private ",
            "protected ",
            "async ",
        ] {
            rest = rest.strip_prefix(modifier).unwrap_or(rest);
        }
        if let Some(after) = rest.strip_prefix("get ").or_else(|| rest.strip_prefix("set ")) {
            rest = after;
            accessor = true;
        }
        if rest == before {
            break;
        }
    }
    let name: String = rest.chars().take_while(is_ident).collect();
    if name.is_empty() || name == "constructor" {
        return;
    }
    let after = skip_generics(rest[name.len()..].trim_start_matches('?'));
    let declared_type = if after.starts_with('(') {
        if accessor {
            after.split_once("):").and_then(|(_, ty)| type_name(ty))
        } else {
            implemented.methods.insert(format!(".{name}"));
            None
        }
    } else {
        after.strip_prefix(':').and_then(type_name)
    };
    let decl = implemented.types.entry(owner.to_owned()).or_default();
    decl.members.insert(name, declared_type);
}

/// The first type a member's type text names: `Collection<Client> | null` is `Collection`.
fn type_name(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix("readonly ").unwrap_or(text);
    let text = match text.strip_prefix("import(") {
        Some(rest) => rest.split_once(").")?.1,
        None => text,
    };
    let name: String = text.chars().take_while(is_ident).collect();
    // `Item[]` is an array of the type, which has the members of an array and not of `Item`.
    let array = text[name.len()..].starts_with('[');
    (!name.is_empty() && !array).then_some(name)
}

/// Every dotted `gnoblin.` name and every `.method(` call in one page. A `gnoblin.` name must not
/// follow a path or identifier character, so `docs/gnoblin.d.ts` and `https://` URLs are not
/// names; file suffixes are dropped.
pub fn scan_doc(doc: &str, text: &str) -> Vec<Mention> {
    let mut mentions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        scan_paths(doc, line, line_number, &mut mentions);
        scan_calls(doc, line, line_number, &mut mentions);
    }
    mentions
}

fn scan_paths(doc: &str, line: &str, line_number: usize, mentions: &mut Vec<Mention>) {
    let bytes = line.as_bytes();
    let mut search = 0;
    while let Some(found) = line[search..].find("gnoblin.") {
        let start = search + found;
        search = start + "gnoblin.".len();
        // A name inside a longer identifier, path or URL is not a name we check.
        let preceded = start > 0 && (is_name_char(bytes[start - 1]) || bytes[start - 1] == b'/');
        if preceded {
            continue;
        }
        let mut end = search;
        while end < bytes.len() && (is_name_char(bytes[end]) || bytes[end] == b'.' || bytes[end] == b'-') {
            end += 1;
        }
        let token = line[start..end].trim_end_matches(['.', '-']);
        search = end;
        let file_name = FILE_SUFFIXES.iter().any(|suffix| token.ends_with(suffix));
        if token.len() > "gnoblin.".len() && !file_name {
            mentions.push(Mention {
                name: token.to_owned(),
                doc: doc.to_owned(),
                line: line_number,
            });
        }
    }
}

fn scan_calls(doc: &str, line: &str, line_number: usize, mentions: &mut Vec<Mention>) {
    let bytes = line.as_bytes();
    for (dot, _) in line.match_indices('.') {
        let after = &bytes[dot + 1..];
        if !after.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
            continue;
        }
        let ident_len = after.iter().take_while(|c| is_name_char(**c)).count();
        if after.get(ident_len) != Some(&b'(') {
            continue;
        }
        let before = dot.checked_sub(1).map(|at| bytes[at]);
        if !before.is_some_and(|c| is_name_char(c) || c == b')' || c == b']') {
            continue;
        }
        let receiver_len = bytes[..dot].iter().rev().take_while(|c| is_name_char(**c)).count();
        let receiver = &line[dot - receiver_len..dot];
        let method = &line[dot + 1..dot + 1 + ident_len];
        if BUILTIN_METHODS.contains(&method) || BUILTIN_RECEIVERS.contains(&receiver) {
            continue;
        }
        mentions.push(Mention {
            name: format!(".{method}"),
            doc: doc.to_owned(),
            line: line_number,
        });
    }
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_ident(c: &char) -> bool {
    c.is_ascii_alphanumeric() || *c == '_'
}

/// Compare the documented names with the implemented surface and the allow list.
pub fn reconcile(mentions: &[Mention], implemented: &Implemented, allow: &[AllowEntry]) -> DocsReport {
    let mut first: BTreeMap<(String, String), usize> = BTreeMap::new();
    for mention in mentions {
        first
            .entry((mention.name.clone(), mention.doc.clone()))
            .or_insert(mention.line);
    }
    let mut report = DocsReport {
        documented: first.len(),
        ..DocsReport::default()
    };
    for ((name, doc), line) in &first {
        if implemented.provides(name) {
            continue;
        }
        report.missing += 1;
        if allow.iter().any(|entry| &entry.name == name && &entry.doc == doc) {
            report.allowed += 1;
        } else {
            report.unlisted.push(Mention {
                name: name.clone(),
                doc: doc.clone(),
                line: *line,
            });
        }
    }
    for entry in allow {
        let key = (entry.name.clone(), entry.doc.clone());
        if implemented.provides(&entry.name) {
            report.stale.push(format!(
                "{} in {} is implemented now; remove its allow entry",
                entry.name, entry.doc
            ));
        } else if !first.contains_key(&key) {
            report.stale.push(format!(
                "{} in {} is no longer documented; remove its allow entry",
                entry.name, entry.doc
            ));
        }
    }
    report
}

fn load_allow(root: &Path) -> Result<Vec<AllowEntry>, String> {
    let path = root.join(ALLOW);
    let text = fs::read_to_string(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let parsed: AllowFile = toml::from_str(&text).map_err(|error| format!("invalid TOML in {ALLOW}: {error}"))?;
    for entry in &parsed.allow {
        if entry.reason.trim().is_empty() {
            return Err(format!("{ALLOW}: `{}` in {} has no reason", entry.name, entry.doc));
        }
        let slice = entry.owner_slice.as_str();
        let numbered = slice
            .strip_prefix('S')
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        if !numbered && slice != "docs wrong" {
            return Err(format!(
                "{ALLOW}: `{}` in {} has owner-slice `{slice}`, not an S-number or `docs wrong`",
                entry.name, entry.doc
            ));
        }
    }
    Ok(parsed.allow)
}

fn load_docs(root: &Path) -> Result<Vec<Mention>, String> {
    let mut pages: Vec<_> = DOC_FILES.iter().map(|file| root.join(file)).collect();
    for dir in DOC_DIRS.iter().copied() {
        pages.extend(
            fs::read_dir(root.join(dir))
                .map_err(|error| format!("could not read {dir}: {error}"))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "md")),
        );
    }
    pages.sort();
    let mut mentions = Vec::new();
    for page in pages {
        let relative = page
            .strip_prefix(root)
            .unwrap_or(&page)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&page).map_err(|error| format!("could not read {relative}: {error}"))?;
        mentions.extend(scan_doc(&relative, &text));
    }
    Ok(mentions)
}

/// Run the gate from the repository root. Prints one line per failure and exits non-zero on any.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    let outcome = (|| -> Result<DocsReport, String> {
        let types = fs::read_to_string(root.join(TYPES)).map_err(|error| format!("could not read {TYPES}: {error}"))?;
        let implemented = parse_declarations(&types);
        let allow = load_allow(root)?;
        Ok(reconcile(&load_docs(root)?, &implemented, &allow))
    })();
    let report = match outcome {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[docs-parity] {error}");
            return ExitCode::from(2);
        }
    };
    for mention in &report.unlisted {
        println!(
            "[docs-parity] unlisted {}:{} {}",
            mention.doc, mention.line, mention.name
        );
    }
    for stale in &report.stale {
        println!("[docs-parity] stale {stale}");
    }
    println!(
        "[docs-parity] {} documented names, {} not implemented ({} allowed, {} unlisted), {} stale",
        report.documented,
        report.missing,
        report.allowed,
        report.unlisted.len(),
        report.stale.len()
    );
    if report.unlisted.is_empty() && report.stale.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECLARATIONS: &str = r#"
declare module "gnoblin" {
    export interface Gnoblin {
        /** Docs. */
        readonly process: ProcessTable;
        readonly clients: Collection<Client>;
        readonly main: Client;
        readonly list: Client[];
        readonly extensions: import("gnoblin/extensions/xr").Holder | null;
        motion(seconds: number): number;
        on<K extends keyof GnoblinSignals>(name: K, handler: (event: GnoblinSignals[K]) => void): Subscription;
    }
    export interface ProcessTable {
        spawn(command: string, options?: SpawnOptions): Process;
        readonly size: number;
    }
    export class Node {
        readonly parent: Node | null;
        close(): void;
    }
    export class Client extends Node {
        readonly appId: string;
        animate(
            values: ClientAnimation,
            options: AnimateOptions,
        ): void;
        get style(): ClientStyle;
    }
    export interface Collection<T> extends Iterable<T> {
        where(query: Query<T>): Collection<T>;
        readonly length: number;
    }
    export class ClientStyle {
        opacity: number;
    }
    export interface Rect extends Vec2, Size2D {}
}
declare module "gnoblin/schema/signals" {
    export interface GnoblinSignals {
        "gnoblin.window.created": { readonly name: "gnoblin.window.created"; };
        "gnoblin.session.lock_state_changed": { readonly state: string; };
    }
}
"#;

    fn implemented() -> Implemented {
        parse_declarations(DECLARATIONS)
    }

    fn entry(name: &str, doc: &str) -> AllowEntry {
        AllowEntry {
            name: name.into(),
            doc: doc.into(),
            reason: "why".into(),
            owner_slice: "docs wrong".into(),
        }
    }

    fn names(mentions: &[Mention]) -> Vec<&str> {
        let mut names: Vec<_> = mentions.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn declarations_give_methods_events_and_typed_members() {
        let surface = implemented();
        for method in [".motion", ".on", ".spawn", ".close", ".animate", ".where"] {
            assert!(surface.methods.contains(method), "{method}: {:?}", surface.methods);
        }
        assert!(
            surface.events.contains("gnoblin.window.created"),
            "{:?}",
            surface.events
        );
        assert!(!surface.methods.contains(".opacity"), "a property is not a method");
        assert_eq!(surface.types["Client"].bases, ["Node"]);
        assert_eq!(surface.types["Rect"].bases, ["Vec2", "Size2D"]);
        assert_eq!(surface.types["Client"].members["style"].as_deref(), Some("ClientStyle"));
        assert_eq!(
            surface.types["Gnoblin"].members["clients"].as_deref(),
            Some("Collection")
        );
        assert_eq!(
            surface.types["Gnoblin"].members["extensions"].as_deref(),
            Some("Holder")
        );
    }

    #[test]
    fn a_dotted_path_is_followed_through_the_declared_types() {
        let surface = implemented();
        for name in [
            "gnoblin.process",
            "gnoblin.process.spawn",
            "gnoblin.clients.where",
            "gnoblin.clients.length",
            "gnoblin.motion",
            "gnoblin.window",
            "gnoblin.window.created",
            "gnoblin.session.lock-state-changed",
        ] {
            assert!(surface.provides(name), "{name}");
        }
        for name in ["gnoblin.process.explode", "gnoblin.nothing", "gnoblin.window.exploded"] {
            assert!(!surface.provides(name), "{name}");
        }
    }

    #[test]
    fn a_path_through_an_inherited_member_or_an_undeclared_type_is_accepted() {
        let surface = implemented();
        assert!(
            surface.provides("gnoblin.main.close"),
            "Node.close, through Client's base"
        );
        assert!(
            surface.provides("gnoblin.main.style.opacity"),
            "through an accessor's type"
        );
        assert!(
            surface.provides("gnoblin.clients.where.anything"),
            "nothing is known past a method"
        );
        assert!(
            surface.provides("gnoblin.extensions.xr"),
            "a type outside the file ends the check"
        );
        assert!(!surface.provides("gnoblin.clients.appId"), "a Collection has no appId");
        assert!(
            surface.provides("gnoblin.list.find"),
            "an array has the members of an array"
        );
    }

    #[test]
    fn a_method_call_and_a_dotted_name_are_found_but_urls_paths_and_builtins_are_not() {
        let mentions = scan_doc(
            "docs/shell-api/x.md",
            "Call client.animate({}) and `gnoblin.window.created`.\nSee https://example.com/gnoblin.a and docs/gnoblin.d.ts.\nUse list.map(f), console.log(1) and this.draw().\n",
        );
        assert_eq!(names(&mentions), [".animate", "gnoblin.window.created"], "{mentions:?}");
        assert_eq!(mentions[0].line, 1);
    }

    #[test]
    fn a_call_on_a_call_result_and_on_an_index_is_a_method_call() {
        let mentions = scan_doc(
            "docs/shell-api/x.md",
            "gnoblin.find(\"Window\")[0].close(); a.b().raise();\n",
        );
        assert_eq!(
            names(&mentions),
            [".b", ".close", ".raise", "gnoblin.find"],
            "{mentions:?}"
        );
    }

    #[test]
    fn a_hyphenated_event_name_is_read_whole() {
        let mentions = scan_doc("docs/events/a.md", "Subscribe to `gnoblin.session.lock-requested`.\n");
        assert_eq!(names(&mentions), ["gnoblin.session.lock-requested"], "{mentions:?}");
    }

    #[test]
    fn a_hyphen_spelling_is_satisfied_by_its_underscored_event() {
        let mentions = scan_doc(
            "docs/events/a.md",
            "Subscribe to `gnoblin.session.lock-state-changed`.\n",
        );
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.missing, 0, "{report:?}");
        assert!(report.unlisted.is_empty(), "{report:?}");
    }

    #[test]
    fn a_hyphen_spelling_of_an_unimplemented_event_is_still_unlisted() {
        let mentions = scan_doc("docs/events/a.md", "Use `gnoblin.session.no-such-event`.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.unlisted.len(), 1, "{report:?}");
    }

    #[test]
    fn a_documented_name_absent_from_the_surface_is_unlisted() {
        let mentions = scan_doc("docs/events/a.md", "Use `gnoblin.input.gesture`.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.missing, 1);
        assert_eq!(report.unlisted.len(), 1, "{report:?}");
        assert!(report.stale.is_empty());
    }

    #[test]
    fn an_allow_entry_for_a_missing_name_covers_it() {
        let mentions = scan_doc("docs/events/a.md", "Use `gnoblin.input.gesture`.\n");
        let allow = [entry("gnoblin.input.gesture", "docs/events/a.md")];
        let report = reconcile(&mentions, &implemented(), &allow);
        assert_eq!((report.missing, report.allowed), (1, 1));
        assert!(report.unlisted.is_empty() && report.stale.is_empty(), "{report:?}");
    }

    #[test]
    fn an_allow_entry_that_became_implemented_is_stale() {
        let mentions = scan_doc("docs/shell-api/a.md", "Call `gnoblin.process.spawn(\"x\")`.\n");
        let allow = [entry("gnoblin.process.spawn", "docs/shell-api/a.md")];
        let report = reconcile(&mentions, &implemented(), &allow);
        assert_eq!(report.stale.len(), 1, "{report:?}");
        assert!(report.stale[0].contains("implemented now"));
    }

    #[test]
    fn an_allow_entry_for_a_name_no_longer_documented_is_stale() {
        let allow = [entry(".gone", "docs/shell-api/a.md")];
        let report = reconcile(&[], &implemented(), &allow);
        assert_eq!(report.stale.len(), 1, "{report:?}");
        assert!(report.stale[0].contains("no longer documented"));
    }

    #[test]
    fn a_method_the_declarations_lack_is_unlisted() {
        let mentions = scan_doc("docs/shell-api/a.md", "Call `client.explode()`.\n");
        let report = reconcile(&mentions, &implemented(), &[]);
        assert_eq!(report.unlisted.len(), 1, "{report:?}");
        assert_eq!(report.unlisted[0].name, ".explode");
    }

    #[test]
    fn the_published_declarations_provide_the_core_of_the_api() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the xtask lives in the repository");
        let text = fs::read_to_string(root.join(TYPES)).expect("the published declarations read");
        let surface = parse_declarations(&text);
        for name in [
            "gnoblin.clients",
            "gnoblin.process.spawn",
            "gnoblin.session.lock",
            "gnoblin.on",
            "gnoblin.window.created",
            "gnoblin.config.reload_failed",
            ".animate",
            ".moveToOutput",
        ] {
            assert!(surface.provides(name), "{name}");
        }
        assert!(!surface.provides("gnoblin.no_such_member"));
    }
}

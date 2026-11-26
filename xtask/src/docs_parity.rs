//! The docs-parity gate: a name that a shell-api or events page documents must be implemented.
//!
//! The implemented surface is what a script imports: the native library, which a bare `import ... from "gnoblin-core"` /
//! `"gnoblin-std"` runs on. It is the declarations of its generated classes (the `object-types` target of `gnoblin-sdkgen`, the
//! files `crates/gnoblin-js/build.rs` embeds and `gnoblinctl types init` hands an editor), rendered here because no generated
//! file is committed. The members `gnoblin-std` adds to the core's classes are `declare module` augmentations in them. The
//! TypeScript library under `data/js` is not read: a page that documents a name only it has fails.
//!
//! Plus the schema's signals. A dotted `gnoblin.` name is followed member by member from the `Gnoblin` class through the types it
//! names. A `gnoblin.<object>.<signal>` event is a signal of the schema. A `.method(` call is matched by name across every
//! class and interface, because a page does not say which object the receiver is.
//!
//! A documented name that is not implemented must be listed in `xtask/docs-parity-allow.toml` with the
//! slice that will implement it, or `docs wrong`. The gate fails on an unlisted name, and on a listed name
//! that is now implemented or no longer documented, so the list cannot go stale.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use gnoblin_schema::{Catalog, proto};
use serde::Deserialize;

/// The packages of the native library the surface is read from.
const PACKAGES: &[&str] = &["gnoblin-core", "gnoblin-std"];
const ALLOW: &str = "xtask/docs-parity-allow.toml";
const DOC_DIRS: &[&str] = &["docs/shell-api", "docs/events"];
const DOC_FILES: &[&str] = &["docs/user-shell.md", "docs/user-scripts.md"];
/// The interface every dotted `gnoblin.` name starts from.
const ROOT_TYPE: &str = "Gnoblin";
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
/// names a declared type carries that type, so a dotted path can be followed through it. Both libraries declare some
/// classes (`Gnoblin`, `Client`) and the same member may be declared in each with its own type, so a member keeps every type
/// it was declared with.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TypeDecl {
    bases: Vec<String>,
    members: BTreeMap<String, Vec<Option<String>>>,
    /// `Prefixed<"Client", ClientListener>` bases, as `(prefix, listener)`: a subscription `on<Prefix><Event>` for each
    /// method `on<Event>` of the listener.
    prefixed: Vec<(String, String)>,
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
        // A hyphen spelling names the underscored signal.
        let name = name.replace('-', "_");
        let prefix = format!("{name}.");
        self.events.contains(&name) || self.events.iter().any(|event| event.starts_with(&prefix)) || self.reaches(&name)
    }

    /// Follow `gnoblin.a.b.c` from the root type, through every type a member was declared with. A member declared with a type
    /// that is not declared here (a primitive, a union, a type of the language, or none at all) ends the check: nothing is known
    /// beyond it, so the rest is accepted.
    fn reaches(&self, name: &str) -> bool {
        let mut current: BTreeSet<String> = BTreeSet::from([ROOT_TYPE.to_owned()]);
        for segment in name.split('.').skip(1) {
            let mut next = BTreeSet::new();
            let mut found = false;
            let mut unknown = false;
            for ty in &current {
                for declared in self.member(ty, segment, 0) {
                    found = true;
                    match declared.filter(|declared| self.types.contains_key(declared)) {
                        Some(declared) => {
                            next.insert(declared);
                        }
                        None => unknown = true,
                    }
                }
            }
            if !found {
                return false;
            }
            if unknown {
                return true;
            }
            current = next;
        }
        true
    }

    /// The declared types of the member `name` of a type or of the types it extends: empty when there is none.
    fn member(&self, ty: &str, name: &str, depth: usize) -> Vec<Option<String>> {
        let Some(decl) = self.types.get(ty) else {
            return Vec::new();
        };
        let mut found = decl.members.get(name).cloned().unwrap_or_default();
        if depth > 8 {
            return found;
        }
        // A subscription a `Prefixed` base generates is a member that has the listener's method behind it.
        for (prefix, listener) in &decl.prefixed {
            let event = name
                .strip_prefix("on")
                .and_then(|rest| rest.strip_prefix(prefix.as_str()));
            if let Some(event) = event.filter(|event| !event.is_empty())
                && !self.member(listener, &format!("on{event}"), depth + 1).is_empty()
            {
                found.push(None);
            }
        }
        for base in &decl.bases {
            found.extend(self.member(base, name, depth + 1));
        }
        found
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
/// `interface` (a header that a formatter broke over several lines is one line to this reader); its members are the
/// lines at the indent of its first member, up to the closing brace, whatever the indent unit is (the TypeScript library is
/// written with four spaces, the declarations of the native library with two). Lines at any other depth (parameters, object
/// types, comments) are not members. A declaration inside a `declare module` block is read like any other, so what a library
/// adds to another's class there is a member of that class.
///
/// Two bases are generated sets of subscriptions, not types: `Subscriptions<Listener>` is the methods of the listener
/// (a subscription for each), and `Prefixed<"Client", Listener>` is the same under `onClient<Event>` names.
pub fn parse_declarations(text: &str) -> Implemented {
    let mut implemented = Implemented::default();
    // The declaration being read: where it was declared, its name, and the indent of its members once the first is seen.
    let mut open: Option<(usize, String, Option<usize>)> = None;
    let mut lines = text.lines();
    while let Some(first) = lines.next() {
        let mut line = first.to_owned();
        let indent = first.len() - first.trim_start().len();
        if open.is_none() && declaration(first.trim()).is_some() && !first.contains('{') {
            // The header goes on until the line that opens the body.
            for next in lines.by_ref() {
                line.push(' ');
                line.push_str(next.trim());
                if next.contains('{') {
                    break;
                }
            }
        }
        let body = line.trim();
        if let Some((declared_at, owner, member_indent)) = &mut open {
            if indent == *declared_at && body.starts_with('}') {
                open = None;
            } else if !body.is_empty() && indent > *declared_at && indent == *member_indent.get_or_insert(indent) {
                read_member(owner, body, &mut implemented);
            }
        } else if let Some((name, bases)) = declaration(body) {
            let decl = implemented.types.entry(name.clone()).or_default();
            for base in bases {
                match base {
                    Base::Type(base) => decl.bases.push(base),
                    Base::Prefixed { prefix, listener } => decl.prefixed.push((prefix, listener)),
                }
            }
            if body.ends_with('{') {
                open = Some((indent, name, None));
            }
        }
    }
    add_prefixed_methods(&mut implemented);
    implemented
}

/// A base of a declaration: a type to inherit members from, or the subscriptions generated from a listener.
#[derive(Debug, PartialEq, Eq)]
enum Base {
    Type(String),
    Prefixed { prefix: String, listener: String },
}

/// The bases an `extends` entry names: `Subscriptions<L>` has the members of `L`, `Prefixed<"P", L>` the subscriptions of
/// `L` under the prefix, `MemberSubscriptions<T>` those of both listeners of the members a collection holds, and any
/// other entry is the type it starts with.
fn base_of(entry: &str) -> Vec<Base> {
    let entry = entry.trim();
    let name: String = entry.chars().take_while(is_ident).collect();
    if name.is_empty() {
        return Vec::new();
    }
    let arguments = entry[name.len()..]
        .strip_prefix('<')
        .and_then(|rest| rest.rfind('>').map(|end| &rest[..end]))
        .map(split_top_level);
    let listener = |argument: Option<&&str>| {
        argument
            .map(|text| text.trim().chars().take_while(is_ident).collect::<String>())
            .filter(|listener| !listener.is_empty())
    };
    match (name.as_str(), arguments) {
        ("Subscriptions", Some(arguments)) => listener(arguments.first()).map(Base::Type).into_iter().collect(),
        ("Prefixed", Some(arguments)) if arguments.len() == 2 => {
            let prefix = arguments[0].trim().trim_matches('"').to_owned();
            listener(arguments.get(1))
                .map(|listener| Base::Prefixed { prefix, listener })
                .into_iter()
                .collect()
        }
        ("MemberSubscriptions", Some(_)) => ["ClientListener", "OutputListener"]
            .map(|listener| Base::Type(listener.to_owned()))
            .into(),
        _ => vec![Base::Type(name)],
    }
}

/// Counts the generated `on<Prefix><Event>` subscriptions as methods, because a page calls them as `.onClientCreated(`.
fn add_prefixed_methods(implemented: &mut Implemented) {
    let mut names = Vec::new();
    for decl in implemented.types.values() {
        for (prefix, listener) in &decl.prefixed {
            let Some(listener) = implemented.types.get(listener) else {
                continue;
            };
            for member in listener.members.keys() {
                if let Some(event) = member.strip_prefix("on") {
                    names.push(format!(".on{prefix}{event}"));
                }
            }
        }
    }
    implemented.methods.extend(names);
}

/// The name and base types of a `class` or `interface` line, or `None` for any other line.
fn declaration(body: &str) -> Option<(String, Vec<Base>)> {
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
            split_top_level(list).into_iter().flat_map(base_of).collect()
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
/// adds the type it names.
fn read_member(owner: &str, body: &str, implemented: &mut Implemented) {
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
    // A named subscription declared by hand (`readonly onClientCreated: Subscribe<...>`) is called like a method.
    if declared_type.as_deref() == Some("Subscribe") {
        implemented.methods.insert(format!(".{name}"));
    }
    let decl = implemented.types.entry(owner.to_owned()).or_default();
    let declared = decl.members.entry(name).or_default();
    if !declared.contains(&declared_type) {
        declared.push(declared_type);
    }
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

/// The declarations of the native library: what `gnoblin-sdkgen` renders for the `object-types` target, rendered here the way
/// `crates/gnoblin-js/build.rs` renders them (`gnoblin_sdkgen::emit_for_build`) and `gnoblinctl types init` hands them to an
/// editor. A bare `import ... from "gnoblin-core"` or `"gnoblin-std"` runs on the classes these declare
/// (`js/gnoblin-core/generated/{object_base,objects}.d.ts`, `js/gnoblin-std/generated/objects.d.ts`), whose JavaScript is the
/// `object-js` target of the same catalog. The members `gnoblin-std` adds to the core's classes (`gnoblin.workspaces`) are
/// declared in `declare module` blocks of the std file.
fn native_surface(root: &Path) -> Result<String, String> {
    let spec = gnoblin_sdkgen::SdkSpec::load(root)?;
    let rendered = gnoblin_sdkgen::files_of(&gnoblin_sdkgen::render(&spec, Some("object-types"))?);
    let mut files: Vec<(String, String)> = rendered
        .into_iter()
        .filter(|(path, _)| {
            path.ends_with(".d.ts")
                && path
                    .strip_prefix("js/")
                    .and_then(|rest| rest.split('/').next())
                    .is_some_and(|package| PACKAGES.contains(&package))
        })
        .collect();
    files.sort();
    Ok(files.iter().fold(String::new(), |mut text, (_, source)| {
        text.push('\n');
        text.push_str(source);
        text
    }))
}

/// Every signal the schema declares, as `namespace.object.signal`: the events the compositor raises, which the
/// documents name as `gnoblin.<object>.<signal>`.
fn schema_signals(root: &Path) -> Result<BTreeSet<String>, String> {
    let catalog = Catalog::load_dir(&root.join(proto::SCHEMA_DIR)).map_err(|error| error.to_string())?;
    let mut signals = BTreeSet::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            for signal in &object.signals {
                signals.insert(format!("{namespace}.{}.{}", object.name, signal.name).replace('-', "_"));
            }
        }
    }
    Ok(signals)
}

/// Run the gate from the repository root. Prints one line per failure and exits non-zero on any.
pub fn run() -> ExitCode {
    let root = Path::new(".");
    let outcome = (|| -> Result<DocsReport, String> {
        let mut implemented = parse_declarations(&native_surface(root)?);
        implemented.events.extend(schema_signals(root)?);
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
declare module "gnoblin-core" {
    export interface Gnoblin {
        /** Docs. */
        readonly process: ProcessTable;
        readonly clients: Collection<Client>;
        readonly main: Client;
        readonly list: Client[];
        readonly extensions: import("gnoblin-std/extensions/xr").Holder | null;
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
"#;

    /// The surface of the declarations, and two of the schema's signals (the gate reads those from the schema).
    fn implemented() -> Implemented {
        let mut surface = parse_declarations(DECLARATIONS);
        surface
            .events
            .extend(["gnoblin.window.created", "gnoblin.session.lock_state_changed"].map(str::to_owned));
        surface
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
        assert_eq!(
            surface.types["Client"].members["style"],
            [Some("ClientStyle".to_owned())]
        );
        assert_eq!(
            surface.types["Gnoblin"].members["clients"],
            [Some("Collection".to_owned())]
        );
        assert_eq!(
            surface.types["Gnoblin"].members["extensions"],
            [Some("Holder".to_owned())]
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
    fn the_published_surface_provides_the_core_of_the_api() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the xtask lives in the repository");
        let text = native_surface(root).expect("the library reads");
        let mut surface = parse_declarations(&text);
        surface.events.extend(schema_signals(root).expect("the schema loads"));
        for name in [
            "gnoblin.clients",
            "gnoblin.process.spawn",
            "gnoblin.session.lock",
            "gnoblin.session.addListener",
            "gnoblin.onClientCreated",
            "gnoblin.onClientTitleChanged",
            "gnoblin.onOutputModeChanged",
            "gnoblin.session.on",
            "gnoblin.runtime.on",
            "gnoblin.clients.on",
            "gnoblin.clients.addListener",
            "gnoblin.outputs.on",
            ".onClientCreated",
            "gnoblin.window.created",
            "gnoblin.config.reload_failed",
            ".set",
            ".moveToOutput",
        ] {
            assert!(surface.provides(name), "{name}");
        }
        assert!(!surface.provides("gnoblin.no_such_member"));
        assert!(surface.provides("gnoblin.on"), "the native root has the generated `on`");
        // The TypeScript library's one method for each event of an object (`gnoblin.clients.onAdded(handler)`) is not native: an
        // object has `on("added", handler)` and `addListener`, so a page that writes the method form must fail the gate.
        for name in [
            "gnoblin.session.onBell",
            "gnoblin.runtime.onConfigReloaded",
            "gnoblin.clients.onAdded",
            "gnoblin.clients.onTitleChanged",
            "gnoblin.outputs.onModeChanged",
        ] {
            assert!(!surface.provides(name), "{name} is the TypeScript library's spelling");
        }
    }

    /// A listener, and the objects that take it, as the library declares them: the subscriptions are generated from the
    /// listener class by `Subscriptions`, `Prefixed` and `MemberSubscriptions`, and a formatter may break a header.
    const SUBSCRIPTIONS: &str = r#"
export class ClientListener {
    onTitleChanged?(client: Client, title: string): void;
    onClosed?(client: Client): void;
}
export class CollectionListener<M> {
    onAdded?(member: M, collection: Collection<M>): void;
}
export class OutputListener {
    onModeChanged?(output: Output, mode: string): void;
}
export class Client {}
export interface Client extends Subscriptions<ClientListener> {}
export class Collection<T> {}
export interface Collection<T> extends Subscriptions<CollectionListener<T>>, MemberSubscriptions<T> {}
export class Gnoblin {
    readonly clients: Collection<Client>;
}
export interface Gnoblin
    extends
        Prefixed<"Client", ClientListener>,
        Prefixed<"Output", OutputListener> {
    readonly onClientCreated: Subscribe<CollectionListener<Client>, "onAdded">;
}
"#;

    #[test]
    fn generated_subscriptions_are_members_and_methods_of_the_objects_that_take_the_listener() {
        let surface = parse_declarations(SUBSCRIPTIONS);
        for name in [
            "gnoblin.onClientCreated",
            "gnoblin.onClientTitleChanged",
            "gnoblin.onClientClosed",
            "gnoblin.onOutputModeChanged",
            "gnoblin.clients.onAdded",
            "gnoblin.clients.onTitleChanged",
            "gnoblin.clients.onModeChanged",
            ".onClientCreated",
            ".onClientTitleChanged",
            ".onOutputModeChanged",
            ".onTitleChanged",
        ] {
            assert!(surface.provides(name), "{name}: {surface:?}");
        }
        for name in [
            "gnoblin.onClientModeChanged",
            "gnoblin.onOutputTitleChanged",
            "gnoblin.onClient",
        ] {
            assert!(!surface.provides(name), "{name} is not generated");
        }
    }

    /// The declarations a native shell gets (`object-types`): two-space indent, accessors as properties, a generated `on`, and the
    /// members `gnoblin.std` adds to the core's classes in a `declare module` block.
    const NATIVE_DECLARATIONS: &str = r#"
export declare class Gnoblin {
  static get(): Gnoblin;
  readonly clients: ClientCollection | undefined;
  shell(command: string): Process;
  on<K extends keyof GnoblinEvents>(event: K, handler: (...args: GnoblinEvents[K]) => void): Listener;
}
export declare class ClientCollection {
  toList(): Client[];
}
export declare class Workspaces {
  readonly list: Workspace[];
  byId(id: string): Workspace | undefined;
  static formatMode(width: number, height: number): string;
}

declare module "gnoblin-core/generated/objects" {
  /** What `gnoblin.std` adds to the core's `Gnoblin`. */
  interface Gnoblin {
    readonly workspaces: Workspaces;
    show(workspace: Workspace): void;
  }
}
"#;

    #[test]
    fn the_members_of_two_space_native_declarations_are_read() {
        let surface = parse_declarations(NATIVE_DECLARATIONS);
        for name in [
            "gnoblin.clients",
            "gnoblin.shell",
            "gnoblin.on",
            "gnoblin.workspaces",
            "gnoblin.workspaces.list",
            "gnoblin.workspaces.byId",
            "gnoblin.show",
            ".toList",
            ".shell",
            ".on",
            ".byId",
            ".formatMode",
            ".show",
        ] {
            assert!(surface.provides(name), "{name}");
        }
        assert!(!surface.provides("gnoblin.workspaces.no_such_member"));
        assert!(!surface.provides(".explode"));
    }

    #[test]
    fn the_native_surface_alone_provides_the_members_the_pages_use() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the xtask lives in the repository");
        let native = parse_declarations(&native_surface(root).expect("the library reads"));
        for name in [
            ".toList",
            ".showById",
            ".snapTo",
            "gnoblin.workspaces.list",
            "gnoblin.shell",
        ] {
            assert!(native.provides(name), "{name} is not in the native surface");
        }
        assert!(!native.provides("gnoblin.workspaces.no_such_member"));
    }
}

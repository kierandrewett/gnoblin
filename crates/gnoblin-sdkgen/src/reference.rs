//! The API reference pages: every class of the two object catalogs as Markdown, for TypeScript, Python and Rust.
//!
//! The pages are written by `cargo xtask docs-reference` into the website's build input and are never committed. Nothing here describes
//! a signature of its own. A declaration on a page is a slice of the text the SDK targets render (`object-types` for the TypeScript and
//! Python declarations, `object-api` for the Rust API), so a page cannot name a member the SDK lacks or spell one differently from the
//! file a type checker reads. What this module adds is:
//!
//! - the descriptions: the comment above each class, property, method and signal in the schema files, which the catalog does not carry;
//! - the member tables, from the catalog, with each member in the language's own spelling.
//!
//! A class the catalog declares that a language's SDK has no declaration for is an error, so a change of the generators that would
//! silently empty a page fails the build instead.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use gnoblin_schema::{Method, Namespace, ObjectKind, ObjectType, Property, Signal};

use super::model::MODEL_DIRS;
use super::object_script::{module_of, package_of};
use super::text::{camel, pascal, python_ident, rust_ident, snake};
use super::{SdkSpec, files_of, render, values};

/// The languages of the SDKs, in the order of the sidebar and of a documentation page's language tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocLanguage {
    TypeScript,
    Python,
    Rust,
}

impl DocLanguage {
    /// Every language, in order.
    pub const ALL: [DocLanguage; 3] = [DocLanguage::TypeScript, DocLanguage::Python, DocLanguage::Rust];

    /// The directory of the language's reference pages, and the word that opens a fence of its code.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            DocLanguage::TypeScript => "typescript",
            DocLanguage::Python => "python",
            DocLanguage::Rust => "rust",
        }
    }

    /// The name shown to a reader.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DocLanguage::TypeScript => "TypeScript",
            DocLanguage::Python => "Python",
            DocLanguage::Rust => "Rust",
        }
    }
}

/// What a schema file says about the classes of one namespace.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Docs {
    classes: BTreeMap<String, ClassDocs>,
    enums: BTreeMap<String, String>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ClassDocs {
    text: String,
    /// Properties, methods and signals by their schema name.
    members: BTreeMap<String, String>,
}

/// The header a `name =` line is waited for under.
enum WaitingName {
    Class(String),
    Enum(String),
    Member(String),
    Other,
}

/// Joins the lines of a comment: a run of lines is one paragraph, and an empty comment line (`#`) ends it.
fn join_comment(lines: &[String]) -> String {
    let mut paragraphs: Vec<String> = vec![String::new()];
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            if !paragraphs.last().is_some_and(String::is_empty) {
                paragraphs.push(String::new());
            }
        } else {
            let last = paragraphs.last_mut().expect("a paragraph is always open");
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(line);
        }
    }
    paragraphs.retain(|paragraph| !paragraph.is_empty());
    paragraphs.join("\n\n")
}

/// The value of `name = "x"`, when the line is that.
fn name_value(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("name")?.trim_start().strip_prefix('=')?.trim();
    rest.strip_prefix('"')?.split('"').next()
}

/// Reads the comments of one schema file. The comment lines directly above a table header are that table's description; the
/// table's name is the first `name =` line under the header. Parameters, fields, variants and augments are not documented.
pub(crate) fn parse_docs(text: &str) -> (Option<String>, Docs) {
    let mut docs = Docs::default();
    let mut namespace = None;
    let mut pending: Vec<String> = Vec::new();
    let mut waiting = WaitingName::Other;
    let mut class: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(comment) = line.strip_prefix('#') {
            pending.push(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            continue;
        }
        if line.is_empty() {
            pending.clear();
            continue;
        }
        if let Some(header) = line.strip_prefix("[[").and_then(|rest| rest.strip_suffix("]]")) {
            let doc = join_comment(&pending);
            pending.clear();
            waiting = match header {
                "object" => WaitingName::Class(doc),
                "enum" => WaitingName::Enum(doc),
                "object.property" | "object.method" | "object.signal" if class.is_some() => WaitingName::Member(doc),
                "augment" => {
                    class = None;
                    WaitingName::Other
                }
                _ => WaitingName::Other,
            };
            continue;
        }
        pending.clear();
        if line.starts_with('[') {
            waiting = WaitingName::Other;
            continue;
        }
        if let Some(value) = line
            .strip_prefix("namespace")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
        {
            namespace = value
                .trim()
                .strip_prefix('"')
                .and_then(|v| v.split('"').next())
                .map(str::to_owned);
            continue;
        }
        let Some(name) = name_value(line) else { continue };
        match std::mem::replace(&mut waiting, WaitingName::Other) {
            WaitingName::Class(doc) => {
                class = Some(name.to_owned());
                docs.classes.entry(name.to_owned()).or_default().text = doc;
            }
            WaitingName::Enum(doc) => {
                docs.enums.insert(name.to_owned(), doc);
            }
            WaitingName::Member(doc) => {
                if let Some(class) = &class {
                    docs.classes
                        .entry(class.clone())
                        .or_default()
                        .members
                        .insert(name.to_owned(), doc);
                }
            }
            WaitingName::Other => {}
        }
    }
    (namespace, docs)
}

/// The comments of every schema file of the two catalogs, by namespace.
fn load_docs(root: &Path) -> Result<BTreeMap<String, Docs>, String> {
    let mut all: BTreeMap<String, Docs> = BTreeMap::new();
    for dir in MODEL_DIRS {
        let dir = root.join(dir);
        let mut files: Vec<_> = fs::read_dir(&dir)
            .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "toml"))
            .collect();
        files.sort();
        for file in files {
            let text = fs::read_to_string(&file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
            let (namespace, docs) = parse_docs(&text);
            let Some(namespace) = namespace else { continue };
            let into = all.entry(namespace).or_default();
            for (name, class) in docs.classes {
                let merged = into.classes.entry(name).or_default();
                if !class.text.is_empty() {
                    merged.text = class.text;
                }
                merged.members.extend(class.members);
            }
            into.enums.extend(docs.enums);
        }
    }
    Ok(all)
}

/// The text of a comment in a table cell or a paragraph: raw `<` and `|` would be read as HTML and as a column, outside and inside
/// code alike.
fn escape(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for c in text.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                out.push(c);
            }
            '|' => out.push_str("\\|"),
            '<' if !in_code => out.push_str("&lt;"),
            '*' if !in_code => out.push_str("\\*"),
            _ => out.push(c),
        }
    }
    out
}

/// The blocks of a TypeScript declaration file by the name they declare, and the `declare module` blocks in file order.
#[derive(Default)]
struct TsBlocks {
    named: BTreeMap<String, String>,
    modules: Vec<String>,
}

fn ts_blocks(text: &str) -> TsBlocks {
    let mut blocks = TsBlocks::default();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if !(line.starts_with("export ") || line.starts_with("declare module ")) {
            index += 1;
            continue;
        }
        let start = index;
        // A block ends at the closing brace in the first column, or at the first line that ends a statement.
        let multi = line.ends_with('{');
        loop {
            let current = lines[index];
            let done = if multi {
                current == "}" || current == "};"
            } else {
                current.ends_with(';')
            };
            index += 1;
            if done || index >= lines.len() {
                break;
            }
        }
        let body = lines[start..index].join("\n");
        if line.starts_with("declare module ") {
            blocks.modules.push(body);
            continue;
        }
        let mut words = line.split_whitespace().skip(1);
        let mut word = words.next();
        while matches!(word, Some("declare" | "abstract" | "default")) {
            word = words.next();
        }
        if matches!(
            word,
            Some("class" | "interface" | "type" | "const" | "enum" | "function")
        ) && let Some(name) = words.next()
        {
            let name: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            blocks.named.insert(name, body);
        }
    }
    blocks
}

/// The top-level blocks of a Python stub by the name they declare: a class with its decorators, or an alias.
fn py_blocks(text: &str) -> BTreeMap<String, String> {
    let mut blocks = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let top = !line.is_empty() && !line.starts_with(char::is_whitespace);
        if !top {
            index += 1;
            continue;
        }
        let start = index;
        let mut decorated = line.starts_with('@');
        while decorated {
            index += 1;
            decorated = lines.get(index).is_some_and(|next| next.starts_with('@'));
        }
        let header = lines.get(index).copied().unwrap_or("");
        let name: Option<String> = if let Some(rest) = header.strip_prefix("class ") {
            Some(rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        } else if header.starts_with(|c: char| c.is_alphabetic() || c == '_')
            && (header.contains(" = ") || header.contains(": "))
        {
            Some(
                header
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect(),
            )
        } else {
            None
        };
        index += 1;
        while lines
            .get(index)
            .is_some_and(|next| next.is_empty() || next.starts_with(char::is_whitespace))
        {
            index += 1;
        }
        let mut end = index;
        while end > start && lines[end - 1].is_empty() {
            end -= 1;
        }
        if let Some(name) = name {
            blocks.insert(name, lines[start..end].join("\n"));
        }
    }
    blocks
}

/// What the Rust API declares for one name: a struct, an enum or a trait, and the public functions of its `impl` blocks.
#[derive(Default)]
struct RustItem {
    head: Vec<String>,
    functions: Vec<String>,
}

/// The Rust items of an unformatted `object-api` file by name. A signature is one line, which the generator writes from a template.
fn rust_items(text: &str) -> BTreeMap<String, RustItem> {
    let mut items: BTreeMap<String, RustItem> = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let declared = ["pub struct ", "pub enum ", "pub trait "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix).map(|rest| (*prefix, rest)));
        if let Some((prefix, rest)) = declared {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let mut body = vec![line.to_owned()];
            if line.ends_with('{') {
                index += 1;
                while index < lines.len() && lines[index] != "}" {
                    body.push(lines[index].to_owned());
                    index += 1;
                }
                body.push("}".to_owned());
            }
            let item = items.entry(name).or_default();
            if prefix == "pub trait " {
                // A trait's signatures are its own lines.
                item.head = vec![line.trim_end_matches('{').trim_end().to_owned()];
                item.functions = body
                    .iter()
                    .filter(|l| l.starts_with("    fn "))
                    .map(|l| l.trim_end_matches(';').to_owned())
                    .collect();
            } else {
                item.head = body;
            }
        } else if let Some(rest) = line.strip_prefix("impl ")
            && let Some(name) = rest.strip_suffix(" {")
            && !name.contains(' ')
            && !name.contains('<')
        {
            let mut functions = Vec::new();
            index += 1;
            while index < lines.len() && lines[index] != "}" {
                let inner = lines[index];
                if inner.starts_with("    pub fn ") {
                    functions.push(inner.trim_end_matches('{').trim_end().to_owned());
                }
                index += 1;
            }
            items.entry(name.to_owned()).or_default().functions.extend(functions);
        }
        index += 1;
    }
    items
}

/// A Rust item as the reference shows it: the public fields only, then the public functions.
fn rust_text(name: &str, item: &RustItem) -> String {
    let mut out = String::new();
    let first = item.head.first().map_or("", String::as_str);
    if first.starts_with("pub trait ") {
        let _ = writeln!(out, "{first} {{");
        for function in &item.functions {
            let _ = writeln!(out, "{function};");
        }
        out.push_str("}\n");
        return tidy_rust(&out);
    }
    if first.ends_with('{') {
        let is_enum = first.starts_with("pub enum ");
        let body: Vec<&str> = item
            .head
            .iter()
            .skip(1)
            .map(String::as_str)
            .filter(|line| {
                let line = line.trim_start();
                line != "}"
                    && !line.starts_with("#[")
                    && !line.starts_with("///")
                    && (is_enum || line.starts_with("pub "))
            })
            .collect();
        if body.is_empty() {
            let _ = writeln!(out, "{first} /* private */ }}");
        } else {
            let _ = writeln!(out, "{first}");
            for line in body {
                let _ = writeln!(out, "{line}");
            }
            out.push_str("}\n");
        }
    } else if !first.is_empty() {
        let _ = writeln!(out, "{first}");
    }
    if !item.functions.is_empty() {
        let _ = writeln!(out, "\nimpl {name} {{");
        for function in &item.functions {
            let _ = writeln!(out, "{function};");
        }
        out.push_str("}\n");
    }
    tidy_rust(&out)
}

/// The generated code calls the shared crates `go`; a reader sees the names without that alias.
fn tidy_rust(text: &str) -> String {
    text.replace("go::typed::", "")
        .replace("go::client::", "")
        .replace("go::", "")
}

/// The spelling of a property in a language.
fn property_name(lang: DocLanguage, property: &Property) -> String {
    match lang {
        DocLanguage::TypeScript => values::js_field(property),
        DocLanguage::Python => values::py_field(property),
        DocLanguage::Rust => values::rust_field(property),
    }
}

fn method_name(lang: DocLanguage, method: &Method) -> String {
    match lang {
        DocLanguage::TypeScript => method.extras.export.js.clone().unwrap_or_else(|| camel(&method.name)),
        DocLanguage::Python => python_ident(&method.extras.export.py.clone().unwrap_or_else(|| snake(&method.name))),
        DocLanguage::Rust => rust_ident(&method.extras.export.rust.clone().unwrap_or_else(|| snake(&method.name))),
    }
}

/// The name a signal is listened to by: the key of the `<Class>Events` interface in TypeScript, the string `on` takes in Python.
fn signal_name(lang: DocLanguage, signal: &Signal) -> String {
    match lang {
        DocLanguage::TypeScript => signal.extras.export.js.clone().unwrap_or_else(|| camel(&signal.name)),
        DocLanguage::Python => signal.extras.export.py.clone().unwrap_or_else(|| snake(&signal.name)),
        DocLanguage::Rust => format!(
            "on_{}",
            rust_ident(&signal.extras.export.rust.clone().unwrap_or_else(|| snake(&signal.name)))
        ),
    }
}

fn class_name(lang: DocLanguage, object: &ObjectType) -> String {
    match lang {
        DocLanguage::TypeScript => values::js_class(&object.name, &object.export),
        DocLanguage::Python => values::py_class(&object.name, &object.export),
        DocLanguage::Rust => values::rust_class(&object.name, &object.export),
    }
}

/// The declaration text of every class in one language and namespace.
enum Slices {
    Dts(TsBlocks),
    Stubs(BTreeMap<String, String>),
    Api(BTreeMap<String, RustItem>),
}

impl Slices {
    /// The declaration of `name`, with the option and event types the language declares beside it.
    fn find(&self, name: &str, extras: &[&str]) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        match self {
            Slices::Dts(blocks) => {
                parts.push(blocks.named.get(name)?.clone());
                for suffix in ["Options", "CreateOptions", "Init", "Events", "Literal"] {
                    if let Some(block) = blocks.named.get(&format!("{name}{suffix}")) {
                        parts.push(block.clone());
                    }
                }
                for extra in extras {
                    if let Some(block) = blocks.named.get(*extra) {
                        parts.push(block.clone());
                    }
                }
            }
            Slices::Stubs(blocks) => {
                parts.push(blocks.get(name)?.clone());
                for extra in extras {
                    if let Some(block) = blocks.get(*extra) {
                        parts.push(block.clone());
                    }
                }
            }
            Slices::Api(items) => {
                parts.push(rust_text(name, items.get(name)?));
                for companion in [format!("{name}Options"), format!("{name}Handlers")] {
                    if let Some(item) = items.get(&companion) {
                        parts.push(rust_text(&companion, item));
                    }
                }
                for extra in extras {
                    if let Some(item) = items.get(*extra) {
                        parts.push(rust_text(extra, item));
                    }
                }
            }
        }
        Some(parts.join("\n\n"))
    }

    /// The blocks that add members to a class of the core: a `declare module` in TypeScript, `<Class>Augment` in Python and the
    /// extension trait `<Class>StdExt` in Rust.
    fn extensions(&self) -> Vec<String> {
        match self {
            Slices::Dts(blocks) => blocks.modules.clone(),
            Slices::Stubs(blocks) => blocks
                .iter()
                .filter(|(name, _)| name.ends_with("Augment"))
                .map(|(_, block)| block.clone())
                .collect(),
            Slices::Api(items) => items
                .iter()
                .filter(|(name, _)| name.ends_with("StdExt"))
                .map(|(name, item)| rust_text(name, item))
                .collect(),
        }
    }
}

struct PageSource<'a> {
    lang: DocLanguage,
    namespace: &'a Namespace,
    declarations: Slices,
}

fn kind_line(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Handle => "A handle: the library holds the object, and your code holds a reference to it.",
        ObjectKind::Service => "A service: the library makes one, and a function of the class gets it.",
        ObjectKind::Record => "A record: a value that is copied when it is read. Write it back to change it.",
        ObjectKind::Interface => {
            "An interface: your code implements the methods it wants to answer and gives the object to the library."
        }
    }
}

/// The comment of a member, from the class that declares it or the nearest base that does.
fn member_doc<'a>(
    docs: &'a Docs,
    spec: &'a SdkSpec,
    namespace: &Namespace,
    object: &ObjectType,
    name: &str,
) -> &'a str {
    let mut current = Some(object.name.as_str());
    while let Some(class) = current {
        if let Some(text) = docs.classes.get(class).and_then(|c| c.members.get(name)) {
            return text;
        }
        current = spec
            .objects
            .objects(namespace)
            .find(|candidate| candidate.name == class)
            .and_then(|candidate| candidate.base.as_deref());
    }
    ""
}

fn member_table(source: &PageSource<'_>, spec: &SdkSpec, docs: &Docs, object: &ObjectType) -> String {
    let lang = source.lang;
    let mut rows = String::new();
    let mut row = |name: String, kind: &str, doc: &str| {
        let _ = writeln!(rows, "| `{name}` | {kind} | {} |", escape(doc).replace('\n', " "));
    };
    for property in &object.properties {
        let kind = match (property.readonly, property.animated) {
            (true, _) => "read-only property",
            (false, true) => "property, animated",
            (false, false) => "property",
        };
        row(
            property_name(lang, property),
            kind,
            member_doc(docs, spec, source.namespace, object, &property.name),
        );
    }
    for method in &object.methods {
        let kind = if method.function { "function" } else { "method" };
        row(
            format!("{}()", method_name(lang, method)),
            kind,
            member_doc(docs, spec, source.namespace, object, &method.name),
        );
    }
    for signal in &object.signals {
        row(
            signal_name(lang, signal),
            "signal",
            member_doc(docs, spec, source.namespace, object, &signal.name),
        );
    }
    if rows.is_empty() {
        return rows;
    }
    format!("| Member | Kind | What it does |\n| --- | --- | --- |\n{rows}")
}

fn intro(lang: DocLanguage, package: &str, module: &str) -> String {
    let is_core = package == "gnoblin-core";
    match lang {
        DocLanguage::TypeScript => format!(
            "Import a class from `\"{package}\"`. A property is a property, a method is a method, and a signal is an event you listen to \
             with `on(\"name\", handler)`, which returns a `Listener`. Members are `camelCase`.\n\n```typescript\nimport {{ /* names from this page */ }} from \"{package}\";\n```\n"
        ),
        DocLanguage::Python => format!(
            "Import a class from `gnoblin.objects.{module}`. A property is an attribute, a method is a method, and a signal is an event you \
             listen to with `on(\"name\", handler)`, which returns a `Listener`. Members are `snake_case`.\n\n```python\nfrom gnoblin.objects.{module} import ...  # names from this page\n```\n"
        ),
        DocLanguage::Rust => {
            let krate = if is_core { "gnoblin_core" } else { "gnoblin_std" };
            format!(
                "Import a type from `{krate}::objects`. A property is a getter `name()` and, when it is writable, a setter `set_name(value)`. \
                 A signal is a method `on_name(run)` that returns a `Listener`, which stops listening when it is dropped. Every call returns \
                 `Res<T>`.\n\n```rust\nuse {krate}::objects::{{/* names from this page */}};\n```\n"
            )
        }
    }
}

fn render_page(spec: &SdkSpec, docs: &Docs, source: &PageSource<'_>) -> Result<String, String> {
    let lang = source.lang;
    let namespace = source.namespace;
    let package = package_of(namespace)?;
    let module = module_of(namespace)?;
    let library = if package == "gnoblin-core" { "core" } else { "std" };
    let mut out = String::new();
    let _ = writeln!(out, "# {} reference: {library}\n", lang.label());
    let _ = writeln!(
        out,
        "Every type of the {library} library as the {} SDK declares it. The declarations on this page are the ones a type checker reads, written from the schema, and \
         the descriptions are the schema's own comments.\n",
        lang.label()
    );
    out.push_str(&intro(lang, package, module));
    out.push('\n');

    let mut objects: Vec<&ObjectType> = spec
        .objects
        .objects(namespace)
        .filter(|object| object.augment_of.is_none())
        .collect();
    objects.sort_by_key(|object| class_name(lang, object));
    for object in objects {
        let name = class_name(lang, object);
        let variants: Vec<String> = object
            .variants
            .iter()
            .map(|variant| match lang {
                DocLanguage::TypeScript => values::js_class(&variant.name, &variant.export),
                DocLanguage::Python => values::py_class(&variant.name, &variant.export),
                DocLanguage::Rust => values::rust_class(&variant.name, &variant.export),
            })
            .collect();
        let extras: Vec<&str> = variants.iter().map(String::as_str).collect();
        let declaration = source.declarations.find(&name, &extras).ok_or_else(|| {
            format!(
                "the {} SDK declares no `{name}`, the class `{}` of `{namespace}`: the reference cannot show it",
                lang.label(),
                object.name
            )
        })?;
        let _ = writeln!(out, "## {name}\n");
        if let Some(text) = docs
            .classes
            .get(&object.name)
            .map(|c| c.text.as_str())
            .filter(|t| !t.is_empty())
        {
            let _ = writeln!(out, "{}\n", escape(text));
        }
        let mut kind = kind_line(object.kind).to_owned();
        if let Some(base) = &object.base
            && let Some(base) = spec
                .objects
                .objects(namespace)
                .find(|candidate| &candidate.name == base)
        {
            let _ = write!(kind, " It extends `{}`, and has its members.", class_name(lang, base));
        }
        let _ = writeln!(out, "{kind}\n");
        let table = member_table(source, spec, docs, object);
        if !table.is_empty() {
            let _ = writeln!(out, "{table}");
        }
        let _ = writeln!(out, "```{} plain\n{}\n```\n", lang.slug(), declaration.trim_end());
    }

    let mut enums: Vec<&gnoblin_schema::EnumDecl> = spec.objects.enums(namespace).collect();
    enums.sort_by_key(|declared| declared.name.clone());
    if !enums.is_empty() {
        out.push_str("## Enums\n\nA value is one of the names listed. ");
        out.push_str(match lang {
            DocLanguage::TypeScript => "TypeScript takes the name as a string.\n\n",
            DocLanguage::Python => "Python takes the member of the `IntEnum`, or the name as a string.\n\n",
            DocLanguage::Rust => "Rust takes the variant of the enum.\n\n",
        });
        out.push_str("| Enum | Values | What it is |\n| --- | --- | --- |\n");
        let mut declared_text = Vec::new();
        for declared in &enums {
            let name = pascal(&declared.name);
            let doc = docs.enums.get(&declared.name).map_or("", String::as_str);
            let values: Vec<String> = declared.values.iter().map(|value| format!("`{value}`")).collect();
            let _ = writeln!(out, "| `{name}` | {} | {} |", values.join(", "), escape(doc));
            if let Some(text) = source.declarations.find(&name, &[]) {
                declared_text.push(text);
            }
        }
        if !declared_text.is_empty() {
            let _ = write!(out, "\n```{} plain\n{}\n```\n", lang.slug(), declared_text.join("\n\n"));
        }
        out.push('\n');
    }

    let extensions = source.declarations.extensions();
    if !extensions.is_empty() {
        out.push_str(
            "## Members added to the core\n\nThe library adds these members to classes of the core library. \
             A shell that does not load this library has none of them.\n\n",
        );
        let _ = write!(out, "```{} plain\n{}\n```\n", lang.slug(), extensions.join("\n\n"));
    }
    Ok(out)
}

fn render_index() -> String {
    let mut out = String::from(
        "# API reference\n\nThe classes a shell uses to read and change the desktop, in the three languages a shell can be written in. \
         Each page is written from the schema catalog when the site is built, so it names the members the SDK has and no others.\n\n\
         The core library is the compositor's own objects: windows, outputs, input, the session and processes. The std library is the desktop above it: \
         workspaces, layouts, the scene, styles and widgets.\n\n| Language | Core | Std |\n| --- | --- | --- |\n",
    );
    for lang in DocLanguage::ALL {
        let _ = writeln!(
            out,
            "| {} | [Core](./{slug}/core.md) | [Std](./{slug}/std.md) |",
            lang.label(),
            slug = lang.slug()
        );
    }
    out.push_str(
        "\nThe guides explain how to use them: [TypeScript shell](../typescript-shell.md), [Python shell](../python-shell.md) and \
         [Rust shell](../rust-shell.md). [The SDK generator](../sdk-generator.md) says where the declarations come from.\n",
    );
    out
}

/// Renders every reference page: `(path under the site's page root, Markdown)`. The paths are `reference/index.md` and
/// `reference/<language>/<core|std>.md`.
///
/// # Errors
///
/// The specification was not loaded from a workspace (the descriptions are in its schema files), a file cannot be read, or an SDK has no
/// declaration for a class of the catalog.
pub fn render_reference(spec: &SdkSpec) -> Result<Vec<(String, String)>, String> {
    let root = spec
        .root
        .as_deref()
        .ok_or("the reference needs the schema files of a workspace: load the specification with `SdkSpec::load`")?;
    let docs = load_docs(root)?;
    let types = files_of(&render(spec, Some("object-types"))?);
    let api = files_of(&render(spec, Some("object-api"))?);
    let text_of = |files: &[(String, String)], path: &str| -> Result<String, String> {
        files
            .iter()
            .find(|(file, _)| file == path)
            .map(|(_, text)| text.clone())
            .ok_or_else(|| format!("the SDK renders no `{path}`"))
    };

    let mut pages = vec![("reference/index.md".to_owned(), render_index())];
    for namespace in spec.objects.namespaces() {
        let package = package_of(&namespace)?;
        let module = module_of(&namespace)?;
        let library = if package == "gnoblin-core" { "core" } else { "std" };
        let empty = Docs::default();
        let docs = docs.get(namespace.as_str()).unwrap_or(&empty);
        for lang in DocLanguage::ALL {
            let declarations = match lang {
                DocLanguage::TypeScript => Slices::Dts(ts_blocks(&text_of(
                    &types,
                    &format!("js/{package}/generated/objects.d.ts"),
                )?)),
                DocLanguage::Python => Slices::Stubs(py_blocks(&text_of(
                    &types,
                    &format!("python/gnoblin/objects/{module}.pyi"),
                )?)),
                DocLanguage::Rust => Slices::Api(rust_items(&text_of(&api, &format!("rust-objects/{module}_api.rs"))?)),
            };
            let source = PageSource {
                lang,
                namespace: &namespace,
                declarations,
            };
            pages.push((
                format!("reference/{}/{library}.md", lang.slug()),
                render_page(spec, docs, &source)?,
            ));
        }
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;

    #[test]
    fn a_comment_above_a_table_describes_it_and_a_parameter_is_not_a_member() {
        let (namespace, docs) = parse_docs(
            "# The file.\nnamespace = \"gnoblin.model\"\n\n# One watch.\n#\n# It ends.\n[[object]]\nname = \"idle_watch\"\n\n# True while idle.\n[[object.property]]\nname = \"is_idle\"\n\n# Ends it.\n[[object.method]]\nname = \"cancel\"\n\n[[object.method.param]]\nname = \"reason\"\n\n# Quiet.\n[[enum]]\nname = \"quiet\"\n",
        );
        assert_eq!(namespace.as_deref(), Some("gnoblin.model"));
        let class = &docs.classes["idle_watch"];
        assert_eq!(class.text, "One watch.\n\nIt ends.");
        assert_eq!(class.members["is_idle"], "True while idle.");
        assert_eq!(class.members["cancel"], "Ends it.");
        assert!(!class.members.contains_key("reason"));
        assert_eq!(docs.enums["quiet"], "Quiet.");
    }

    #[test]
    fn a_comment_separated_by_a_blank_line_describes_nothing() {
        let (_, docs) = parse_docs("namespace = \"x.y\"\n\n# A note.\n\n[[object]]\nname = \"a\"\n");
        assert_eq!(docs.classes["a"].text, "");
    }

    #[test]
    fn the_declarations_of_each_language_are_found_by_name() {
        let ts = ts_blocks(
            "/** The enum. */\nexport type Mode = \"a\" | \"b\";\n\n/** A class. */\nexport declare class Watch {\n  readonly id: string;\n  cancel(): void;\n}\ndeclare module \"x\" {\n  interface Y {\n    z: number;\n  }\n}\n",
        );
        assert_eq!(ts.named["Mode"], "export type Mode = \"a\" | \"b\";");
        assert!(ts.named["Watch"].contains("cancel(): void;"));
        assert_eq!(ts.modules.len(), 1);
        let py = py_blocks(
            "@dataclasses.dataclass\nclass Rect:\n    x: float = ...\n\nclass Watch:\n    def cancel(self) -> None: ...\n",
        );
        assert!(py["Rect"].starts_with("@dataclasses.dataclass\nclass Rect:"));
        assert!(py["Watch"].contains("def cancel"));
        let rust = rust_items(
            "pub struct Watch {\n    object: go::client::Object,\n}\n\nimpl Watch {\n    /// Ends it.\n    pub fn cancel(&self) -> go::Res<()> {\n        Ok(())\n    }\n}\n",
        );
        let text = rust_text("Watch", &rust["Watch"]);
        assert!(text.contains("pub struct Watch { /* private */ }"), "{text}");
        assert!(text.contains("pub fn cancel(&self) -> Res<()>;"), "{text}");
    }

    #[test]
    fn every_class_of_both_catalogs_has_a_section_in_every_language() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let pages = render_reference(&spec).expect("the reference renders");
        assert_eq!(
            pages.len(),
            1 + 2 * 3,
            "the index and a page for each language and library"
        );
        for (path, text) in &pages {
            assert!(text.starts_with("# "), "{path} starts with its title");
        }
        let typescript = &pages
            .iter()
            .find(|(path, _)| path == "reference/typescript/core.md")
            .expect("page")
            .1;
        assert!(typescript.contains("## IdleWatch"), "a class has a section");
        assert!(
            typescript.contains("readonly isIdle: boolean;"),
            "the declaration is the SDK's own"
        );
        assert!(
            typescript.contains("True while the session has been idle"),
            "the schema comment is the description"
        );
        let rust = &pages
            .iter()
            .find(|(path, _)| path == "reference/rust/core.md")
            .expect("page")
            .1;
        assert!(rust.contains("pub fn is_idle(&self) -> Res<bool>;"), "{rust:.2000}");
        let python = &pages
            .iter()
            .find(|(path, _)| path == "reference/python/core.md")
            .expect("page")
            .1;
        assert!(python.contains("def is_idle(self) -> bool: ..."));
    }
}

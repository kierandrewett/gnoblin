//! The descriptions of the classes: the comments the schema files carry above each class, property, method, signal, parameter, signal
//! field, enum, enum value, callback, callback parameter and variant.
//!
//! The catalog (`gnoblin-schema`) does not keep comments, so the generator reads them from the files again. A comment is the lines of `#`
//! directly above a table header; the table's name is the first `name =` line under the header. A blank line between the comment and the
//! header means the comment describes nothing. An enum's values are described by the comment above each value when `values` is written one
//! value to a line:
//!
//! ```toml
//! values = [
//!     # The compositor's own curve.
//!     "default",
//!     # No curve.
//!     "flat",
//! ]
//! ```
//!
//! The keys of a table that are not text (`range`, `unit`, `raises`, `example`) are in the catalog, as [`gnoblin_schema::Notes`].
//!
//! [`Docs`] reaches every target that writes declarations (`object-types` for TypeScript and Python, `object-api` for Rust) and the
//! reference model (`reference.json`), so the description of a class is the same sentence in the editor's hover text, in the generated
//! reference of each language and in the Rust docs. Each target decides how its language writes a comment (`/** */`, a docstring, `///`);
//! this module only holds the text.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_schema::augment_class_name;
use gnoblin_schema::{Namespace, ObjectType};

/// The languages of the SDKs, in the order of a documentation page's language tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocLanguage {
    TypeScript,
    Python,
    Rust,
}

impl DocLanguage {
    /// Every language, in order.
    pub const ALL: [DocLanguage; 3] = [DocLanguage::TypeScript, DocLanguage::Python, DocLanguage::Rust];

    /// The word that opens a fence of the language's code.
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
pub struct NamespaceDocs {
    classes: BTreeMap<String, ClassDocs>,
    enums: BTreeMap<String, EnumDocs>,
    callbacks: BTreeMap<String, ClassDocs>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ClassDocs {
    text: String,
    /// Properties, methods and signals by their schema name.
    members: BTreeMap<String, String>,
    /// The parameters of a method and the fields of a signal, by `member.name`; the parameters of a callback, by name.
    children: BTreeMap<String, String>,
    /// The variants of a family by their name, and their properties by `variant.name`.
    variants: BTreeMap<String, String>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct EnumDocs {
    text: String,
    values: BTreeMap<String, String>,
}

impl NamespaceDocs {
    /// The description of the class `name`, or an empty text.
    pub fn class(&self, name: &str) -> &str {
        self.classes.get(name).map_or("", |class| class.text.as_str())
    }

    /// The description of the enum `name`, or an empty text.
    pub fn enum_doc(&self, name: &str) -> &str {
        self.enums.get(name).map_or("", |declared| declared.text.as_str())
    }

    /// The description of the value `value` of the enum `name`, or an empty text.
    pub fn enum_value(&self, name: &str, value: &str) -> &str {
        self.enums
            .get(name)
            .and_then(|declared| declared.values.get(value))
            .map_or("", String::as_str)
    }

    /// The description of the callback `name`, or an empty text.
    pub fn callback(&self, name: &str) -> &str {
        self.callbacks.get(name).map_or("", |declared| declared.text.as_str())
    }

    /// The description of the parameter `param` of the callback `name`, or an empty text.
    pub fn callback_param(&self, name: &str, param: &str) -> &str {
        self.callbacks
            .get(name)
            .and_then(|declared| declared.children.get(param))
            .map_or("", String::as_str)
    }

    /// The description of the member `member` of the class `class`, from the class that declares it or the nearest base that does.
    /// `base_of` names the base of a class.
    pub fn member<'a>(&'a self, class: &str, member: &str, base_of: impl Fn(&str) -> Option<&'a str>) -> &'a str {
        let mut owner = class;
        loop {
            if let Some(text) = self.classes.get(owner).and_then(|c| c.members.get(member)) {
                return text;
            }
            match base_of(owner) {
                Some(base) => owner = base,
                None => return "",
            }
        }
    }

    /// The description of the parameter of a method or the field of a signal, `child`, of the member `member` of `class`, from the class
    /// that declares the member or the nearest base that does.
    pub fn child<'a>(
        &'a self,
        class: &str,
        member: &str,
        child: &str,
        base_of: impl Fn(&str) -> Option<&'a str>,
    ) -> &'a str {
        let key = format!("{member}.{child}");
        let mut owner = class;
        loop {
            if let Some(class) = self.classes.get(owner)
                && class.members.contains_key(member)
            {
                return class.children.get(&key).map_or("", String::as_str);
            }
            match base_of(owner) {
                Some(base) => owner = base,
                None => return "",
            }
        }
    }

    /// The description of the variant `variant` of the family `class`, or an empty text.
    pub fn variant(&self, class: &str, variant: &str) -> &str {
        self.classes
            .get(class)
            .and_then(|class| class.variants.get(variant))
            .map_or("", String::as_str)
    }

    /// The description of the property `property` of the variant `variant` of the family `class`, or an empty text.
    pub fn variant_property(&self, class: &str, variant: &str, property: &str) -> &str {
        self.classes
            .get(class)
            .and_then(|class| class.variants.get(&format!("{variant}.{property}")))
            .map_or("", String::as_str)
    }

    fn merge(&mut self, other: NamespaceDocs) {
        for (name, class) in other.classes {
            let merged = self.classes.entry(name).or_default();
            if !class.text.is_empty() {
                merged.text = class.text;
            }
            merged.members.extend(class.members);
            merged.children.extend(class.children);
            merged.variants.extend(class.variants);
        }
        self.enums.extend(other.enums);
        self.callbacks.extend(other.callbacks);
    }
}

/// The description of every namespace the libraries serve.
#[derive(Default, Debug)]
pub struct Docs {
    namespaces: BTreeMap<String, NamespaceDocs>,
}

/// The descriptions of a namespace that has none: what a catalog without schema comments (a test fixture) is rendered with.
pub(crate) static NO_DOCS: NamespaceDocs = NamespaceDocs {
    classes: BTreeMap::new(),
    enums: BTreeMap::new(),
    callbacks: BTreeMap::new(),
};

impl Docs {
    /// Reads the comments of every `.toml` file under `dirs`, and of `extra` files (an origin and its text) that are not on disk.
    ///
    /// # Errors
    ///
    /// A directory or a file cannot be read.
    pub fn load(dirs: &[PathBuf], extra: &[&str]) -> Result<Docs, String> {
        let mut docs = Docs::default();
        for dir in dirs {
            let mut files: Vec<PathBuf> = fs::read_dir(dir)
                .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "toml"))
                .collect();
            files.sort();
            for file in files {
                docs.add(&read(&file)?);
            }
        }
        for text in extra {
            docs.add(text);
        }
        Ok(docs)
    }

    fn add(&mut self, text: &str) {
        let (namespace, docs) = parse_docs(text);
        if let Some(namespace) = namespace {
            self.namespaces.entry(namespace).or_default().merge(docs);
        }
    }

    /// The descriptions of `namespace`; empty when its files say nothing.
    pub fn namespace(&self, namespace: &Namespace) -> &NamespaceDocs {
        self.namespaces.get(namespace.as_str()).unwrap_or(&NO_DOCS)
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// The header a `name =` line is waited for under.
enum WaitingName {
    Class(String),
    Enum(String),
    Callback(String),
    CallbackParam(String),
    Member(String),
    /// A parameter of a method or a field of a signal.
    Child(String),
    Variant(String),
    VariantProperty(String),
    /// An `[[augment]]`, which is named by its `target`.
    Target,
    Other,
}

/// Joins the lines of a comment: a run of lines is one paragraph, and an empty comment line (`#`) ends it. A run of lines indented by two
/// spaces or more (a grammar, a table) is kept as it is, as a fenced block of `text`.
fn join_comment(lines: &[String]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let mut paragraph = String::new();
    let mut code: Vec<&str> = Vec::new();
    for line in lines {
        if line.starts_with("  ") && !line.trim().is_empty() {
            if !paragraph.is_empty() {
                blocks.push(std::mem::take(&mut paragraph));
            }
            code.push(line.trim_end());
            continue;
        }
        if !code.is_empty() {
            blocks.push(fence(&code));
            code.clear();
        }
        let line = line.trim();
        if line.is_empty() {
            if !paragraph.is_empty() {
                blocks.push(std::mem::take(&mut paragraph));
            }
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
        }
    }
    if !paragraph.is_empty() {
        blocks.push(paragraph);
    }
    if !code.is_empty() {
        blocks.push(fence(&code));
    }
    blocks.join("\n\n")
}

/// Preformatted lines as a fenced block, without the indentation they all share.
fn fence(lines: &[&str]) -> String {
    let indent = lines
        .iter()
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut text = String::from("```text");
    for line in lines {
        text.push('\n');
        text.push_str(&line[indent..]);
    }
    text.push_str("\n```");
    text
}

/// The value of `key = "x"`, when the line is that.
fn string_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
    rest.strip_prefix('"')?.split('"').next()
}

/// The multi-line string delimiter a line leaves open, if it does: a line with an odd number of `'''` or `"""` opens one or closes one.
fn opens_string(line: &str) -> Option<&'static str> {
    ["'''", "\"\"\""]
        .into_iter()
        .find(|delimiter| line.matches(delimiter).count() % 2 == 1)
}

/// Reads the comments of one schema file: its namespace and what it says about the classes.
pub(crate) fn parse_docs(text: &str) -> (Option<String>, NamespaceDocs) {
    let mut docs = NamespaceDocs::default();
    let mut namespace = None;
    let mut pending: Vec<String> = Vec::new();
    let mut waiting = WaitingName::Other;
    let mut class: Option<String> = None;
    let mut member: Option<String> = None;
    let mut variant: Option<String> = None;
    let mut enumeration: Option<String> = None;
    let mut callback: Option<String> = None;
    // The delimiter of the multi-line string a line is inside: its lines are values, not comments or headers.
    let mut in_string: Option<&'static str> = None;
    // The enum whose `values = [` array is being read, one value to a line.
    let mut values_of: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(delimiter) = in_string {
            if line.matches(delimiter).count() % 2 == 1 {
                in_string = None;
            }
            continue;
        }
        if let Some(name) = &values_of {
            if let Some(comment) = line.strip_prefix('#') {
                pending.push(comment.strip_prefix(' ').unwrap_or(comment).to_owned());
            } else if line.starts_with(']') {
                values_of = None;
                pending.clear();
            } else if let Some(value) = line.strip_prefix('"').and_then(|rest| rest.split('"').next()) {
                let doc = join_comment(&pending);
                if !doc.is_empty() {
                    docs.enums
                        .entry(name.clone())
                        .or_default()
                        .values
                        .insert(value.to_owned(), doc);
                }
                pending.clear();
            } else {
                pending.clear();
            }
            continue;
        }
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
            if header != "enum" {
                enumeration = None;
            }
            waiting = match header {
                "object" => {
                    member = None;
                    variant = None;
                    WaitingName::Class(doc)
                }
                "enum" => WaitingName::Enum(doc),
                "callback" => WaitingName::Callback(doc),
                "callback.param" if callback.is_some() => WaitingName::CallbackParam(doc),
                "object.property" | "object.method" | "object.signal" | "augment.property" | "augment.method"
                    if class.is_some() =>
                {
                    WaitingName::Member(doc)
                }
                "object.method.param" | "object.signal.field" | "augment.method.param"
                    if class.is_some() && member.is_some() =>
                {
                    WaitingName::Child(doc)
                }
                "object.variant" if class.is_some() => WaitingName::Variant(doc),
                "object.variant.property" if class.is_some() && variant.is_some() => WaitingName::VariantProperty(doc),
                "augment" => {
                    class = None;
                    member = None;
                    WaitingName::Target
                }
                _ => WaitingName::Other,
            };
            continue;
        }
        pending.clear();
        if let Some(delimiter) = opens_string(line) {
            in_string = Some(delimiter);
        }
        if line.starts_with('[') {
            waiting = WaitingName::Other;
            continue;
        }
        if let Some(value) = string_value(line, "namespace") {
            namespace = Some(value.to_owned());
            continue;
        }
        if let Some(name) = &enumeration
            && let Some(rest) = line
                .strip_prefix("values")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix('='))
                .map(str::trim)
            && rest.starts_with('[')
            && !rest.contains(']')
        {
            values_of = Some(name.clone());
            continue;
        }
        if matches!(waiting, WaitingName::Target) {
            if let Some(target) = string_value(line, "target")
                && let Some(core_class) = target.strip_prefix("core.")
            {
                class = Some(augment_class_name(core_class));
                waiting = WaitingName::Other;
            }
            continue;
        }
        let Some(name) = string_value(line, "name") else {
            continue;
        };
        match std::mem::replace(&mut waiting, WaitingName::Other) {
            WaitingName::Class(doc) => {
                class = Some(name.to_owned());
                docs.classes.entry(name.to_owned()).or_default().text = doc;
            }
            WaitingName::Enum(doc) => {
                enumeration = Some(name.to_owned());
                docs.enums.entry(name.to_owned()).or_default().text = doc;
            }
            WaitingName::Callback(doc) => {
                callback = Some(name.to_owned());
                docs.callbacks.entry(name.to_owned()).or_default().text = doc;
            }
            WaitingName::CallbackParam(doc) => {
                if let Some(callback) = &callback {
                    docs.callbacks
                        .entry(callback.clone())
                        .or_default()
                        .children
                        .insert(name.to_owned(), doc);
                }
            }
            WaitingName::Member(doc) => {
                member = Some(name.to_owned());
                if let Some(class) = &class {
                    docs.classes
                        .entry(class.clone())
                        .or_default()
                        .members
                        .insert(name.to_owned(), doc);
                }
            }
            WaitingName::Child(doc) => {
                if let (Some(class), Some(member)) = (&class, &member) {
                    docs.classes
                        .entry(class.clone())
                        .or_default()
                        .children
                        .insert(format!("{member}.{name}"), doc);
                }
            }
            WaitingName::Variant(doc) => {
                variant = Some(name.to_owned());
                if let Some(class) = &class {
                    docs.classes
                        .entry(class.clone())
                        .or_default()
                        .variants
                        .insert(name.to_owned(), doc);
                }
            }
            WaitingName::VariantProperty(doc) => {
                if let (Some(class), Some(variant)) = (&class, &variant) {
                    docs.classes
                        .entry(class.clone())
                        .or_default()
                        .variants
                        .insert(format!("{variant}.{name}"), doc);
                }
            }
            WaitingName::Target | WaitingName::Other => {}
        }
    }
    (namespace, docs)
}

/// The base class of `object`, when it is one of the namespace's own.
pub(crate) fn base_of<'a>(objects: &BTreeMap<&'a str, &'a ObjectType>, class: &str) -> Option<&'a str> {
    objects
        .get(class)
        .and_then(|object| object.base.as_deref())
        .filter(|base| !base.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_above_a_table_describes_it_and_a_parameter_is_not_a_member() {
        let (namespace, docs) = parse_docs(
            "# The file.\nnamespace = \"gnoblin.model\"\n\n# One watch.\n#\n# It ends.\n[[object]]\nname = \"idle_watch\"\n\n# True while idle.\n[[object.property]]\nname = \"is_idle\"\n\n# Ends it.\n[[object.method]]\nname = \"cancel\"\n\n# Why.\n[[object.method.param]]\nname = \"reason\"\n\n# Quiet.\n[[enum]]\nname = \"quiet\"\n",
        );
        assert_eq!(namespace.as_deref(), Some("gnoblin.model"));
        let class = &docs.classes["idle_watch"];
        assert_eq!(class.text, "One watch.\n\nIt ends.");
        assert_eq!(class.members["is_idle"], "True while idle.");
        assert_eq!(class.members["cancel"], "Ends it.");
        assert!(!class.members.contains_key("reason"));
        assert_eq!(class.children["cancel.reason"], "Why.");
        assert_eq!(docs.enum_doc("quiet"), "Quiet.");
    }

    #[test]
    fn an_indented_run_of_lines_stays_a_block() {
        let (_, docs) = parse_docs(
            "namespace = \"x.y\"\n\n# A grammar:\n#\n#     a := b\n#       | c\n#\n# That is all.\n[[object]]\nname = \"a\"\n",
        );
        assert_eq!(
            docs.classes["a"].text,
            "A grammar:\n\n```text\na := b\n  | c\n```\n\nThat is all."
        );
    }

    #[test]
    fn a_comment_separated_by_a_blank_line_describes_nothing() {
        let (_, docs) = parse_docs("namespace = \"x.y\"\n\n# A note.\n\n[[object]]\nname = \"a\"\n");
        assert_eq!(docs.classes["a"].text, "");
    }

    #[test]
    fn the_members_of_an_augment_belong_to_its_class() {
        let (_, docs) = parse_docs(
            "namespace = \"gnoblin.std\"\n\n# Not described.\n[[augment]]\ntarget = \"core.client\"\n\n# Starts it.\n[[augment.method]]\nname = \"move_with_keys\"\n\n# Which.\n[[augment.method.param]]\nname = \"keys\"\n\n# Reads it.\n[[augment.property]]\nname = \"style\"\n",
        );
        let class = &docs.classes["client_augment"];
        assert_eq!(class.members["move_with_keys"], "Starts it.");
        assert_eq!(class.members["style"], "Reads it.");
        assert_eq!(class.children["move_with_keys.keys"], "Which.");
        assert_eq!(class.text, "");
    }

    #[test]
    fn a_member_is_described_by_the_nearest_base_that_does() {
        let (_, docs) = parse_docs(
            "namespace = \"x.y\"\n\n# Base.\n[[object]]\nname = \"base\"\n\n# The id.\n[[object.property]]\nname = \"id\"\n\n# Ends.\n[[object.method]]\nname = \"end\"\n\n# Why.\n[[object.method.param]]\nname = \"reason\"\n\n[[object]]\nname = \"derived\"\nbase = \"base\"\n",
        );
        let bases = |class: &str| (class == "derived").then_some("base");
        assert_eq!(docs.member("derived", "id", bases), "The id.");
        assert_eq!(docs.member("derived", "other", bases), "");
        assert_eq!(docs.child("derived", "end", "reason", bases), "Why.");
        assert_eq!(docs.child("derived", "end", "other", bases), "");
    }

    #[test]
    fn a_signal_field_a_callback_a_variant_and_an_enum_value_have_descriptions() {
        let (_, docs) = parse_docs(
            "namespace = \"x.y\"\n\n# The mode.\n[[enum]]\nname = \"mode\"\nsince = \"0.2.0\"\nvalues = [\n    # Quiet.\n    \"quiet\",\n    \"loud\",\n    # Both words.\n    # On two lines.\n    \"both\",\n]\n\n# A hook.\n[[callback]]\nname = \"hook\"\n\n# The event.\n[[callback.param]]\nname = \"event\"\n\n# A family.\n[[object]]\nname = \"fam\"\n\n# Fires.\n[[object.signal]]\nname = \"fired\"\n\n# How hard.\n[[object.signal.field]]\nname = \"force\"\n\n# The blur.\n[[object.variant]]\nname = \"blur\"\n\n# The radius.\n[[object.variant.property]]\nname = \"radius\"\n",
        );
        assert_eq!(docs.enum_doc("mode"), "The mode.");
        assert_eq!(docs.enum_value("mode", "quiet"), "Quiet.");
        assert_eq!(docs.enum_value("mode", "loud"), "");
        assert_eq!(docs.enum_value("mode", "both"), "Both words. On two lines.");
        assert_eq!(docs.callback("hook"), "A hook.");
        assert_eq!(docs.callback_param("hook", "event"), "The event.");
        assert_eq!(docs.classes["fam"].children["fired.force"], "How hard.");
        assert_eq!(docs.variant("fam", "blur"), "The blur.");
        assert_eq!(docs.variant_property("fam", "blur", "radius"), "The radius.");
    }

    #[test]
    fn the_lines_of_a_multi_line_string_are_not_comments_or_headers() {
        let (_, docs) = parse_docs(
            "namespace = \"x.y\"\n\n# One.\n[[object]]\nname = \"a\"\nexample = { python = '''\n# a comment of the program\n[[object]]\nname = \"b\"\n''' }\n\n# Two.\n[[object.property]]\nname = \"p\"\n",
        );
        assert_eq!(docs.classes["a"].text, "One.");
        assert!(!docs.classes.contains_key("b"));
        assert_eq!(docs.classes["a"].members["p"], "Two.");
    }
}

//! The descriptions of the classes: the comments the schema files carry above each class, property, method, signal and enum.
//!
//! The catalog (`gnoblin-schema`) does not keep comments, so the generator reads them from the files again. A comment is the lines of `#`
//! directly above a table header; the table's name is the first `name =` line under the header. A blank line between the comment and the
//! header means the comment describes nothing. Parameters, fields, variants and enum values are not documented.
//!
//! [`Docs`] reaches every target that writes declarations (`object-types` for TypeScript and Python, `object-api` for Rust), so the
//! description of a class is the same sentence in the editor's hover text, in the generated reference of each language and in the Rust
//! docs. Each target decides how its language writes a comment (`/** */`, a docstring, `///`); this module only holds the text.

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
    enums: BTreeMap<String, String>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ClassDocs {
    text: String,
    /// Properties, methods and signals by their schema name.
    members: BTreeMap<String, String>,
}

impl NamespaceDocs {
    /// The description of the class `name`, or an empty text.
    pub fn class(&self, name: &str) -> &str {
        self.classes.get(name).map_or("", |class| class.text.as_str())
    }

    /// The description of the enum `name`, or an empty text.
    pub fn enum_doc(&self, name: &str) -> &str {
        self.enums.get(name).map_or("", String::as_str)
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

    fn merge(&mut self, other: NamespaceDocs) {
        for (name, class) in other.classes {
            let merged = self.classes.entry(name).or_default();
            if !class.text.is_empty() {
                merged.text = class.text;
            }
            merged.members.extend(class.members);
        }
        self.enums.extend(other.enums);
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
    Member(String),
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

/// Reads the comments of one schema file: its namespace and what it says about the classes.
pub(crate) fn parse_docs(text: &str) -> (Option<String>, NamespaceDocs) {
    let mut docs = NamespaceDocs::default();
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
                "object.property" | "object.method" | "object.signal" | "augment.property" | "augment.method"
                    if class.is_some() =>
                {
                    WaitingName::Member(doc)
                }
                "augment" => {
                    class = None;
                    WaitingName::Target
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
        if let Some(value) = string_value(line, "namespace") {
            namespace = Some(value.to_owned());
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
            "namespace = \"gnoblin.std\"\n\n# Not described.\n[[augment]]\ntarget = \"core.client\"\n\n# Starts it.\n[[augment.method]]\nname = \"move_with_keys\"\n\n# Reads it.\n[[augment.property]]\nname = \"style\"\n",
        );
        let class = &docs.classes["client_augment"];
        assert_eq!(class.members["move_with_keys"], "Starts it.");
        assert_eq!(class.members["style"], "Reads it.");
        assert_eq!(class.text, "");
    }

    #[test]
    fn a_member_is_described_by_the_nearest_base_that_does() {
        let (_, docs) = parse_docs(
            "namespace = \"x.y\"\n\n# Base.\n[[object]]\nname = \"base\"\n\n# The id.\n[[object.property]]\nname = \"id\"\n\n[[object]]\nname = \"derived\"\nbase = \"base\"\n",
        );
        let bases = |class: &str| (class == "derived").then_some("base");
        assert_eq!(docs.member("derived", "id", bases), "The id.");
        assert_eq!(docs.member("derived", "other", bases), "");
    }
}

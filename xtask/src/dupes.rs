//! `cargo xtask dupes`: type declarations that two crates define twice.
//!
//! The rewrite adds crates in parallel, and a lane that cannot see a sibling's
//! crate declares its own `Rect`, `Size` or `Event` rather than waiting for the
//! shared one. That is how a workspace ends up converting between four spellings
//! of the same value at every seam. This command parses every workspace crate's
//! sources and reports the shapes that already exist elsewhere:
//!
//! - **same name**: the same struct or enum name defined in two crates, which is
//!   almost always two types that should be one;
//! - **same shape**: different names with identical members (the same field names
//!   and types for a named-field struct, the same variant names for an enum),
//!   which is the same type under two spellings. Tuple structs are names around
//!   a value rather than shapes, so they are compared by name only;
//! - **near shape**: structs with at least three fields whose field sets overlap
//!   with a Jaccard similarity of 0.8 or more, which catches a copy that gained
//!   one field.
//!
//! # Allow-list
//!
//! Not every match is a defect: `gnoblin-model::Answer` and the provider's
//! answer are deliberately different types, and two object types can honestly
//! share a field list. Those pairs go in `xtask/dupes-allow.toml`, and every
//! entry carries a `reason` because an unexplained exemption is a missing
//! review:
//!
//! ```toml
//! [[allow]]
//! left = "gnoblin-model::Answer"
//! right = "gnoblin-model::Value"
//! kind = "same-shape"          # optional: same-name, same-shape or near-shape
//! reason = "an answer is a provider's encoded reply; a value is a typed graph value"
//! ```
//!
//! # Running the gate
//!
//! `cargo xtask dupes` fails on any hit the allow-list does not waive: the shared
//! types have moved into `gnoblin-types` and every remaining pair is either that or
//! a reviewed homonym, so a new duplicate is a regression rather than the state of
//! the migration. The check runs locally, as every check in this rewrite does:
//!
//! ```text
//! cargo xtask dupes --deny     # the gate, exits non-zero on an unwaived hit
//! cargo xtask dupes --warn     # the report, exits zero, for a lane mid-migration
//! ```
//!
//! It is deliberately *not* a step in `just ci`: the rewrite's validation is local
//! and `just ci` must stay the commands a contributor can run without a network. Run
//! `cargo xtask dupes --deny` before merging anything that adds a crate or moves a
//! type.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use quote::ToTokens;
use serde::Deserialize;
use syn::spanned::Spanned;
use syn::{Attribute, Fields, Item, ItemEnum, ItemStruct, Meta};

/// The allow-list, relative to the workspace root.
const ALLOW_FILE: &str = "xtask/dupes-allow.toml";

/// The Jaccard similarity at which two field sets count as a near match.
const NEAR_SHAPE_THRESHOLD: f64 = 0.8;

/// The fewest fields a near-shape comparison needs; two-field structs are too
/// common (`width`/`height`, `x`/`y`) to say anything.
const NEAR_SHAPE_MIN_FIELDS: usize = 3;

/// What kind of declaration a definition is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A struct.
    Struct,
    /// An enum.
    Enum,
}

/// One declared struct or enum in a workspace crate.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeDef {
    /// The crate the declaration lives in.
    pub krate: String,
    /// The declared name.
    pub name: String,
    /// Struct or enum.
    pub kind: Kind,
    /// A struct's named fields as `name: type` keys, in declaration order.
    ///
    /// Tuple and unit structs carry none, because a newtype is a name rather
    /// than a shape. An enum's payload fields are not part of its shape either:
    /// the variant names are what an enum's shape is.
    pub fields: Vec<String>,
    /// An enum's variant names.
    pub variants: BTreeSet<String>,
    /// The source file, relative to the scan root.
    pub file: String,
    /// The line the declaration starts on.
    pub line: usize,
}

impl TypeDef {
    /// The `crate::Name` spelling the report prints.
    #[must_use]
    pub fn qualified(&self) -> String {
        format!("{}::{}", self.krate, self.name)
    }

    /// The set a shape comparison uses.
    fn shape(&self) -> BTreeSet<&str> {
        match self.kind {
            Kind::Struct => self.fields.iter().map(String::as_str).collect(),
            Kind::Enum => self.variants.iter().map(String::as_str).collect(),
        }
    }
}

/// How two definitions relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HitKind {
    /// The same name in two crates.
    SameName,
    /// Different names, identical members.
    SameShape,
    /// Different names, field sets overlapping above the threshold.
    NearShape,
}

impl HitKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::SameName => "same-name",
            Self::SameShape => "same-shape",
            Self::NearShape => "near-shape",
        }
    }

    fn order(self) -> u8 {
        match self {
            Self::SameName => 0,
            Self::SameShape => 1,
            Self::NearShape => 2,
        }
    }
}

/// One reported pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Which check produced the pair.
    pub kind: HitKind,
    /// The similarity for a near match, one for an exact match.
    pub similarity: f64,
    /// The definition that sorts first.
    pub left: TypeDef,
    /// Its partner.
    pub right: TypeDef,
}

impl Hit {
    /// The two `crate::Name` spellings, in either order.
    fn pair(&self) -> (String, String) {
        (self.left.qualified(), self.right.qualified())
    }
}

impl fmt::Display for Hit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let similarity = if self.kind == HitKind::NearShape {
            format!(" {:.2}", self.similarity)
        } else {
            String::new()
        };
        write!(
            formatter,
            "{}{} {} <-> {}  ({}:{}, {}:{})",
            self.kind.as_str(),
            similarity,
            self.left.qualified(),
            self.right.qualified(),
            self.left.file,
            self.left.line,
            self.right.file,
            self.right.line,
        )
    }
}

/// One allow-list entry.
#[derive(Clone, Debug, Deserialize)]
pub struct AllowEntry {
    /// One side of the pair, as `crate::Name`.
    pub left: String,
    /// The other side, as `crate::Name`.
    pub right: String,
    /// The check this entry waives, when it should waive only one.
    #[serde(default)]
    pub kind: Option<String>,
    /// Why the two declarations are deliberately separate.
    pub reason: String,
}

impl AllowEntry {
    fn permits(&self, hit: &Hit) -> bool {
        let (left, right) = hit.pair();
        let sides_match = (self.left == left && self.right == right) || (self.left == right && self.right == left);
        let kind_matches = self.kind.as_deref().is_none_or(|kind| kind == hit.kind.as_str());
        sides_match && kind_matches
    }
}

/// The file the allow-list is read from.
#[derive(Clone, Debug, Default, Deserialize)]
struct AllowList {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

/// One crate to scan.
struct CrateRoot {
    name: String,
    src: PathBuf,
}

/// Read every workspace member's sources and report the duplicate shapes.
pub fn run(deny: bool) -> ExitCode {
    let root = Path::new(".");
    match report(root, deny) {
        Ok(clean) => {
            if deny && !clean {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(message) => {
            eprintln!("[dupes] {message}");
            ExitCode::FAILURE
        }
    }
}

/// Print the report and return whether every hit was allowed.
fn report(root: &Path, deny: bool) -> Result<bool, String> {
    let crates = workspace_crates(root)?;
    let definitions = scan(&crates, root)?;
    let hits = hits(&definitions);
    let allow = load_allow_list(root)?;
    let mut unallowed = 0;

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for hit in &hits {
        let allowed = allow.iter().any(|entry| entry.permits(hit));
        if !allowed {
            unallowed += 1;
        }
        *counts.entry(hit.kind.as_str()).or_default() += 1;
        println!("[dupes] {}: {hit}", if allowed { "allowed" } else { "hit" });
    }

    let summary = if counts.is_empty() {
        "no duplicates".to_owned()
    } else {
        counts
            .iter()
            .map(|(kind, count)| format!("{count} {kind}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "[dupes] {} declaration(s) in {} crate(s): {summary}{}",
        definitions.len(),
        crates.len(),
        if unallowed == 0 {
            String::new()
        } else {
            format!(", {unallowed} not in {ALLOW_FILE}")
        }
    );
    if !definitions.is_empty() && unallowed > 0 && !deny {
        println!("[dupes] warning mode: {unallowed} hit(s) reported, exiting zero; pass --deny to fail");
    }
    for entry in &allow {
        let used = hits.iter().any(|hit| entry.permits(hit));
        if !used {
            println!(
                "[dupes] warning: allow entry {} <-> {} matches nothing",
                entry.left, entry.right
            );
        }
    }
    Ok(unallowed == 0)
}

/// The workspace members, as crate roots to scan.
fn workspace_crates(root: &Path) -> Result<Vec<CrateRoot>, String> {
    let manifest = read_toml(&root.join("Cargo.toml"))?;
    let members = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "the workspace manifest declares no members".to_owned())?;
    let mut crates = Vec::new();
    for member in members {
        let relative = member
            .as_str()
            .ok_or_else(|| "a workspace member is not a string".to_owned())?;
        let directory = root.join(relative);
        let package = read_toml(&directory.join("Cargo.toml"))
            .map_err(|error| format!("workspace member {relative}: {error}"))?;
        let name = package
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("workspace member {relative} declares no package name"))?;
        crates.push(CrateRoot {
            name: name.to_owned(),
            src: directory.join("src"),
        });
    }
    crates.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(crates)
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    toml::from_str(&text).map_err(|error| format!("invalid TOML in {}: {error}", path.display()))
}

/// Parse every `.rs` file under each crate's `src`.
fn scan(crates: &[CrateRoot], display_root: &Path) -> Result<Vec<TypeDef>, String> {
    let mut definitions = Vec::new();
    for krate in crates {
        let mut files = Vec::new();
        collect_files(&krate.src, &mut files)?;
        files.sort();
        for file in files {
            let text =
                fs::read_to_string(&file).map_err(|error| format!("could not read {}: {error}", file.display()))?;
            let parsed =
                syn::parse_file(&text).map_err(|error| format!("could not parse {}: {error}", file.display()))?;
            let display = display_path(&file, display_root);
            visit_items(&krate.name, &display, &parsed.items, &mut definitions);
        }
    }
    definitions.sort();
    Ok(definitions)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        // A crate with no `src` directory has nothing to declare.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("could not read {}: {error}", directory.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {}: {error}", directory.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

/// Walk the items of a file, skipping test modules.
fn visit_items(krate: &str, file: &str, items: &[Item], out: &mut Vec<TypeDef>) {
    for item in items {
        match item {
            Item::Struct(item) => out.push(struct_def(krate, file, item)),
            Item::Enum(item) => out.push(enum_def(krate, file, item)),
            Item::Mod(module) => {
                if is_test_only(&module.attrs) {
                    continue;
                }
                if let Some((_, items)) = &module.content {
                    visit_items(krate, file, items, out);
                }
            }
            _ => {}
        }
    }
}

/// Whether an attribute list marks a module as test-only.
///
/// Test modules declare helper structs with the same names as the types they
/// exercise, so including them would flood the report with fixtures.
fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && matches!(&attr.meta, Meta::List(list) if list.tokens.to_string().contains("test"))
    })
}

fn struct_def(krate: &str, file: &str, item: &ItemStruct) -> TypeDef {
    let fields = match &item.fields {
        Fields::Named(named) => named
            .named
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().map_or_else(String::new, ToString::to_string);
                format!("{name}: {}", normalized_type(field.ty.to_token_stream()))
            })
            .collect(),
        // A tuple struct is a name around a value, and a family of newtypes over
        // the same payload is deliberate: `Tick` and `Revision` are both a `u64`
        // and must stay distinct. They therefore contribute no comparable shape;
        // the same-name check still covers them.
        Fields::Unnamed(_) | Fields::Unit => Vec::new(),
    };
    TypeDef {
        krate: krate.to_owned(),
        name: item.ident.to_string(),
        kind: Kind::Struct,
        fields,
        variants: BTreeSet::new(),
        file: file.to_owned(),
        line: item.span().start().line,
    }
}

fn enum_def(krate: &str, file: &str, item: &ItemEnum) -> TypeDef {
    TypeDef {
        krate: krate.to_owned(),
        name: item.ident.to_string(),
        kind: Kind::Enum,
        fields: Vec::new(),
        variants: item.variants.iter().map(|variant| variant.ident.to_string()).collect(),
        file: file.to_owned(),
        line: item.span().start().line,
    }
}

/// A type's spelling with all token whitespace removed, so `Vec< u8 >` and
/// `Vec<u8>` compare equal.
fn normalized_type(tokens: proc_macro2::TokenStream) -> String {
    tokens.to_string().split_whitespace().collect::<String>()
}

/// Every pair of definitions the checks report, in a stable order.
pub fn hits(definitions: &[TypeDef]) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();

    // (a) The same name in two crates.
    for (index, left) in definitions.iter().enumerate() {
        for right in &definitions[index + 1..] {
            if left.krate != right.krate && left.name == right.name {
                hits.push(Hit {
                    kind: HitKind::SameName,
                    similarity: 1.0,
                    left: left.clone(),
                    right: right.clone(),
                });
            }
        }
    }

    // (b) Different names with identical members.
    for (index, left) in definitions.iter().enumerate() {
        for right in &definitions[index + 1..] {
            if left.krate == right.krate || left.name == right.name || left.kind != right.kind {
                continue;
            }
            let (left_shape, right_shape) = (left.shape(), right.shape());
            let same = !left_shape.is_empty() && left_shape == right_shape;
            if same {
                hits.push(Hit {
                    kind: HitKind::SameShape,
                    similarity: 1.0,
                    left: left.clone(),
                    right: right.clone(),
                });
            }
        }
    }

    // (c) Structs whose field sets nearly match.
    for (index, left) in definitions.iter().enumerate() {
        if left.kind != Kind::Struct || left.fields.len() < NEAR_SHAPE_MIN_FIELDS {
            continue;
        }
        for right in &definitions[index + 1..] {
            if right.kind != Kind::Struct
                || right.fields.len() < NEAR_SHAPE_MIN_FIELDS
                || left.krate == right.krate
                || left.name == right.name
            {
                continue;
            }
            let (left_shape, right_shape) = (left.shape(), right.shape());
            if left_shape == right_shape {
                // Already reported as a same-shape hit.
                continue;
            }
            let similarity = jaccard(&left_shape, &right_shape);
            if similarity >= NEAR_SHAPE_THRESHOLD {
                hits.push(Hit {
                    kind: HitKind::NearShape,
                    similarity,
                    left: left.clone(),
                    right: right.clone(),
                });
            }
        }
    }

    hits.sort_by(|left, right| {
        left.kind
            .order()
            .cmp(&right.kind.order())
            .then_with(|| left.pair().cmp(&right.pair()))
    });
    hits
}

/// The Jaccard similarity of two sets: the shared size over the total size.
fn jaccard(left: &BTreeSet<&str>, right: &BTreeSet<&str>) -> f64 {
    let shared = left.intersection(right).count();
    let total = left.union(right).count();
    if total == 0 { 0.0 } else { shared as f64 / total as f64 }
}

/// Read the allow-list beside this crate.
fn load_allow_list(root: &Path) -> Result<Vec<AllowEntry>, String> {
    let path = root.join(ALLOW_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        // No allow-list is the normal state before an exemption is needed.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let list: AllowList =
        toml::from_str(&text).map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;
    for entry in &list.allow {
        if entry.reason.trim().is_empty() {
            return Err(format!(
                "{}: the entry {} <-> {} has an empty reason; every exemption states why the declarations are separate",
                path.display(),
                entry.left,
                entry.right
            ));
        }
        if let Some(kind) = &entry.kind
            && !["same-name", "same-shape", "near-shape"].contains(&kind.as_str())
        {
            return Err(format!(
                "{}: the entry {} <-> {} names `{kind}`, which is not a hit kind",
                path.display(),
                entry.left,
                entry.right
            ));
        }
    }
    Ok(list.allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree where every check has a hit, plus crates that share nothing.
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
    }

    fn scan_fixture(name: &str) -> (Vec<CrateRoot>, Vec<TypeDef>) {
        let root = fixture(name);
        let mut crates = Vec::new();
        for entry in fs::read_dir(&root).expect("fixture root") {
            let entry = entry.expect("fixture entry");
            if !entry.path().is_dir() {
                continue;
            }
            crates.push(CrateRoot {
                name: entry.file_name().to_string_lossy().into_owned(),
                src: entry.path().join("src"),
            });
        }
        crates.sort_by(|left, right| left.name.cmp(&right.name));
        let definitions = scan(&crates, &root).expect("fixture sources parse");
        (crates, definitions)
    }

    fn pairs(hits: &[Hit], kind: HitKind) -> Vec<(String, String)> {
        hits.iter().filter(|hit| hit.kind == kind).map(Hit::pair).collect()
    }

    #[test]
    fn the_same_name_in_two_crates_is_reported() {
        let (_, definitions) = scan_fixture("dupes");
        let hits = hits(&definitions);
        let same_name = pairs(&hits, HitKind::SameName);
        assert!(
            same_name.contains(&("alpha::Widget".to_owned(), "beta::Widget".to_owned())),
            "{same_name:#?}"
        );
        let hit = hits
            .iter()
            .find(|hit| hit.kind == HitKind::SameName && hit.left.name == "Widget")
            .expect("the Widget pair");
        assert!(hit.to_string().contains("alpha/src/lib.rs:"));
        assert!(hit.to_string().contains("beta/src/lib.rs:"));
    }

    #[test]
    fn different_names_with_the_same_members_are_reported() {
        let (_, definitions) = scan_fixture("dupes");
        let hits = hits(&definitions);
        let same_shape = pairs(&hits, HitKind::SameShape);
        assert!(
            same_shape.contains(&("alpha::Widget".to_owned(), "beta::Dimensions".to_owned())),
            "{same_shape:#?}"
        );
        assert!(
            same_shape.contains(&("alpha::Mode".to_owned(), "beta::Kind".to_owned())),
            "enums compare by variant names: {same_shape:#?}"
        );
    }

    #[test]
    fn a_near_match_is_reported_with_its_similarity() {
        let (_, definitions) = scan_fixture("dupes");
        let hits = hits(&definitions);
        let near = hits
            .iter()
            .find(|hit| {
                hit.kind == HitKind::NearShape && hit.pair() == ("alpha::Layout".to_owned(), "gamma::Frame".to_owned())
            })
            .expect("the near-shape pair");
        assert!((near.similarity - 0.8).abs() < f64::EPSILON, "{}", near.similarity);
        assert!(near.to_string().starts_with("near-shape 0.80 "), "{near}");

        // Two-field structs never produce a near match, however similar.
        assert!(
            !hits
                .iter()
                .any(|hit| hit.kind == HitKind::NearShape && hit.left.fields.len() < NEAR_SHAPE_MIN_FIELDS),
            "a pair with fewer than {NEAR_SHAPE_MIN_FIELDS} fields was compared"
        );
    }

    #[test]
    fn crates_that_share_nothing_produce_no_hits() {
        let (_, definitions) = scan_fixture("dupes-clean");
        assert!(!definitions.is_empty(), "the clean fixture declares types");
        assert_eq!(hits(&definitions), Vec::new());
    }

    #[test]
    fn test_modules_are_not_scanned() {
        let (_, definitions) = scan_fixture("dupes");
        assert!(
            !definitions.iter().any(|definition| definition.name == "TestOnly"),
            "a cfg(test) module is a fixture, not a duplicate"
        );
    }

    #[test]
    fn an_allow_entry_waives_one_pair_with_its_reason() {
        let (_, definitions) = scan_fixture("dupes");
        let hits = hits(&definitions);
        let widget = hits
            .iter()
            .find(|hit| hit.pair() == ("alpha::Widget".to_owned(), "beta::Widget".to_owned()))
            .expect("the Widget pair");
        let entry = AllowEntry {
            left: "beta::Widget".to_owned(),
            right: "alpha::Widget".to_owned(),
            kind: Some("same-name".to_owned()),
            reason: "fixture".to_owned(),
        };
        assert!(entry.permits(widget), "an entry matches in either order");
        let other_kind = AllowEntry {
            kind: Some("near-shape".to_owned()),
            ..entry.clone()
        };
        assert!(!other_kind.permits(widget), "the kind scopes the entry");
    }

    #[test]
    fn the_repository_allow_list_parses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let allow = load_allow_list(&root).expect("the checked-in allow-list is valid");
        assert!(allow.iter().all(|entry| !entry.reason.trim().is_empty()));
    }

    #[test]
    fn the_workspace_scan_finds_its_own_crates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let crates = workspace_crates(&root).expect("workspace members");
        assert!(crates.iter().any(|krate| krate.name == "gnoblin-types"));
        assert!(crates.iter().any(|krate| krate.name == "gnoblin-wm"));
    }
}

//! `gnoblin-sdkgen`: every language SDK, generated from one description.
//!
//! The sources of truth are the schema catalog and the two ABI crates; [`SdkSpec`] is what they add up to, and a [`Target`]
//! turns a spec into files. Nothing the generator writes is committed. Three call sites render it:
//!
//! | call site                                  | when                | what it does                                                            |
//! | ------------------------------------------ | ------------------- | ----------------------------------------------------------------------- |
//! | `build.rs` of an SDK or runtime crate      | every build         | [`emit_for_build`] writes that crate's targets into `OUT_DIR`             |
//! | `cargo xtask dist install`                 | install             | writes the editor files into the prefix                                 |
//! | `cargo xtask gen-sdks [--out DIR] [--check]` | by hand, in CI    | writes a readable copy (`target/sdk`), or checks the generator's output |
//!
//! The generator reads no clock, no environment variable and no locale, and orders everything by name, so two renders are
//! byte-identical. Every generated file starts with a banner that names the generator, the target and the catalog hash, so a
//! reader who opens one knows not to edit it and where it comes from.
//!
//! Adding a language is one [`Target`] implementation and one line in [`targets`]; `docs/sdk-generator.md` is the contract.
//!
//! The descriptions of the classes (the comments above each class, property, method, parameter, signal, signal field, enum and enum value
//! in the schema files) are read once into [`SdkSpec::docs`], and the declaration targets (`object-types`, `object-api`) write them, with
//! the range, unit, default, error codes and example of a member, as the doc comments of their language (`/** */`, a docstring, `///`).
//! The generated reference of the website is the tool of each language run over those declarations, so a description is the same sentence
//! in an editor, in TypeDoc, in Sphinx and in rustdoc. The `reference` target writes the same facts, and the declarations the other
//! targets wrote, as one JSON file.
//!
//! The targets, and the logical paths they write (relative to the output directory):
//!
//! | target       | writes                                                                                        |
//! | ------------ | --------------------------------------------------------------------------------------------- |
//! | `rust`       | `rust-core/generated.rs`: schema names, typed node classes, the error kinds, as one file       |
//! | `std-dyn`    | `rust-std-dyn/generated.rs`: the dynamic call of every std table function                      |
//! | `python`     | `python/gnoblin/`: typed node classes and `.pyi`, ABI constants, verbs, the std bindings       |
//! | `typescript` | `js/gnoblin-core/generated/` and `js/gnoblin-std/generated/`: types, schema table, ABI, verbs  |
//! | `reference`  | `reference/reference.json`: every class, member, enum and callback of the libraries, with its docs |

pub mod abi_c;
#[cfg(test)]
mod augment_tests;
mod coverage;
mod declarations;
mod describe;
mod docs;
mod layout_schema;
mod layouts;
mod model;
mod object_api;
mod object_js;
mod object_py;
mod object_script;
mod objects;
mod python;
mod reference;
mod rust;
mod std_dyn;
mod surface;
mod text;
mod typescript;
mod typescript_std;
mod values;
mod verbs;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub use coverage::{Coverage, KindCoverage, coverage};
pub use docs::{DocLanguage, Docs, NamespaceDocs};
pub use model::SdkSpec;
pub use surface::{LockReport, check_surface_lock, render_surface_locks, surface_lines};

/// The command that writes a readable copy of everything, named in every banner and error.
pub const COMMAND: &str = "cargo xtask gen-sdks";

/// The comment syntax of a generated file's banner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comment {
    /// `// text`: Rust, TypeScript, C.
    Slashes,
    /// `# text`: Python, TOML, shell.
    Hash,
}

/// One generated file.
pub struct File {
    /// The logical path, relative to the output directory (`python/gnoblin/std.py`).
    pub path: String,
    /// The comment syntax of the banner this file gets; `None` for a file that must not start with one.
    pub comment: Option<Comment>,
    /// The content, without the banner.
    pub text: String,
}

/// A language an SDK is generated for.
pub trait Target {
    /// The name `--target` takes and the banner prints.
    fn name(&self) -> &'static str;

    /// Renders the target's files.
    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String>;
}

/// Every target, in the order they run.
pub fn targets() -> Vec<Box<dyn Target>> {
    vec![
        Box::new(rust::RustTarget),
        Box::new(std_dyn::StdDynTarget),
        Box::new(python::PythonTarget),
        Box::new(typescript::TypeScriptTarget),
        Box::new(objects::ObjectsTarget),
        Box::new(object_api::ObjectApiTarget),
        Box::new(object_script::ObjectJsTarget),
        Box::new(object_script::ObjectPyTarget),
        Box::new(object_script::ObjectTypesTarget),
        Box::new(reference::ReferenceTarget),
    ]
}

/// The banner of a generated file.
fn banner(spec: &SdkSpec, target: &str, comment: Comment) -> String {
    let lines = [
        format!(
            "GENERATED by gnoblin-sdkgen (target: {target}) at build time. Nothing here is committed and nothing here is edited:"
        ),
        format!("change the schema or the ABI crates; `{COMMAND} --out DIR` writes a readable copy."),
        format!(
            "catalog 0x{:016x}, {}",
            spec.catalog_hash,
            spec.libraries
                .iter()
                .map(|library| format!("{}-abi {}.{}", library.name, library.abi.0, library.abi.1))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    let prefix = match comment {
        Comment::Slashes => "//",
        Comment::Hash => "#",
    };
    let mut text = String::new();
    for line in lines {
        text.push_str(prefix);
        text.push(' ');
        text.push_str(&line);
        text.push('\n');
    }
    text
}

/// Formats Rust source the way `cargo fmt` would, with the toolchain the repository pins and its `rustfmt.toml`, so a readable
/// copy of a generated Rust file looks like source. Builds do not call it: the compiler does not care.
///
/// # Errors
///
/// `rustfmt` is not installed or refused the source.
pub fn format_rust(source: &str, config: &Path) -> Result<String, String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--config-path"])
        .arg(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run rustfmt (it is a component of the pinned toolchain): {error}"))?;
    let mut stdin = child.stdin.take().ok_or("rustfmt has no stdin")?;
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| format!("rustfmt did not finish: {error}"))?;
    writer
        .join()
        .map_err(|_| "the rustfmt writer panicked".to_owned())?
        .map_err(|error| format!("cannot feed rustfmt: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustfmt refused the generated source: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("rustfmt wrote invalid UTF-8: {error}"))
}

/// The files one target rendered: `(path, full text)` pairs, banners included, in path order.
pub struct TargetFiles {
    /// The target's name.
    pub target: &'static str,
    /// The files, sorted by path.
    pub files: Vec<(String, String)>,
}

/// Renders every selected target to `(path, full text)` pairs, banners included. The text is not formatted.
///
/// # Errors
///
/// A target cannot render, renders a path twice, or `only` names no target.
pub fn render(spec: &SdkSpec, only: Option<&str>) -> Result<Vec<TargetFiles>, String> {
    let mut rendered = Vec::new();
    for target in targets() {
        if only.is_some_and(|only| only != target.name()) {
            continue;
        }
        let mut files = Vec::new();
        for file in target.generate(spec)? {
            let text = match file.comment {
                Some(comment) => format!("{}\n{}", banner(spec, target.name(), comment), file.text),
                None => file.text,
            };
            files.push((file.path, text));
        }
        let mut seen = BTreeSet::new();
        for (path, _) in &files {
            if !seen.insert(path.clone()) {
                return Err(format!("target {} generates {path} twice", target.name()));
            }
        }
        files.sort();
        rendered.push(TargetFiles {
            target: target.name(),
            files,
        });
    }
    if rendered.is_empty() {
        return Err(format!("no target named {}", only.unwrap_or("")));
    }
    Ok(rendered)
}

/// Runs `rustfmt` over every Rust file of `rendered`.
///
/// # Errors
///
/// `rustfmt` fails on a file.
pub fn format_all(rendered: &mut [TargetFiles], config: &Path) -> Result<(), String> {
    for target in rendered {
        for (path, text) in &mut target.files {
            if path.ends_with(".rs") {
                *text = format_rust(text, config).map_err(|error| format!("{path}: {error}"))?;
            }
        }
    }
    Ok(())
}

/// Every file of `rendered`, in target order then path order.
pub fn files_of(rendered: &[TargetFiles]) -> Vec<(String, String)> {
    rendered
        .iter()
        .flat_map(|target| target.files.iter().cloned())
        .collect()
}

/// Writes `files` under `dir`, creating directories, and leaves a file whose text is already there alone (so a build does not
/// touch what did not change). Returns the paths written.
///
/// # Errors
///
/// A directory or a file cannot be written.
pub fn write_files(dir: &Path, files: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (path, text) in files {
        let full: PathBuf = dir.join(path);
        if fs::read_to_string(&full).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::write(&full, text).map_err(|error| format!("cannot write {}: {error}", full.display()))?;
        written.push(path.clone());
    }
    Ok(written)
}

/// What a `build.rs` calls: renders the targets named in `wanted` (names of [`targets`]) from the schema files of the crates this
/// generator was compiled with ([`SdkSpec::load_packaged`]) and writes them under `OUT_DIR`, then tells cargo to rerun when those
/// files change. It works the same in the workspace and in a crate from crates.io. Returns the logical path of every file the targets
/// render, sorted, so a script that embeds them lists exactly these and not what an earlier build left in `OUT_DIR`.
///
/// The files keep their logical paths, so a build script that embeds one reads `OUT_DIR/js/gnoblin-core/generated/abi.ts`. A
/// failure ends the script with a message, which is what a build script's failure looks like.
pub fn emit_for_build(wanted: &[&str]) -> Vec<String> {
    match emit(wanted) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("[sdkgen] {error}");
            std::process::exit(1);
        }
    }
}

/// What the `build.rs` of a crate that serves a catalog of its own calls (a test fixture, not a library of the desktop): renders the
/// static catalog (`rust-objects/<stem>_catalog.rs`) and the typed API (`rust-objects/<stem>_api.rs`) of the namespace in the schema
/// files under `dirs`, relative to the crate, into `OUT_DIR`, and its classes for the script languages (`script-objects/<stem>.{js,d.ts,py,pyi}`). The namespace is the one `dirs` declare.
pub fn emit_catalog_for_build(dirs: &[&str], stem: &str) {
    if let Err(error) = emit_catalog(dirs, stem) {
        eprintln!("[sdkgen] {error}");
        std::process::exit(1);
    }
}

fn emit_catalog(dirs: &[&str], stem: &str) -> Result<(), String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| "CARGO_MANIFEST_DIR is not set".to_owned())?;
    let out = std::env::var("OUT_DIR").map_err(|_| "OUT_DIR is not set".to_owned())?;
    let paths: Vec<PathBuf> = dirs.iter().map(|dir| Path::new(&manifest).join(dir)).collect();
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let catalog = gnoblin_schema::Catalog::load_dirs(&refs, &[])
        .map_err(|error| format!("the catalog does not load: {error}"))?;
    let namespaces = catalog.namespaces();
    let [namespace] = namespaces.as_slice() else {
        return Err(format!(
            "expected one namespace under {dirs:?}, found {}",
            namespaces.len()
        ));
    };
    let files = vec![
        (
            format!("rust-objects/{stem}_catalog.rs"),
            objects::render(&catalog, namespace)?,
        ),
        (
            format!("rust-objects/{stem}_api.rs"),
            object_api::render(&catalog, namespace)?,
        ),
    ];
    // The classes of the same catalog for the script languages, so the fixture's tests can run a script against the library it serves.
    let view = object_script::ScriptView::new(&catalog, namespace);
    let mut files = files;
    files.extend([
        (format!("script-objects/{stem}.js"), object_js::render_js(&view)?),
        (format!("script-objects/{stem}.d.ts"), object_js::render_dts(&view)?),
        (format!("script-objects/{stem}.py"), object_py::render(&view, false)?),
        (format!("script-objects/{stem}.pyi"), object_py::render(&view, true)?),
    ]);
    write_files(Path::new(&out), &files)?;
    Ok(())
}

fn emit(wanted: &[&str]) -> Result<Vec<String>, String> {
    let out = std::env::var("OUT_DIR").map_err(|_| "OUT_DIR is not set".to_owned())?;
    // The schema files of the crates this generator was compiled with: the workspace's in the workspace, the packaged ones in a
    // crate from crates.io. Never a path relative to the crate being built, which a packaged crate does not have.
    for dir in SdkSpec::packaged_dirs() {
        println!("cargo:rerun-if-changed={dir}");
    }
    let spec = SdkSpec::load_packaged()?;
    let mut paths = Vec::new();
    for name in wanted {
        let files = files_of(&render(&spec, Some(name))?);
        write_files(Path::new(&out), &files)?;
        paths.extend(files.into_iter().map(|(path, _)| path));
    }
    paths.sort();
    Ok(paths)
}

/// The workspace root, for the tests of every module.
#[cfg(test)]
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        workspace_root()
    }

    /// Two renders are the same bytes: nothing the generator reads changes between calls.
    #[test]
    fn two_renders_are_byte_identical() {
        let spec = SdkSpec::load(&root()).expect("the specification loads");
        let first = files_of(&render(&spec, None).expect("every target renders"));
        let second = files_of(&render(&spec, None).expect("every target renders again"));
        assert_eq!(first, second);
    }

    #[test]
    fn every_generated_file_carries_the_banner_and_the_catalog_hash() {
        let spec = SdkSpec::load(&root()).expect("the specification loads");
        let hash = format!("0x{:016x}", spec.catalog_hash);
        for target in render(&spec, None).expect("every target renders") {
            assert!(!target.files.is_empty(), "{} renders nothing", target.target);
            for (path, text) in target.files {
                let head: String = text.lines().take(6).collect::<Vec<_>>().join("\n");
                assert!(head.contains("GENERATED by gnoblin-sdkgen"), "{path} has no banner");
                assert!(head.contains(&hash), "{path} does not name the catalog hash");
            }
        }
    }

    #[test]
    fn targets_have_distinct_names_and_no_two_write_one_path() {
        let all = targets();
        let mut names = BTreeSet::new();
        for target in &all {
            assert!(names.insert(target.name()), "{} twice", target.name());
        }
        let spec = SdkSpec::load(&root()).expect("the specification loads");
        let mut paths = BTreeSet::new();
        for (path, _) in files_of(&render(&spec, None).expect("every target renders")) {
            assert!(paths.insert(path.clone()), "{path} is written by two targets");
        }
    }

    /// Nothing the generator writes starts in a directory a person edits: every logical path begins with one of the output roots.
    #[test]
    fn paths_are_logical_not_repository_paths() {
        let spec = SdkSpec::load(&root()).expect("the specification loads");
        for (path, _) in files_of(&render(&spec, None).expect("every target renders")) {
            assert!(
                [
                    "rust-core/",
                    "rust-std-dyn/",
                    "rust-objects/",
                    "python/",
                    "js/",
                    "reference/"
                ]
                .iter()
                .any(|prefix| path.starts_with(prefix)),
                "{path}"
            );
        }
    }

    #[test]
    fn write_files_touches_only_what_changed() {
        let scratch = tempfile::tempdir().expect("a directory");
        let files = vec![("a/b.txt".to_owned(), "one\n".to_owned())];
        assert_eq!(write_files(scratch.path(), &files).expect("written"), ["a/b.txt"]);
        assert!(write_files(scratch.path(), &files).expect("rewritten").is_empty());
        let changed = vec![("a/b.txt".to_owned(), "two\n".to_owned())];
        assert_eq!(write_files(scratch.path(), &changed).expect("changed"), ["a/b.txt"]);
    }
    /// The text of one generated file.
    fn generated(target: &str, path: &str) -> String {
        let spec = SdkSpec::load(&root()).expect("the specification loads");
        let rendered = render(&spec, Some(target)).expect("the target renders");
        files_of(&rendered)
            .into_iter()
            .find(|(file, _)| file == path)
            .unwrap_or_else(|| panic!("{path} is not rendered"))
            .1
    }

    #[test]
    fn a_python_stub_reads_an_object_as_absent_only_where_the_catalog_says_optional() {
        let model = generated("object-types", "python/gnoblin/objects/model.pyi");
        // `launches` is declared with no `optional`: a type checker must not make a script test it for `None`.
        let launches = model
            .lines()
            .find(|line| line.contains("def launches(self) ->"))
            .expect("the property is declared");
        assert!(!launches.contains("None"), "{launches}");
        // A member the catalog marks optional still answers `| None`.
        // (A member the schema describes has its docstring under the header, so the line ends at the colon.)
        assert!(
            model.lines().any(|line| line.contains("def ")
                && line.contains("-> ")
                && (line.ends_with("| None: ...") || line.ends_with("| None:"))),
            "an optional answer is `| None`"
        );
    }

    #[test]
    fn a_schema_description_is_the_comment_of_its_member_in_every_language() {
        let sentence = "True while the session has been idle for `ms` with no inhibitor.";
        let typescript = generated("object-types", "js/gnoblin-core/generated/objects.d.ts");
        assert!(typescript.contains(&format!("/** {sentence}")), "a TSDoc comment");
        let python = generated("object-types", "python/gnoblin/objects/model.pyi");
        assert!(python.contains(&format!("\"\"\"{sentence}")), "a docstring");
        let rust = generated("object-api", "rust-objects/model_api.rs");
        assert!(rust.contains(&format!("/// {sentence}")), "a doc comment");
        // A class has its description as well, and a member an augment adds to a core class has the augment's.
        assert!(typescript.contains("/** One idle watch."), "{}", &typescript[..400]);
        let moved = "Starts moving the window with the arrow keys.";
        let std = generated("object-types", "js/gnoblin-std/generated/objects.d.ts");
        assert!(std.contains(&format!("/** {moved}")), "the augment's member in TSDoc");
        let rust_std = generated("object-api", "rust-objects/std_api.rs");
        assert!(
            rust_std.contains(&format!("/// {moved}")),
            "the augment's member in a doc comment"
        );
    }

    #[test]
    fn a_description_that_would_end_a_comment_does_not() {
        let mut code = crate::text::CodeWriter::new("  ");
        code.jsdoc("A star-slash */ in the text.");
        code.pydoc("A \"\"\" in the text, and a quote at the end\"");
        let text = code.finish();
        assert_eq!(text.matches("*/").count(), 1, "only the comment's own end: {text}");
        assert!(text.contains("\\\"\\\"\\\""), "{text}");
        assert!(text.trim_end().ends_with("end\\\"\"\"\""), "{text}");
    }

    #[test]
    fn a_python_stub_names_the_base_of_a_derived_class() {
        let std = generated("object-types", "python/gnoblin/objects/std.pyi");
        assert!(std.contains("class Camera3D(SceneNode):"), "a camera is a scene node");
        assert!(std.contains("class View(SceneNode):"), "a view is a scene node");
    }

    /// A service is reached with `get`, in Rust as in TypeScript and Python, so a failure to reach it names `Class.get`: the target a
    /// script reads must be the same word in all three languages.
    #[test]
    fn a_rust_service_names_get_as_the_target_of_a_failed_get() {
        let model = generated("object-api", "rust-objects/model_api.rs");
        let mut services = 0;
        for (start, _) in model.match_indices("    pub fn get(objects: &Objects") {
            let body = &model[start..];
            let body = &body[..body.find("\n    }\n").expect("the function ends")];
            assert!(body.contains("\"get\")"), "{body}");
            assert!(!body.contains("\"new\")"), "{body}");
            services += 1;
        }
        assert!(services > 0, "the core catalog has a service");
    }
}

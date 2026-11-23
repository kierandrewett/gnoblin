//! Which stock library a shell runs on, decided before any module of it evaluates.
//!
//! A shell runs entirely on the native library (`gnoblin-core/generated/objects` and `gnoblin-std/generated/objects`: the classes
//! generated from the catalog, over the object protocol of the libraries) or entirely on the TypeScript library (`data/js`), never
//! on both: two object models in one generation would hand a script handles of one that the other does not know.
//!
//! The package entries `gnoblin-core` and `gnoblin-std` are the TypeScript library's until the generated classes cover what a shell
//! uses, and the XR extension (`gnoblin-std/extensions/xr`) is only there. So the choice reads the shell's static import graph:
//! it starts at the entry module and follows every import and re-export that names a file, through the resolution the loader uses,
//! and parses each file with the parser the loader strips with. The shell runs on the native library when
//!
//! - some module of the graph imports one of the generated object modules, and
//! - no module imports anything else of the stock library (a package entry, a deep module, the XR extension), and
//! - no module contains a dynamic `import()`, whose specifier is known only when it runs.
//!
//! Any other graph runs on the TypeScript library, which is what every shell ran on before the native one existed. A type-only
//! import is erased before the module runs and does not count. A graph that cannot be read to its end (a file that does not exist,
//! does not parse or cannot be resolved) is [`Choice::Unknown`] when nothing it did read needs the TypeScript library: the load fails
//! the same way on either library, and the caller keeps the library it already runs on so that the failure the author sees is the
//! real one.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::loader::resolve_file;

/// The generated modules a shell imports to run on the native library.
const NATIVE_MODULES: [&str; 3] = [
    "gnoblin-core/generated/objects",
    "gnoblin-core/generated/object_base",
    "gnoblin-std/generated/objects",
];

/// The stock packages: an import of one or of a module under it is an import of the stock library.
const PACKAGES: [&str; 2] = ["gnoblin-core", "gnoblin-std"];

/// The stock library a generation runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockRuntime {
    /// The classes generated from the catalog, over the object protocol of the libraries.
    Generated,
    /// The hand-written TypeScript library in `data/js`, over the engine's own object model.
    Typescript,
}

impl StockRuntime {
    /// The runtime as a word, for a message.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Typescript => "typescript",
        }
    }
}

/// What the import graph of a shell says about the library it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Every module was read, the shell imports the generated object modules and nothing else of the stock library.
    Generated,
    /// A module needs the TypeScript library, or no module needs the native one; the text says why.
    Typescript(String),
    /// The graph could not be read to its end and nothing read so far needs the TypeScript library.
    Unknown,
}

/// Whether `specifier` names `package` or a module under it.
fn in_package(specifier: &str, package: &str) -> bool {
    specifier == package
        || specifier
            .strip_prefix(package)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// What one module's own text says: why it needs the TypeScript library, whether it imports a generated object module, and the
/// specifiers of the files it imports.
struct Reads {
    typescript: Option<String>,
    native: bool,
    files: Vec<String>,
}

/// Reads one module: its static requests and its dynamic imports. `None` when the text does not parse.
fn read_module(path: &Path, source: &str) -> Option<Reads> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path)
        .unwrap_or_else(|_| SourceType::ts())
        .with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.errors().next().is_some() {
        return None;
    }
    let record = &parsed.module_record;
    let mut reads = Reads {
        typescript: None,
        native: false,
        files: Vec::new(),
    };
    for (specifier, requests) in &record.requested_modules {
        // `import type` is erased before the module runs, so it loads nothing.
        if requests.iter().all(|request| request.is_type) {
            continue;
        }
        let specifier: &str = specifier.as_str();
        if NATIVE_MODULES.contains(&specifier) {
            reads.native = true;
        } else if PACKAGES.iter().any(|package| in_package(specifier, package)) {
            reads.typescript.get_or_insert_with(|| {
                format!(
                    "{} imports {specifier}, which only the TypeScript library has",
                    path.display()
                )
            });
        } else if specifier.starts_with("./") || specifier.starts_with("../") || Path::new(specifier).is_absolute() {
            reads.files.push(specifier.to_owned());
        }
    }
    if !record.dynamic_imports.is_empty() {
        reads.typescript.get_or_insert_with(|| {
            format!(
                "{} contains an import() whose module is known only when it runs",
                path.display()
            )
        });
    }
    Some(reads)
}

/// The library the shell whose entry module is `entry` (a path under `root`, or absolute) needs.
pub(crate) fn choose(root: &Path, entry: &Path) -> Choice {
    let Ok(root) = fs::canonicalize(root) else {
        return Choice::Unknown;
    };
    let Some(first) = entry.to_str() else {
        return Choice::Unknown;
    };
    match resolve_file(&root, "", first) {
        Ok(entry) => walk(&root, vec![entry], false, false),
        Err(_) => Choice::Unknown,
    }
}

/// The library the shell needs whose entry module is `source`, the text of the file `path` (the starter, which the engine embeds
/// and does not load from the config root).
pub(crate) fn choose_source(root: &Path, path: &Path, source: &str) -> Choice {
    let Ok(root) = fs::canonicalize(root) else {
        return Choice::Unknown;
    };
    let Some(reads) = read_module(path, source) else {
        return Choice::Unknown;
    };
    if let Some(reason) = reads.typescript {
        return Choice::Typescript(reason);
    }
    let base = path.to_str().unwrap_or_default().to_owned();
    let mut files = Vec::new();
    let mut unknown = false;
    for specifier in &reads.files {
        match resolve_file(&root, &base, specifier) {
            Ok(file) => files.push(file),
            Err(_) => unknown = true,
        }
    }
    walk(&root, files, reads.native, unknown)
}

/// Follows the files of the graph that `pending` starts, each once. `native` and `unknown` are what the modules read before already said.
fn walk(root: &Path, mut pending: Vec<PathBuf>, mut native: bool, mut unknown: bool) -> Choice {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            unknown = true;
            continue;
        };
        let Some(reads) = read_module(&path, &source) else {
            unknown = true;
            continue;
        };
        if let Some(reason) = reads.typescript {
            return Choice::Typescript(reason);
        }
        native |= reads.native;
        let base = path.to_str().unwrap_or_default().to_owned();
        for specifier in &reads.files {
            match resolve_file(root, &base, specifier) {
                Ok(file) => pending.push(file),
                Err(_) => unknown = true,
            }
        }
    }
    if unknown {
        Choice::Unknown
    } else if native {
        Choice::Generated
    } else {
        Choice::Typescript(
            "it imports no generated object module, so the global `gnoblin` is the TypeScript library's".to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{Choice, choose, choose_source};

    fn root_with(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
        for (name, text) in files {
            fs::write(root.join(name), text).expect("the module is written");
        }
        (dir, root)
    }

    fn typescript(choice: &Choice) -> &str {
        match choice {
            Choice::Typescript(reason) => reason,
            other => panic!("expected the TypeScript library, got {other:?}"),
        }
    }

    const OBJECTS: &str = "import { Gnoblin } from \"gnoblin-core/generated/objects\";\n";

    #[test]
    fn a_shell_that_imports_only_generated_modules_through_static_imports_is_generated() {
        let (_dir, root) = root_with(&[
            (
                "shell.ts",
                "import { Gnoblin } from \"gnoblin-core/generated/objects\";\nimport { Workspaces } from \"gnoblin-std/generated/objects\";\nimport { helper } from \"./helper\";\nimport type { Other } from \"gnoblin-std/extensions/xr\";\nimport type {} from \"gnoblin-std\";\nhelper(Gnoblin, Workspaces);\n",
            ),
            (
                "helper.ts",
                "export const helper = (..._: unknown[]): void => {};\nexport * from \"./leaf.js\";\n",
            ),
            ("leaf.ts", "export const leaf = 1;\n"),
        ]);
        assert_eq!(choose(&root, Path::new("shell.ts")), Choice::Generated);
    }

    #[test]
    fn the_generated_modules_may_be_imported_by_a_helper_alone() {
        let (_dir, root) = root_with(&[("shell.ts", "import \"./helper\";\n"), ("helper.ts", OBJECTS)]);
        assert_eq!(choose(&root, Path::new("shell.ts")), Choice::Generated);
    }

    /// A shell whose only stock import is the std module, the one that puts the std members on the core's root, runs on the generated
    /// library, whether it names a class of it or imports it for its effect.
    #[test]
    fn a_shell_whose_only_stock_import_is_the_std_object_module_is_generated() {
        for text in [
            "import \"gnoblin-std/generated/objects\";\n",
            "import { Workspaces } from \"gnoblin-std/generated/objects\";\nvoid Workspaces;\n",
        ] {
            let (_dir, root) = root_with(&[("shell.ts", text)]);
            assert_eq!(choose(&root, Path::new("shell.ts")), Choice::Generated, "{text}");
        }
    }

    #[test]
    fn a_shell_that_imports_no_generated_module_stays_on_the_typescript_library() {
        let (_dir, root) = root_with(&[("shell.ts", "gnoblin.clients.length;\n")]);
        let choice = choose(&root, Path::new("shell.ts"));
        assert!(typescript(&choice).contains("global `gnoblin`"), "{choice:?}");
    }

    #[test]
    fn the_package_entries_and_their_deep_modules_are_the_typescript_librarys() {
        let (_dir, root) = root_with(&[
            (
                "entry.ts",
                "import { gnoblin } from \"gnoblin-core\";\nimport { Gnoblin } from \"gnoblin-core/generated/objects\";\n",
            ),
            (
                "deep.ts",
                "import { x } from \"gnoblin-std/layout\";\nimport { Gnoblin } from \"gnoblin-core/generated/objects\";\n",
            ),
        ]);
        assert!(typescript(&choose(&root, Path::new("entry.ts"))).contains("gnoblin-core"));
        assert!(typescript(&choose(&root, Path::new("deep.ts"))).contains("gnoblin-std/layout"));
    }

    #[test]
    fn an_entry_that_imports_the_xr_extension_is_typescript() {
        let (_dir, root) = root_with(&[(
            "shell.ts",
            "import \"gnoblin-std/extensions/xr\";\nimport { Gnoblin } from \"gnoblin-core/generated/objects\";\n",
        )]);
        let choice = choose(&root, Path::new("shell.ts"));
        assert!(typescript(&choice).contains("gnoblin-std/extensions/xr"), "{choice:?}");
    }

    #[test]
    fn a_helper_that_imports_the_xr_extension_makes_the_whole_shell_typescript() {
        let (_dir, root) = root_with(&[
            (
                "shell.ts",
                "import { Gnoblin } from \"gnoblin-core/generated/objects\";\nimport { helper } from \"./deep/helper\";\nhelper();\n",
            ),
            ("more.ts", "export * from \"gnoblin-std/extensions/xr/exports\";\n"),
        ]);
        fs::create_dir_all(root.join("deep")).expect("a directory");
        fs::write(
            root.join("deep/helper.ts"),
            "import \"../more\";\nexport const helper = (): void => {};\n",
        )
        .expect("the helper is written");
        let choice = choose(&root, &root.join("shell.ts"));
        assert!(typescript(&choice).contains("more.ts"), "{choice:?}");
    }

    #[test]
    fn a_dynamic_import_anywhere_in_the_graph_makes_the_shell_typescript() {
        let (_dir, root) = root_with(&[
            ("shell.ts", &format!("{OBJECTS}import \"./lazy\";\n")),
            ("lazy.ts", "export const load = (name: string) => import(name);\n"),
        ]);
        let choice = choose(&root, Path::new("shell.ts"));
        assert!(typescript(&choice).contains("import()"), "{choice:?}");
    }

    #[test]
    fn a_graph_that_cannot_be_read_is_unknown_unless_it_reached_the_typescript_library() {
        let (_dir, root) = root_with(&[
            ("missing.ts", &format!("{OBJECTS}import \"./nowhere\";\n")),
            ("broken.ts", "const x = ;\n"),
            (
                "both.ts",
                "import \"./nowhere\";\nimport \"gnoblin-std/extensions/xr\";\n",
            ),
        ]);
        assert_eq!(choose(&root, Path::new("missing.ts")), Choice::Unknown);
        assert_eq!(choose(&root, Path::new("broken.ts")), Choice::Unknown);
        assert_eq!(choose(&root, Path::new("absent.ts")), Choice::Unknown);
        assert!(matches!(choose(&root, Path::new("both.ts")), Choice::Typescript(_)));
    }

    #[test]
    fn the_starter_is_read_from_its_text() {
        let (_dir, root) = root_with(&[("helper.ts", "export const h = 1;\n")]);
        let path = Path::new("/stock/shell.ts");
        assert!(matches!(
            choose_source(&root, path, "import { gnoblin } from \"gnoblin-core\";\n"),
            Choice::Typescript(_)
        ));
        assert_eq!(choose_source(&root, path, OBJECTS), Choice::Generated);
    }
}

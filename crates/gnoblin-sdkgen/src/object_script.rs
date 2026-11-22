//! Targets `object-js`, `object-py` and `object-types`: the classes of each namespace a library serves, for the script languages.
//!
//! The Rust SDK has `object-api` (`object_api.rs`); these write the same surface for JavaScript, TypeScript and Python from the same
//! catalog, over the object protocol that `gnoblin-runtime-js` and `gnoblin-runtime-python` expose to a script (`__gnoblinObjects` and
//! the built-in module `_gnoblin_objects`). What each kind of class becomes:
//!
//! | kind          | JavaScript and TypeScript                        | Python                                              |
//! | ------------- | ------------------------------------------------ | --------------------------------------------------- |
//! | enum          | a string-literal union                           | an `IntEnum` (a name is accepted where one is read) |
//! | record        | an interface (`Init` form for the defaulted ones) | a `dataclass`                                       |
//! | handle        | a class with accessors, methods and `on`         | a class with properties, methods and `on`           |
//! | service       | a class of static functions, and `get()`         | the same                                            |
//! | interface     | a base class the shell extends                   | a base class the shell subclasses                   |
//! | callback      | a function                                       | a callable                                          |
//!
//! Nothing is written by hand per class: the constructor, a property's accessor, a method, a signal and the conversion of every value
//! are rendered from the catalog. The small runtime the classes share (`object_base`) is a fixed text each generator carries.
//!
//! Paths: `js/native/<package>/generated/objects.js` for each namespace, with `js/native/gnoblin-core/generated/object_base.js` once (the
//! modules the native library is, named by their import names: the JavaScript runtime embeds them apart from the TypeScript library),
//! and `js/<package>/generated/objects.d.ts` with `js/gnoblin-core/generated/object_base.d.ts` for the editor; `python/gnoblin/objects/{model,std}.py`
//! (and `.pyi`) with `python/gnoblin/objects/__init__.py` and `_base.py` once.

use std::collections::BTreeMap;

use gnoblin_schema::{CallbackDecl, Catalog, EnumDecl, Namespace, ObjectType};

use super::objects::stem_of;
use super::{Comment, File, SdkSpec, Target, object_js, object_py};

/// What a namespace is, for the script languages: the classes by name, and the enums and callbacks.
pub(super) struct ScriptView<'a> {
    pub(super) catalog: &'a Catalog,
    pub(super) namespace: &'a Namespace,
    pub(super) objects: BTreeMap<&'a str, &'a ObjectType>,
    pub(super) enums: BTreeMap<&'a str, &'a EnumDecl>,
    pub(super) callbacks: BTreeMap<&'a str, &'a CallbackDecl>,
}

impl<'a> ScriptView<'a> {
    pub(super) fn new(catalog: &'a Catalog, namespace: &'a Namespace) -> ScriptView<'a> {
        ScriptView {
            catalog,
            namespace,
            objects: catalog
                .objects(namespace)
                .map(|object| (object.name.as_str(), object))
                .collect(),
            enums: catalog
                .enums(namespace)
                .map(|declared| (declared.name.as_str(), declared))
                .collect(),
            callbacks: catalog
                .callbacks(namespace)
                .map(|declared| (declared.name.as_str(), declared))
                .collect(),
        }
    }

    /// The class `name` names, or why a reference to it cannot be rendered.
    pub(super) fn class(&self, name: &str) -> Result<&'a ObjectType, String> {
        if name.contains('.') {
            return Err(format!(
                "`{name}` is a class of another namespace: a typed reference across two libraries is not rendered, because it needs both \
                 libraries' handles"
            ));
        }
        self.objects
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` is not a class of `{}`", self.namespace))
    }

    pub(super) fn enum_decl(&self, name: &str) -> Result<&'a EnumDecl, String> {
        if name.contains('.') {
            return Err(format!(
                "`{name}` is an enum of another namespace, which is not rendered"
            ));
        }
        self.enums
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` is not an enum of `{}`", self.namespace))
    }

    pub(super) fn callback(&self, name: &str) -> Result<&'a CallbackDecl, String> {
        self.callbacks
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` is not a callback of `{}`", self.namespace))
    }
}

/// `text` as a string literal that JavaScript and Python both read: `\"`, `\\` and the control characters are escaped, the rest is kept.
pub(super) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// The package a namespace's generated JavaScript belongs to.
pub(super) fn package_of(namespace: &Namespace) -> Result<&'static str, String> {
    Ok(match stem_of(namespace)? {
        "model" => "gnoblin-core",
        _ => "gnoblin-std",
    })
}

/// The Python module (under `gnoblin.objects`) a namespace's generated classes are in.
pub(super) fn module_of(namespace: &Namespace) -> Result<&'static str, String> {
    stem_of(namespace)
}

/// Writes the JavaScript modules.
pub struct ObjectJsTarget;

impl Target for ObjectJsTarget {
    fn name(&self) -> &'static str {
        "object-js"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let mut files = vec![File {
            path: "js/native/gnoblin-core/generated/object_base.js".to_owned(),
            comment: Some(Comment::Slashes),
            text: object_js::BASE_JS.to_owned(),
        }];
        for namespace in spec.objects.namespaces() {
            files.push(File {
                path: format!("js/native/{}/generated/objects.js", package_of(&namespace)?),
                comment: Some(Comment::Slashes),
                text: object_js::render_js(&ScriptView::new(&spec.objects, &namespace))?,
            });
        }
        Ok(files)
    }
}

/// Writes the Python modules.
pub struct ObjectPyTarget;

impl Target for ObjectPyTarget {
    fn name(&self) -> &'static str {
        "object-py"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let mut files = vec![
            File {
                path: "python/gnoblin/objects/__init__.py".to_owned(),
                comment: Some(Comment::Hash),
                text: object_py::INIT_PY.to_owned(),
            },
            File {
                path: "python/gnoblin/objects/_base.py".to_owned(),
                comment: Some(Comment::Hash),
                text: object_py::BASE_PY.to_owned(),
            },
        ];
        for namespace in spec.objects.namespaces() {
            files.push(File {
                path: format!("python/gnoblin/objects/{}.py", module_of(&namespace)?),
                comment: Some(Comment::Hash),
                text: object_py::render(&ScriptView::new(&spec.objects, &namespace), false)?,
            });
        }
        Ok(files)
    }
}

/// Writes the declarations an editor reads: `.d.ts` for the JavaScript modules and `.pyi` for the Python ones.
pub struct ObjectTypesTarget;

impl Target for ObjectTypesTarget {
    fn name(&self) -> &'static str {
        "object-types"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let mut files = vec![
            File {
                path: "js/gnoblin-core/generated/object_base.d.ts".to_owned(),
                comment: Some(Comment::Slashes),
                text: object_js::BASE_DTS.to_owned(),
            },
            File {
                path: "python/gnoblin/objects/__init__.pyi".to_owned(),
                comment: Some(Comment::Hash),
                text: object_py::INIT_PYI.to_owned(),
            },
        ];
        for namespace in spec.objects.namespaces() {
            let view = ScriptView::new(&spec.objects, &namespace);
            files.push(File {
                path: format!("js/{}/generated/objects.d.ts", package_of(&namespace)?),
                comment: Some(Comment::Slashes),
                text: object_js::render_dts(&view)?,
            });
            files.push(File {
                path: format!("python/gnoblin/objects/{}.pyi", module_of(&namespace)?),
                comment: Some(Comment::Hash),
                text: object_py::render(&view, true)?,
            });
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use gnoblin_schema::Catalog;

    use super::{ObjectJsTarget, ObjectPyTarget, ObjectTypesTarget, ScriptView};
    use crate::{SdkSpec, Target, object_js, object_py, workspace_root};

    /// A tool on `PATH`, or the one the environment names.
    fn tool(name: &str, variable: &str) -> Option<PathBuf> {
        if let Some(explicit) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            return Some(PathBuf::from(explicit));
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// The files a target rendered, written under `dir` at their logical paths.
    fn write_all(dir: &Path, files: &[(String, String)]) {
        for (path, text) in files {
            let target = dir.join(path);
            fs::create_dir_all(target.parent().expect("a parent")).expect("a directory");
            fs::write(target, text).expect("a file");
        }
    }

    /// Rendering every kind of class (`probe`) and what both libraries serve, as the three targets do, for the checks below.
    fn rendered_files() -> Vec<(String, String)> {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let mut files = Vec::new();
        for target in [
            &ObjectJsTarget as &dyn Target,
            &ObjectPyTarget as &dyn Target,
            &ObjectTypesTarget as &dyn Target,
        ] {
            for file in target.generate(&spec).expect("the target renders") {
                files.push((file.path, file.text));
            }
        }
        let probe = workspace_root().join("crates/gnoblin-object/fixtures/probe/schema");
        let catalog = Catalog::load_dirs(&[probe.as_path()], &[]).expect("the probe catalog loads");
        let namespace = catalog.namespaces().remove(0);
        let view = ScriptView::new(&catalog, &namespace);
        files.push((
            "probe/objects.js".to_owned(),
            object_js::render_js(&view).expect("probe renders as JavaScript"),
        ));
        files.push((
            "probe/objects.d.ts".to_owned(),
            object_js::render_dts(&view).expect("probe renders as declarations"),
        ));
        files.push((
            "probe/probe.py".to_owned(),
            object_py::render(&view, false).expect("probe renders as Python"),
        ));
        files.push((
            "probe/probe.pyi".to_owned(),
            object_py::render(&view, true).expect("probe renders as a stub"),
        ));
        files
    }

    /// Every Python module compiles, and every stub parses. Skipped where there is no `python3`.
    #[test]
    fn the_python_modules_compile_and_the_stubs_parse() {
        let Some(python) = tool("python3", "GNOBLIN_PYTHON") else {
            eprintln!("python3 is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        write_all(dir.path(), &files);
        for (path, _) in files
            .iter()
            .filter(|(path, _)| path.ends_with(".py") || path.ends_with(".pyi"))
        {
            let parsed = Command::new(&python)
                .args([
                    "-c",
                    "import ast, sys; ast.parse(open(sys.argv[1], encoding='utf-8').read(), sys.argv[1])",
                ])
                .arg(dir.path().join(path))
                .output()
                .expect("python3 runs");
            assert!(
                parsed.status.success(),
                "{path} does not parse: {}",
                String::from_utf8_lossy(&parsed.stderr)
            );
        }
    }

    /// Every JavaScript module parses as a module. Skipped where there is no `node`.
    #[test]
    fn the_javascript_modules_parse() {
        let Some(node) = tool("node", "GNOBLIN_NODE") else {
            eprintln!("node is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        for (path, text) in files.iter().filter(|(path, _)| path.ends_with(".js")) {
            let copy = dir.path().join(path.replace('/', "_").replace(".js", ".mjs"));
            fs::write(&copy, text).expect("a file");
            let checked = Command::new(&node)
                .arg("--check")
                .arg(&copy)
                .output()
                .expect("node runs");
            assert!(
                checked.status.success(),
                "{path} does not parse: {}",
                String::from_utf8_lossy(&checked.stderr)
            );
        }
    }

    /// F6, F8, F9, F11, F12, F13, F15: the generated JavaScript value classes run under `node` against a stand-in for the native layer that
    /// hands a value back the way the library does (a record as an array of its fields, an enum as its id). Defaults start once and keep a
    /// zero or a `false` that is given, a required field and a field the class lacks are a `TypeError`, a family takes only the classes of
    /// its variants and reads back as the variant its tag says, and a tag no variant has is an error. Skipped where there is no `node`.
    #[test]
    fn the_javascript_value_classes_default_check_their_fields_and_tell_variants_apart() {
        let Some(node) = tool("node", "GNOBLIN_NODE") else {
            eprintln!("node is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        let text = |path: &str| {
            files
                .iter()
                .find(|(file, _)| file == path)
                .unwrap_or_else(|| panic!("{path} is not rendered"))
                .1
                .replace("gnoblin-core/generated/object_base", "./object_base.mjs")
        };
        fs::write(
            dir.path().join("object_base.mjs"),
            text("js/native/gnoblin-core/generated/object_base.js"),
        )
        .expect("the base");
        fs::write(
            dir.path().join("std.mjs"),
            text("js/native/gnoblin-std/generated/objects.js"),
        )
        .expect("std");
        fs::write(dir.path().join("probe.mjs"), text("probe/objects.js")).expect("probe");
        fs::write(
            dir.path().join("check.mjs"),
            r##"
const lib = (v) => (v && v.$t === 1 ? v.$v.map(lib) : v && v.$t === 2 ? v.$v : Array.isArray(v) ? v.map(lib) : v === undefined ? null : v);
globalThis.__gnoblinObjects = {
  open: () => 1,
  ids: (ns, name, names) => [0, ...names.map((_, i) => i + 1)],
  create: () => 7,
  release: () => {},
  classOf: () => "",
  call: (slot, id, args) => (globalThis.__answer ?? lib(args[0])),
};
const std = await import("./std.mjs");
const probe = await import("./probe.mjs");
const eq = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
const refuses = (what, run, words) => {
  try { run(); } catch (error) {
    if (!(error instanceof TypeError) || !error.message.includes(words)) throw new Error(`${what}: the error was ${error}`);
    return;
  }
  throw new Error(`${what}: nothing was refused`);
};

// F9: a default starts, and a value that is given stays, even when it is zero, false or the other variant's default.
eq("a border", new std.Border(), { width: 1, color: "#ffffff", offset: 0, inset: false });
eq("a border keeps a zero", new std.Border({ width: 0 }).width, 0);
eq("a shadow", new std.Shadow().blur, 12);
eq("a blur", new std.BlurFilter().radius, 12);
eq("a blur keeps a zero", new std.BlurFilter({ radius: 0 }).radius, 0);
eq("the grades", [new std.SaturateFilter().amount, new std.BrightnessFilter().amount, new std.ContrastFilter().amount], [1, 1, 1]);
const a = new std.ShaderFilter({ source: "x" });
a.params.push(1);
eq("a list default is new for each value", new std.ShaderFilter({ source: "x" }).params, []);
// F11, F12: a field that must be given, and a field the class lacks.
refuses("a shader needs its source", () => new std.ShaderFilter(), "ShaderFilter.source is required");
refuses("a typo", () => new std.Border({ wdith: 2 }), 'no field "wdith"');
refuses("not an object", () => new std.Border(3), "expects an object");
// F6: a call that takes a family takes the classes of its variants.
const { CircleShape, PolygonShape, Probe } = probe;
const shapes = [new CircleShape(), new CircleShape({ radius: 0 }), new PolygonShape({ points: [0, 1, 2] }), new PolygonShape({ points: [], closed: false })];
const p = new Probe(0);
const back = p.echoShapes(shapes);
eq("the shapes come back", back, shapes);
eq("each is its variant", back.map((s) => s.constructor.name), ["CircleShape", "CircleShape", "PolygonShape", "PolygonShape"]);
refuses("a plain object is not a shape", () => p.echoShapes([{ form: "circle" }]), "expected a Shape (one of CircleShape, PolygonShape)");
refuses("a record that is not a variant", () => p.echoShapes([new std.Border()]), "got Border");
// F8: a tag no variant has.
globalThis.__answer = [[9, null, null]];
refuses("an unknown kind", () => p.echoShapes([]), "which this SDK has no class for");
console.log("ok");
"##,
        )
        .expect("the check");
        let ran = Command::new(&node)
            .arg(dir.path().join("check.mjs"))
            .output()
            .expect("node runs");
        assert!(
            ran.status.success() && String::from_utf8_lossy(&ran.stdout).trim() == "ok",
            "{}{}",
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr)
        );
    }

    /// The declarations type-check under `strict`, and a script that uses them as the golden script does compiles while a wrong one does
    /// not. Skipped where there is no `tsc` (`GNOBLIN_TSC` names one).
    #[test]
    fn the_declarations_type_check_a_script_that_uses_them() {
        let Some(tsc) = tool("tsc", "GNOBLIN_TSC") else {
            eprintln!("tsc is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        write_all(dir.path(), &files);
        let root = dir.path().display();
        // The probe's module imports the runtime base like the library's own.
        fs::write(
            dir.path().join("tsconfig.json"),
            format!(
                r#"{{ "compilerOptions": {{ "strict": true, "noEmit": true, "target": "ES2023", "module": "ESNext",
                "moduleResolution": "bundler", "lib": ["ES2023"], "skipLibCheck": false,
                "paths": {{ "gnoblin-core/*": ["{root}/js/gnoblin-core/*"], "gnoblin-std/*": ["{root}/js/gnoblin-std/*"],
                           "probe": ["{root}/probe/objects.d.ts"] }} }},
                "files": ["{root}/use.ts"] }}"#
            ),
        )
        .expect("a tsconfig");
        let uses = r#"
import { EntryModel, FocusPolicy } from "gnoblin-std/generated/objects";
import type { FocusAction, FocusSettingsInit, TextRange } from "gnoblin-std/generated/objects";
import { Paint, Point, Probe, Span } from "probe";
import type { Listener } from "gnoblin-core/generated/object_base";
import type { Client } from "gnoblin-core/generated/objects";
const client: Client | undefined = undefined;

const entry = new EntryModel("hello");
const text: string = entry.text;
entry.text = "changed";
const range: TextRange | undefined = entry.selection;
const listener: Listener = entry.on("changed", () => {});
entry.on("textChanged", (value: string) => value.length);
const settings: FocusSettingsInit = { mode: "sloppy" };
const action: FocusAction = FocusPolicy.focusDecision(settings, { kind: "enter", held: false });
const raised: boolean = action.raise;
const probe = new Probe(3);
const total: number = probe.total([{ x: 1 }, { x: 2, label: "b", shade: "dark" }]);
const span: number = probe.lengthOf({ start: 1, end: 4 });
probe.onTick = (x: number) => x * 2;
class Red extends Paint {
    colour(): string { return "red"; }
}
probe.painter = new Red();
const greeting: string = probe.greet("ada");
const clamped: number = Probe.clamp(15, 0, 10);
void [text, range, listener, raised, total, span, greeting, clamped, client];
"#;
        fs::write(dir.path().join("use.ts"), uses).expect("a script");
        let checked = Command::new(&tsc)
            .args(["--noEmit", "--pretty", "false", "-p"])
            .arg(dir.path().join("tsconfig.json"))
            .output()
            .expect("tsc runs");
        assert!(
            checked.status.success(),
            "the declarations do not type-check:\n{}{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr)
        );
        // A wrong script fails with the names it got wrong: an enum that is not one, a record missing a field, an unknown event.
        let wrong = r#"
import { EntryModel, FocusPolicy } from "gnoblin-std/generated/objects";
const entry = new EntryModel("hello");
entry.on("nothing", () => {});
FocusPolicy.focusDecision({ mode: "sideways" }, { kind: "enter" });
entry.length = 4;
const number: number = entry.text;
"#;
        fs::write(dir.path().join("use.ts"), wrong).expect("a script");
        let refused = Command::new(&tsc)
            .args(["--noEmit", "--pretty", "false", "-p"])
            .arg(dir.path().join("tsconfig.json"))
            .output()
            .expect("tsc runs");
        let report = String::from_utf8_lossy(&refused.stdout).into_owned();
        assert!(!refused.status.success(), "a wrong script compiled:\n{report}");
        for wanted in [
            "\"nothing\"",
            "\"sideways\"",
            "read-only",
            "'string' is not assignable to type 'number'",
        ] {
            assert!(report.contains(wanted), "tsc does not complain of {wanted}:\n{report}");
        }
    }

    /// The generated Python classes of the `probe` catalog (one class of every kind) run against a stand-in for the runtime's native module:
    /// what a class sends and what it makes of what comes back, for an enum, a record, a list, a closure, an interface and a listener.
    #[test]
    fn the_python_classes_convert_every_kind_of_value() {
        let Some(python) = tool("python3", "GNOBLIN_PYTHON") else {
            eprintln!("python3 is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let files: Vec<(String, String)> = ObjectPyTarget
            .generate(&spec)
            .expect("the Python target renders")
            .into_iter()
            .map(|file| (file.path, file.text))
            .collect();
        write_all(dir.path(), &files);
        let probe = workspace_root().join("crates/gnoblin-object/fixtures/probe/schema");
        let catalog = Catalog::load_dirs(&[probe.as_path()], &[]).expect("the probe catalog loads");
        let namespace = catalog.namespaces().remove(0);
        let module = object_py::render(&ScriptView::new(&catalog, &namespace), false).expect("probe renders as Python");
        let package = dir.path().join("python/gnoblin");
        fs::write(package.join("__init__.py"), "").expect("the package");
        fs::write(package.join("objects/probe.py"), module).expect("the probe module");
        let tests = workspace_root().join("crates/gnoblin-sdkgen/tests/python");
        fs::copy(
            tests.join("fake_gnoblin_objects.py"),
            dir.path().join("python/_gnoblin_objects.py"),
        )
        .expect("the stand-in");
        let output = Command::new(&python)
            .arg(tests.join("check_probe.py"))
            .env("PYTHONPATH", dir.path().join("python"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("python3 runs");
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A class that extends one of another namespace has the members of its base, so a script reads them off the derived class.
    #[test]
    fn a_class_of_another_namespace_is_flat_and_its_base_members_are_its_own() {
        let root = tempfile::tempdir().expect("a directory");
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::create_dir_all(&a).expect("a directory");
        fs::create_dir_all(&b).expect("a directory");
        fs::write(
            a.join("thing.toml"),
            "namespace = \"lib.a\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
             [[object.property]]\nname = \"count\"\ntype = \"int\"\nsince = \"0.1.0\"\n\n\
             [[object.method]]\nname = \"bump\"\nsince = \"0.1.0\"\nreturns = \"int\"\n",
        )
        .expect("a schema");
        fs::write(
            b.join("fancy.toml"),
            "namespace = \"lib.b\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"fancy\"\nsince = \"0.1.0\"\nkind = \"handle\"\nbase = \"lib.a.thing\"\n\n\
             [[object.property]]\nname = \"label\"\ntype = \"string\"\nsince = \"0.1.0\"\n",
        )
        .expect("a schema");
        let catalog = Catalog::load_dirs(&[a.as_path(), b.as_path()], &[]).expect("the catalogs load together");
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[1]);
        let js = object_js::render_js(&view).expect("the derived class renders");
        assert!(
            js.contains(r#"["p:count", "m:bump", "p:label"]"#),
            "the base members come first:\n{js}"
        );
        assert!(
            js.contains("get count()") && js.contains("bump()") && js.contains("get label()"),
            "{js}"
        );
        let py = object_py::render(&view, false).expect("the derived class renders in Python");
        assert!(py.contains("def count(self)") && py.contains("def bump(self)"), "{py}");
    }

    /// A method marked `optional` answers none in each language, and one that is not marked does not: `Option<T>` in Rust, `T | undefined`
    /// in the declarations and `T | None` in Python, with the decoder that passes a none answer through, and the flag in the tables.
    #[test]
    fn an_optional_result_is_none_able_in_every_language_and_flagged_for_the_library() {
        let root = tempfile::tempdir().expect("a directory");
        fs::write(
            root.path().join("thing.toml"),
            "namespace = \"lib.a\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
             [[object.method]]\nname = \"find\"\nsince = \"0.1.0\"\nreturns = \"object:thing\"\noptional = true\n\n\
             [[object.method.param]]\nname = \"id\"\ntype = \"string\"\n\n\
             [[object.method]]\nname = \"first\"\nsince = \"0.1.0\"\nreturns = \"object:thing\"\n",
        )
        .expect("a schema");
        let catalog = Catalog::load_dirs(&[root.path()], &[]).expect("a catalog");
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);

        let rust = crate::object_api::render(&catalog, &namespaces[0]).expect("the typed API renders");
        assert!(
            rust.contains("pub fn find(&self, id: &str) -> go::Res<Option<Thing>>"),
            "{rust}"
        );
        assert!(
            rust.contains("pub fn first(&self) -> go::Res<Thing>"),
            "a present result is unchanged: {rust}"
        );

        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(dts.contains("find(id: string): Thing | undefined;"), "{dts}");
        assert!(dts.contains("first(): Thing;"), "{dts}");
        let js = object_js::render_js(&view).expect("the JavaScript renders");
        assert!(js.contains("B.opt(N.call("), "a none answer is undefined: {js}");

        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(stub.contains("def find(self, id: str) -> Thing | None"), "{stub}");
        assert!(stub.contains("def first(self) -> Thing"), "{stub}");
        let py = object_py::render(&view, false).expect("the Python renders");
        assert!(py.contains("_B.opt(self._h.call("), "a none answer is None: {py}");

        let tables = crate::objects::render(&catalog, &namespaces[0]).expect("the tables render");
        assert_eq!(
            tables.matches("abi::MEMBER_OPTIONAL").count(),
            1,
            "only the marked method: {tables}"
        );
    }
    /// A catalog with a service `root` whose `find` knows two kinds, `Thing` and `Other`.
    fn finder_catalog(dir: &Path) -> Catalog {
        fs::write(
            dir.join("root.toml"),
            "namespace = \"lib.r\"\nversion = \"0.1.0\"\nlocal = true\n\n\
             [[object]]\nname = \"query\"\nsince = \"0.1.0\"\nkind = \"record\"\n\n\
             [[object.property]]\nname = \"label\"\ntype = \"string\"\nsince = \"0.1.0\"\noptional = true\n\n\
             [[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
             [[object]]\nname = \"other\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
             [[object]]\nname = \"root\"\nsince = \"0.1.0\"\nkind = \"service\"\n\n\
             [[object.method]]\nname = \"find_things\"\nsince = \"0.1.0\"\nreturns = \"list<object:thing>\"\nfind = \"Thing\"\n\n\
             [[object.method.param]]\nname = \"query\"\ntype = \"object:query\"\noptional = true\n\n\
             [[object.method]]\nname = \"find_others\"\nsince = \"0.1.0\"\nreturns = \"list<object:other>\"\nfind = \"Other\"\n\n\
             [[object.method.param]]\nname = \"query\"\ntype = \"object:query\"\noptional = true\n",
        )
        .expect("a schema");
        Catalog::load_dirs(&[dir], &[]).expect("a catalog")
    }

    /// A service with `find` methods gets `find(kind, filter)` in every language, written from the kinds the catalog names, and a service
    /// without them gets none.
    #[test]
    fn a_service_with_finders_has_find_in_every_language() {
        let root = tempfile::tempdir().expect("a directory");
        let catalog = finder_catalog(root.path());
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);

        let js = object_js::render_js(&view).expect("the JavaScript renders");
        assert!(
            js.contains(
                r#"const finders_root = { Thing: ["findThings", () => Query], Other: ["findOthers", () => Query] };"#
            ),
            "{js}"
        );
        assert!(
            js.contains(r#"return B.find(this, "root", finders_root, kind, filter);"#),
            "{js}"
        );
        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(
            dts.contains(r#"find(kind: "Thing", filter?: "#) && dts.contains("((found: Thing) => unknown)): Thing[];"),
            "{dts}"
        );
        assert!(
            dts.contains(r#"find(kind: "Other""#) && dts.contains("find(kind: string,"),
            "{dts}"
        );

        let py = object_py::render(&view, false).expect("the Python renders");
        assert!(
            py.contains(r#"return _B.find(self, "root", {"Thing": ("find_things", Query), "Other": ("find_others", Query)}, kind, filter)"#),
            "{py}"
        );
        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(stub.contains(r#"def find(self, kind: Literal["Thing"]"#), "{stub}");
        assert!(stub.contains(r#"def find(self, kind: Literal["Other"]"#), "{stub}");

        let rust = crate::object_api::render(&catalog, &namespaces[0]).expect("the typed API renders");
        assert!(rust.contains("pub trait Finds<S>: Sized"), "{rust}");
        assert!(
            rust.contains("pub fn find<T: Finds<Root>>(&self, filter: Option<&T::Filter>)"),
            "{rust}"
        );
        assert!(
            rust.contains(
                "impl Finds<Root> for Thing {\n    type Filter = Query;\n    const KIND: &'static str = \"Thing\";"
            ),
            "{rust}"
        );
        assert!(rust.contains("service.find_others(filter)"), "{rust}");

        // The catalog of the library without finders has no `find`: nothing is added to a service that names no kind.
        let plain = tempfile::tempdir().expect("a directory");
        fs::write(
            plain.path().join("plain.toml"),
            "namespace = \"lib.p\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"root\"\nsince = \"0.1.0\"\nkind = \"service\"\n\n\
             [[object.method]]\nname = \"ping\"\nsince = \"0.1.0\"\n",
        )
        .expect("a schema");
        let catalog = Catalog::load_dirs(&[plain.path()], &[]).expect("a catalog");
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);
        assert!(!object_js::render_js(&view).expect("renders").contains("find("));
        assert!(!object_py::render(&view, false).expect("renders").contains("def find"));
        assert!(
            !crate::object_api::render(&catalog, &namespaces[0])
                .expect("renders")
                .contains("Finds")
        );
    }

    /// `B.find` of the JavaScript runtime: a table goes to the method of the kind, a function filters what the method lists, and a kind that is
    /// not listed (or one that only the object prototype has) is refused with the kinds that work. Skipped where there is no `node`.
    #[test]
    fn the_javascript_find_picks_the_method_of_the_kind_and_refuses_the_rest() {
        let Some(node) = tool("node", "GNOBLIN_NODE") else {
            eprintln!("node is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        let base = files
            .iter()
            .find(|(file, _)| file == "js/native/gnoblin-core/generated/object_base.js")
            .expect("the base is rendered");
        fs::write(dir.path().join("object_base.mjs"), &base.1).expect("the base");
        fs::write(
            dir.path().join("check.mjs"),
            r##"
globalThis.__gnoblinObjects = {};
const B = await import("./object_base.mjs");
const calls = [];
const service = {
  findThings(filter) { calls.push(["findThings", filter]); return [{ n: 1 }, { n: 2 }, { n: 3 }]; },
  findOthers(filter) { calls.push(["findOthers", filter]); return []; },
};
class Query {
  constructor(init = {}) {
    B.fields(init, "Query", ["label"]);
    this.label = init.label;
  }
}
const finders = { Thing: ["findThings", () => Query], Other: ["findOthers", () => Query] };
const find = (kind, filter) => B.find(service, "gnoblin", finders, kind, filter);
const eq = (what, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
const refuses = (what, run, words) => {
  try { run(); } catch (error) {
    if (!(error instanceof TypeError) || !words.every((word) => error.message.includes(word))) throw new Error(`${what}: the error was ${error}`);
    return;
  }
  throw new Error(`${what}: nothing was refused`);
};
eq("every object", find("Thing"), [{ n: 1 }, { n: 2 }, { n: 3 }]);
eq("a table goes to the method as the filter class", (find("Other", { label: "x" }), calls.at(-1)[1] instanceof Query && calls.at(-1)), ["findOthers", { label: "x" }]);
const own = new Query({ label: "y" });
find("Other", own);
eq("a filter that is the class goes through as it is", calls.at(-1)[1] === own, true);
refuses("a field the class lacks", () => find("Thing", { lable: "x" }), ['no field "lable"', "known: label"]);
eq("a refused table called no method", calls.at(-1)[0], "findOthers");
eq("a function keeps what it likes", find("Thing", (found) => found.n !== 2), [{ n: 1 }, { n: 3 }]);
eq("a function is not handed to the method", calls.at(-1), ["findThings", undefined]);
eq("a function gets one argument", (() => { const seen = []; find("Thing", (...args) => seen.push(args.length)); return seen; })(), [1, 1, 1]);
refuses("a kind the service lacks", () => find("Workspace"), ['no kind "Workspace"', "the kinds that work are Thing, Other"]);
refuses("a prototype name", () => find("toString"), ['no kind "toString"']);
refuses("the prototype itself", () => find("__proto__"), ['no kind "__proto__"']);
refuses("a kind that is not a string", () => find(3), ["no kind 3"]);
refuses("no kind at all", () => find(), ["the kinds that work"]);
console.log("ok");
"##,
        )
        .expect("the check");
        let ran = Command::new(&node)
            .arg(dir.path().join("check.mjs"))
            .output()
            .expect("node runs");
        assert!(
            ran.status.success() && String::from_utf8_lossy(&ran.stdout).trim() == "ok",
            "{}{}",
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr)
        );
    }

    /// `_B.find` of the Python runtime: a query, a dict and a function are filters, and a kind or a field the service lacks is refused with
    /// what works. Skipped where there is no `python3`.
    #[test]
    fn the_python_find_picks_the_method_of_the_kind_and_refuses_the_rest() {
        let Some(python) = tool("python3", "GNOBLIN_PYTHON") else {
            eprintln!("python3 is not installed: the check is skipped");
            return;
        };
        let dir = tempfile::tempdir().expect("a directory");
        let files = rendered_files();
        let base = files
            .iter()
            .find(|(file, _)| file == "python/gnoblin/objects/_base.py")
            .expect("the base is rendered");
        let package = dir.path().join("gnoblin/objects");
        fs::create_dir_all(&package).expect("a package");
        fs::write(dir.path().join("gnoblin/__init__.py"), "").expect("the package");
        fs::write(package.join("__init__.py"), "").expect("the package");
        fs::write(package.join("_base.py"), &base.1).expect("the base");
        fs::write(dir.path().join("_gnoblin_objects.py"), "").expect("the native stand-in");
        fs::write(
            dir.path().join("check.py"),
            r#"
import dataclasses

from gnoblin.objects import _base as B


@dataclasses.dataclass
class Query:
    label: str | None = None
    width: float | None = None


calls = []


class Service:
    def find_things(self, query=None):
        calls.append(("find_things", query))
        return [1, 2, 3]

    def find_others(self, query=None):
        calls.append(("find_others", query))
        return []


finders = {"Thing": ("find_things", Query), "Other": ("find_others", Query)}


def find(kind, filter=None):
    return B.find(Service(), "gnoblin", finders, kind, filter)


def refuses(what, run, *words):
    try:
        run()
    except TypeError as error:
        assert all(word in str(error) for word in words), f"{what}: the error was {error}"
        return
    raise AssertionError(f"{what}: nothing was refused")


assert find("Thing") == [1, 2, 3]
assert calls[-1] == ("find_things", None)
assert find("Other", {"label": "x"}) == [] and calls[-1] == ("find_others", Query(label="x")), calls[-1]
query = Query(width=4)
find("Other", query)
assert calls[-1] == ("find_others", query), "a query goes through as it is"
assert find("Thing", lambda found: found != 2) == [1, 3]
assert calls[-1] == ("find_things", None), "a function is not handed to the method"
refuses("a kind the service lacks", lambda: find("Workspace"), "'Workspace'", "the kinds that work are Thing, Other")
refuses("a kind that is not a string", lambda: find(3), "no kind 3")
refuses("a field the query lacks", lambda: find("Thing", {"lable": "x"}), "'lable'", "known: label, width", "'Thing'")
refuses("a filter that is nothing of the sort", lambda: find("Thing", 7), "Query", "int")
print("ok")
"#,
        )
        .expect("the check");
        let ran = Command::new(&python)
            .arg(dir.path().join("check.py"))
            .env("PYTHONPATH", dir.path())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("python3 runs");
        assert!(
            ran.status.success() && String::from_utf8_lossy(&ran.stdout).trim() == "ok",
            "{}{}",
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr)
        );
    }
}

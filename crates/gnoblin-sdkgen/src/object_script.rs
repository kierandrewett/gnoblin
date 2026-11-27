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
//! A parameter may name a handle of the other library's catalog (`object:gnoblin.model.client` in `gnoblin.std`): the module imports that
//! class and refuses a value that is not an instance of it, because the wire carries no namespace and the two libraries issue the same
//! handle numbers. A result, a property or a signal field of such a class is refused when the module is written.
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

use super::declarations::{Declarations, note};
use super::describe::member_text;
use super::docs::{DocLanguage, NO_DOCS, NamespaceDocs, base_of};
use super::objects::{foreign_handle, foreign_handles, stem_of};
use super::text::pascal;
use super::{Comment, File, SdkSpec, Target, object_js, object_py, values};

/// What a namespace is, for the script languages: the classes by name, and the enums and callbacks.
pub(super) struct ScriptView<'a> {
    pub(super) catalog: &'a Catalog,
    pub(super) namespace: &'a Namespace,
    pub(super) objects: BTreeMap<&'a str, &'a ObjectType>,
    pub(super) enums: BTreeMap<&'a str, &'a EnumDecl>,
    pub(super) callbacks: BTreeMap<&'a str, &'a CallbackDecl>,
    /// What the schema files say about the classes; empty for a catalog that has no comments (a test fixture).
    pub(super) docs: &'a NamespaceDocs,
    /// The language the view's comments are written for.
    pub(super) flavor: DocLanguage,
    /// Where the declarations written are noted, for the reference model.
    pub(super) declarations: Option<&'a Declarations>,
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
            docs: &NO_DOCS,
            flavor: DocLanguage::TypeScript,
            declarations: None,
        }
    }

    /// The same view, with the descriptions the schema files carry.
    pub(super) fn with_docs(mut self, docs: &'a NamespaceDocs) -> ScriptView<'a> {
        self.docs = docs;
        self
    }

    /// The same view, writing its comments for `flavor`.
    pub(super) fn with_flavor(mut self, flavor: DocLanguage) -> ScriptView<'a> {
        self.flavor = flavor;
        self
    }

    /// The same view, noting each declaration it writes in `sink`.
    pub(super) fn with_declarations(mut self, sink: &'a Declarations) -> ScriptView<'a> {
        self.declarations = Some(sink);
        self
    }

    /// Notes that the member `member` of `class` was declared as `text`.
    pub(super) fn declared(&self, class: &str, member: &str, text: impl AsRef<str>) {
        note(self.declarations, class, member, text);
    }

    /// Whether the view carries the schema's descriptions.
    pub(super) fn has_docs(&self) -> bool {
        !std::ptr::eq(self.docs, &NO_DOCS)
    }

    /// The description of the class `name`, or an empty text.
    pub(super) fn class_doc(&self, name: &str) -> &'a str {
        self.docs.class(name)
    }

    /// What the schema says about the member `member` of the class `class` (a property, a method or a signal), written for the view's
    /// language: its description from the class or the nearest base that has one, its parameters, range, unit, default, the codes it
    /// raises and its example. Empty for a view without descriptions.
    pub(super) fn member_doc(&self, class: &str, member: &str) -> String {
        if !self.has_docs() {
            return String::new();
        }
        member_text(self.docs, &self.objects, self.flavor, class, member)
    }

    /// The description of the member `member` of the class `class` and nothing else, from the class or the nearest base that has one.
    pub(super) fn description(&self, class: &str, member: &str) -> &'a str {
        self.docs.member(class, member, |name| base_of(&self.objects, name))
    }

    /// The description of the value `value` of the enum `name`, or an empty text.
    pub(super) fn enum_value_doc(&self, name: &str, value: &str) -> &'a str {
        self.docs.enum_value(name, value)
    }

    /// The description of the enum `name`, or an empty text.
    pub(super) fn enum_doc(&self, name: &str) -> &'a str {
        self.docs.enum_doc(name)
    }

    /// The class `name` names, or why a reference to it cannot be rendered.
    pub(super) fn class(&self, name: &str) -> Result<&'a ObjectType, String> {
        if name.contains('.') {
            return Err(format!(
                "`{name}` is a class of another namespace: it can be a parameter, but a result, a property or a signal field of it is not \
                 rendered, because it needs the other library's handles"
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

/// A handle class of another namespace that a type of this one names, and what the generated module calls it.
pub(super) struct ForeignClass {
    /// The class's name in JavaScript and TypeScript.
    pub(super) js: String,
    /// The class's name in Python.
    pub(super) py: String,
    /// The name this module imports it as: the other namespace's stem and the class (`ModelClient`), so two namespaces' classes of one
    /// name never meet.
    pub(super) alias: String,
    /// The package the class's JavaScript module belongs to (`gnoblin-core`).
    pub(super) package: &'static str,
    /// The Python module of the class (`model`).
    pub(super) module: &'static str,
    /// The namespace that serves the class (`gnoblin.model`), which an object of it that this library hands out is read in.
    pub(super) namespace: String,
}

impl ForeignClass {
    /// The name the Python module imports the class as: a private name, so a shell's `from gnoblin.objects.std import *` does not bring
    /// the other namespace's class along.
    pub(super) fn alias_py(&self) -> String {
        format!("_{}", self.alias)
    }
}

impl<'a> ScriptView<'a> {
    /// The handle class of another namespace that `reference` (`gnoblin.model.client`) names.
    pub(super) fn foreign(&self, reference: &str) -> Result<ForeignClass, String> {
        let (home, class) = foreign_handle(self.catalog, reference)?;
        Ok(ForeignClass {
            js: values::js_class(&class.name, &class.export),
            py: values::py_class(&class.name, &class.export),
            alias: format!(
                "{}{}",
                pascal(stem_of(&home)?),
                values::js_class(&class.name, &class.export)
            ),
            package: package_of(&home)?,
            module: module_of(&home)?,
            namespace: home.as_str().to_owned(),
        })
    }

    /// The classes of other namespaces this namespace's types name, once each: what the module imports.
    pub(super) fn foreigns(&self) -> Result<Vec<ForeignClass>, String> {
        foreign_handles(self.catalog, self.namespace)?
            .into_iter()
            .map(|(home, class)| self.foreign(&format!("{home}.{}", class.name)))
            .collect()
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
            let view = ScriptView::new(&spec.objects, &namespace).with_docs(spec.docs.namespace(&namespace));
            files.push(File {
                path: format!("js/{}/generated/objects.d.ts", package_of(&namespace)?),
                comment: Some(Comment::Slashes),
                text: object_js::render_dts(&view)?,
            });
            let python = ScriptView::new(&spec.objects, &namespace)
                .with_docs(spec.docs.namespace(&namespace))
                .with_flavor(DocLanguage::Python);
            files.push(File {
                path: format!("python/gnoblin/objects/{}.pyi", module_of(&namespace)?),
                comment: Some(Comment::Hash),
                text: object_py::render(&python, true)?,
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

    use gnoblin_schema::{Catalog, Namespace};

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
                .replace("gnoblin-core/generated/objects", "./model.mjs")
        };
        fs::write(
            dir.path().join("object_base.mjs"),
            text("js/native/gnoblin-core/generated/object_base.js"),
        )
        .expect("the base");
        // The std classes take the core's `Client` and add members to the core's classes, so the module they import is there too.
        fs::write(
            dir.path().join("model.mjs"),
            text("js/native/gnoblin-core/generated/objects.js"),
        )
        .expect("the core's classes");
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
  create: (ns, cls, inits) => { (globalThis.__created ??= []).push(inits); return 7; },
  release: () => {},
  guard: () => ({}),
  classOf: () => "",
  key: (() => { let n = 0; return () => String(++n); })(),
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
// The options of a constructor: a field that is given is sent, in the order of the catalog, in the call that makes the object; one that is
// left out or undefined is not.
const sent = (make) => { globalThis.__created = []; make(); return globalThis.__created[0].map(([member]) => member); };
eq("the positional property alone", sent(() => new Probe(1)), [1]);
eq("no option", sent(() => new Probe(1, {})), [1]);
eq("an option", sent(() => new Probe(1, { shade: "dark" })), [1, 2]);
eq("options in the catalog's order", sent(() => new Probe(1, { names: ["a"], shade: "dark" })), [1, 2, 4]);
eq("an undefined option is not sent", sent(() => new Probe(1, { shade: undefined })), [1]);
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
const configured = new Probe(3, { shade: "dark", names: ["a"], onTick: (x: number) => x, origin: { x: 1 } });
probe.onTick = (x: number) => x * 2;
class Red extends Paint {
    colour(): string { return "red"; }
}
probe.painter = new Red();
const greeting: string = probe.greet("ada");
const clamped: number = Probe.clamp(15, 0, 10);
void [text, range, listener, raised, total, span, greeting, clamped, client, configured];
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
import { Probe } from "probe";
new Probe(3, { count: 4 });
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
            "ProbeOptions",
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
        fs::copy(tests.join("fake_gnoblin_errors.py"), package.join("errors.py")).expect("the errors stand-in");
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
    /// The same three properties, an object that may be absent (`plain`), one the catalog marks `optional` and one it says the library
    /// `always` has, in every language: `T | None` unless `always` in Python, as `T | undefined` and `Option<T>` are, and the module reads
    /// an `always` property with the check that refuses a none answer.
    #[test]
    fn python_reads_an_object_as_none_able_unless_the_catalog_says_always() {
        let root = tempfile::tempdir().expect("a directory");
        let catalog = thing_catalog(
            root.path(),
            "[[object.property]]\nname = \"plain\"\ntype = \"object:thing\"\nsince = \"0.1.0\"\n\n\
             [[object.property]]\nname = \"maybe\"\ntype = \"object:thing\"\nsince = \"0.1.0\"\noptional = true\n\n\
             [[object.property]]\nname = \"held\"\ntype = \"object:thing\"\nsince = \"0.1.0\"\nalways = true\n",
        );
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);

        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(stub.contains("def plain(self) -> Thing | None"), "{stub}");
        assert!(stub.contains("def maybe(self) -> Thing | None"), "{stub}");
        assert!(stub.contains("def held(self) -> Thing: ..."), "{stub}");
        assert!(!stub.contains("def held(self) -> Thing | None"), "{stub}");
        // The change of a property hands over the type the property reads as.
        assert!(
            stub.contains("event: Literal[\"plain_changed\"], handler: Callable[[Thing | None], object]"),
            "{stub}"
        );
        assert!(
            stub.contains("event: Literal[\"held_changed\"], handler: Callable[[Thing], object]"),
            "{stub}"
        );

        let py = object_py::render(&view, false).expect("the Python renders");
        assert!(py.contains("_B.opt(self._h.get("), "a none answer is None: {py}");
        assert!(
            py.contains("_B.always(self._h.get(") && py.contains("\"Thing.held\""),
            "an `always` property refuses a none answer and says which: {py}"
        );
        assert_eq!(py.matches("_B.always(").count(), 1, "only the marked property: {py}");

        // The other two languages read it the same way.
        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(
            dts.contains("get plain(): Thing | undefined;") && dts.contains("  held: Thing;"),
            "{dts}"
        );
        let rust = crate::object_api::render(&catalog, &namespaces[0]).expect("the typed API renders");
        assert!(
            rust.contains("pub fn plain(&self) -> go::Res<Option<Thing>>")
                && rust.contains("pub fn held(&self) -> go::Res<Thing>"),
            "{rust}"
        );
    }

    /// The generated classes run against the stand-in for the native module: an object property that was never set reads `None`, and an
    /// `always` one that the library answers with none is a `GnoblinError` that names it, not a `None` under a type that has none.
    #[test]
    fn an_always_property_the_library_answers_with_none_is_refused_where_it_is_read() {
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
        let schema = tempfile::tempdir().expect("a directory");
        let catalog = thing_catalog(
            schema.path(),
            "[[object.property]]\nname = \"plain\"\ntype = \"object:thing\"\nsince = \"0.1.0\"\n\n\
             [[object.property]]\nname = \"held\"\ntype = \"object:thing\"\nsince = \"0.1.0\"\nalways = true\n",
        );
        let namespace = catalog.namespaces().remove(0);
        let module = object_py::render(&ScriptView::new(&catalog, &namespace), false).expect("renders as Python");
        let package = dir.path().join("python/gnoblin");
        fs::write(package.join("__init__.py"), "").expect("the package");
        fs::write(package.join("objects/held.py"), module).expect("the module");
        let tests = workspace_root().join("crates/gnoblin-sdkgen/tests/python");
        fs::copy(tests.join("fake_gnoblin_errors.py"), package.join("errors.py")).expect("the errors stand-in");
        fs::copy(
            tests.join("fake_gnoblin_objects.py"),
            dir.path().join("python/_gnoblin_objects.py"),
        )
        .expect("the stand-in");
        let output = Command::new(&python)
            .arg(tests.join("check_always.py"))
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

    /// Every member of the root `gnoblin` of the catalog is a member of the stub class `GnoblinRoot`, which `from gnoblin import gnoblin`
    /// extends: the list is the catalog's, not written by hand. A property the catalog says is `always` there reads as the object, and
    /// `Gnoblin` is that class with `get`.
    #[test]
    fn every_member_of_the_root_is_in_the_stub_the_gnoblin_object_extends() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let namespaces = spec.objects.namespaces();
        let namespace = namespaces
            .iter()
            .find(|namespace| namespace.as_str() == "gnoblin.model")
            .expect("the model namespace");
        let view = ScriptView::new(&spec.objects, namespace);
        let stub = object_py::render(&view, true).expect("the stub renders");
        let start = stub
            .find("\nclass GnoblinRoot(")
            .expect("the stub has the root's members");
        let end = start
            + stub[start..]
                .find("\nclass Gnoblin(GnoblinRoot):")
                .expect("Gnoblin extends them");
        let members = &stub[start..end];
        let class = &stub[end..];
        assert!(
            class.contains("def get(cls) -> Gnoblin: ...") && !members.contains("def get("),
            "only the class has the constructor:\n{class}"
        );
        let root = view
            .objects
            .values()
            .find(|object| object.name == "gnoblin" && object.kind == gnoblin_schema::ObjectKind::Service)
            .expect("the root service");
        let mut always = Vec::new();
        for slot in crate::objects::slots_of(&spec.objects, &view.objects, root) {
            match slot.member {
                crate::objects::Member::Property(property) => {
                    let ident = object_py::member_name(&property.name, &property.extras.export.py);
                    let line = members
                        .lines()
                        .find(|line| line.trim_start().starts_with(&format!("def {ident}(self) -> ")))
                        .unwrap_or_else(|| panic!("the root's property `{ident}` is in the stub:\n{members}"));
                    if property.always {
                        always.push(ident);
                        assert!(
                            !line.contains("| None"),
                            "an `always` property is not none-able: {line}"
                        );
                    }
                }
                crate::objects::Member::Method(method) => {
                    let ident = object_py::member_name(&method.name, &method.extras.export.py);
                    assert!(
                        members.contains(&format!("def {ident}(self")),
                        "the root's method `{ident}` is in the stub:\n{members}"
                    );
                }
                crate::objects::Member::Signal(_) => {}
            }
        }
        for wanted in [
            "clients",
            "outputs",
            "focus",
            "session",
            "runtime",
            "process",
            "idle",
            "launches",
            "input",
            "dialogs",
            "authentication",
            "lock",
            "timers",
            "compositor",
        ] {
            assert!(
                always.iter().any(|ident| ident == wanted),
                "`{wanted}` is `always`: {always:?}"
            );
        }
        assert!(
            members.contains("def on(self, event: Literal[\"clients_changed\"]"),
            "{members}"
        );
    }

    /// What std adds to a class of the core is declared on that class in the Python stubs, from the catalog: the stub of std has a stub-only
    /// class of the members of each augment, with each property, getter and method, and the stub of the core extends it, so a checker
    /// finds `gnoblin.workspaces` and `client.style` as TypeScript's merged declarations and Rust's extension traits do.
    #[test]
    fn the_python_stub_of_the_core_extends_what_std_adds_to_each_class() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let namespaces = spec.objects.namespaces();
        let of = |name: &str| {
            namespaces
                .iter()
                .find(|namespace| namespace.as_str() == name)
                .expect("a namespace")
        };
        let std_view = ScriptView::new(&spec.objects, of("gnoblin.std"));
        let core_view = ScriptView::new(&spec.objects, of("gnoblin.model"));
        let std = object_py::render(&std_view, true).expect("the std stub renders");
        let core = object_py::render(&core_view, true).expect("the core stub renders");
        let augments = crate::objects::augments_of(&spec.objects, &std_view.objects).expect("the augments are found");
        assert!(augments.len() >= 5, "std augments several classes of the core");
        assert!(core.contains("from gnoblin.objects import std as _ns_std"), "{core}");
        for (class, target) in augments {
            let members = format!("{}StdMembers", object_py::class_name(target));
            let start = std
                .find(&format!("\nclass {members}(StubMembers):"))
                .unwrap_or_else(|| panic!("std declares `{members}`:\n{std}"));
            let body = &std[start + 1..];
            let body = &body[..body.find("\nclass ").map_or(body.len(), |end| end + 1)];
            for property in &class.properties {
                let ident = object_py::member_name(&property.name, &property.extras.export.py);
                assert!(
                    body.contains(&format!("def {ident}(self)")),
                    "`{members}` has `{ident}`:\n{body}"
                );
            }
            for method in &class.methods {
                if crate::objects::is_augment_setter(class, &method.name) {
                    continue;
                }
                let ident = object_py::member_name(&method.name, &method.extras.export.py);
                assert!(
                    body.contains(&format!("def {ident}(self")),
                    "`{members}` has `{ident}`:\n{body}"
                );
            }
            let extended = format!("_ns_std.{members}");
            let class_line = core
                .lines()
                .find(|line| {
                    line.starts_with(&format!("class {}", object_py::class_name(target))) && line.contains(&extended)
                })
                .unwrap_or_else(|| panic!("the core's `{}` extends `{extended}`", target.name));
            assert!(class_line.ends_with("):"), "{class_line}");
        }
        // A member the augment reads is the object, as in the other two languages, and a receiver's writable getter has its setter.
        assert!(
            std.contains("def scene(self) -> Scene") && !std.contains("def scene(self) -> Scene | None"),
            "{std}"
        );
        assert!(std.contains("@floating.setter"), "{std}");
    }

    /// A catalog with a handle `thing` whose constructor names `id`, and `rest` more properties and types.
    fn thing_catalog(dir: &Path, rest: &str) -> Catalog {
        fs::write(
            dir.join("thing.toml"),
            format!(
                "namespace = \"lib.a\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\nctor = [\"id\"]\n\n\
                 [[object.property]]\nname = \"id\"\ntype = \"string\"\nsince = \"0.1.0\"\nreadonly = true\n\n\
                 [[object.property]]\nname = \"size\"\ntype = \"int\"\nsince = \"0.1.0\"\n\n\
                 [[object.property]]\nname = \"age\"\ntype = \"int\"\nsince = \"0.1.0\"\nreadonly = true\n\n{rest}"
            ),
        )
        .expect("a schema");
        Catalog::load_dirs(&[dir], &[]).expect("a catalog")
    }

    /// The options of a constructor are every writable property the positional ones leave, in each language: the positional property stays
    /// first and is not an option, and a read-only property is not either. A catalog that already has a type named `ThingOptions` makes the
    /// options type `ThingCreateOptions`, so the two never meet; a service has no options.
    #[test]
    fn the_options_of_a_constructor_are_the_writable_properties_the_positional_ones_leave() {
        let root = tempfile::tempdir().expect("a directory");
        let catalog = thing_catalog(root.path(), "");
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);

        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(dts.contains("export interface ThingOptions {"), "{dts}");
        assert!(
            dts.contains("constructor(id: string, options?: ThingOptions);"),
            "{dts}"
        );
        assert!(
            dts.contains("size?: number;") && !dts.contains("age?:") && !dts.contains("  id?:"),
            "{dts}"
        );
        let js = object_js::render_js(&view).expect("the JavaScript renders");
        assert!(js.contains("constructor(id, options = {}) {"), "{js}");
        assert!(
            js.contains("if (options.size !== undefined) inits.push("),
            "an option is sent only when it is given: {js}"
        );
        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(
            stub.contains("def __init__(self, id: str, *, size: int | None = None) -> None"),
            "{stub}"
        );
        let rust = crate::object_api::render(&catalog, &namespaces[0]).expect("the typed API renders");
        assert!(rust.contains("pub struct ThingOptions {"), "{rust}");
        assert!(
            rust.contains("pub fn new_with(objects: &Objects, id: &str, options: ThingOptions)"),
            "{rust}"
        );
        assert!(
            rust.contains("pub fn new(objects: &Objects, id: &str) -> go::Res<Thing>"),
            "the constructor without options is unchanged: {rust}"
        );

        // A record that already has the name: the options type takes the other.
        let root = tempfile::tempdir().expect("a directory");
        let rest = "[[object]]\nname = \"thing_options\"\nsince = \"0.1.0\"\nkind = \"record\"\n\n\
                    [[object.property]]\nname = \"size\"\ntype = \"int\"\nsince = \"0.1.0\"\ndefault = 0\n";
        let catalog = thing_catalog(root.path(), rest);
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);
        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(dts.contains("export interface ThingCreateOptions {"), "{dts}");
        assert!(dts.contains("options?: ThingCreateOptions"), "{dts}");
        assert!(
            dts.contains("export class ThingOptions {"),
            "the record keeps its name: {dts}"
        );
        let rust = crate::object_api::render(&catalog, &namespaces[0]).expect("the typed API renders");
        assert!(rust.contains("pub struct ThingCreateOptions {"), "{rust}");
        assert!(
            rust.contains("pub struct ThingOptions {"),
            "the record keeps its name: {rust}"
        );
    }

    /// A constructor parameter called `options` would be the options record's name, so it is refused rather than shadowed.
    #[test]
    fn a_constructor_parameter_called_options_is_refused() {
        let root = tempfile::tempdir().expect("a directory");
        fs::write(
            root.path().join("thing.toml"),
            "namespace = \"lib.a\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\nctor = [\"options\"]\n\n\
             [[object.property]]\nname = \"options\"\ntype = \"string\"\nsince = \"0.1.0\"\n\n\
             [[object.property]]\nname = \"size\"\ntype = \"int\"\nsince = \"0.1.0\"\n",
        )
        .expect("a schema");
        let catalog = Catalog::load_dirs(&[root.path()], &[]).expect("a catalog");
        let namespaces = catalog.namespaces();
        let view = ScriptView::new(&catalog, &namespaces[0]);
        let refused = object_js::render_js(&view).expect_err("the JavaScript constructor is refused");
        assert!(refused.contains("called `options`"), "{refused}");
        let refused = crate::object_api::render(&catalog, &namespaces[0]).expect_err("the Rust constructor is refused");
        assert!(refused.contains("called `options`"), "{refused}");
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
        fs::copy(
            workspace_root().join("crates/gnoblin-sdkgen/tests/python/fake_gnoblin_errors.py"),
            dir.path().join("gnoblin/errors.py"),
        )
        .expect("the errors stand-in");
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
    /// The catalog of the core with a `gnoblin.std` service `windows` of the schema text `members`, which names the core's classes.
    fn catalog_with_std(dir: &Path, members: &str) -> Catalog {
        fs::write(
            dir.join("windows.toml"),
            format!(
                "namespace = \"gnoblin.std\"\nversion = \"0.2.0\"\nlocal = true\n\n\
                 [[object]]\nname = \"windows\"\nsince = \"0.2.0\"\nkind = \"service\"\n\n{members}"
            ),
        )
        .expect("a schema");
        let core = workspace_root().join("crates/gnoblin-core-abi/schema");
        Catalog::load_dirs(&[core.as_path(), dir], &[]).expect("the catalogs load together")
    }

    fn std_namespace(catalog: &Catalog) -> Namespace {
        catalog
            .namespaces()
            .into_iter()
            .find(|namespace| namespace.as_str() == "gnoblin.std")
            .expect("the std namespace")
    }

    const INSET_OF: &str = "[[object.method]]\nname = \"inset_of\"\nsince = \"0.2.0\"\nreturns = \"double\"\n\n\
         [[object.method.param]]\nname = \"client\"\ntype = \"object:gnoblin.model.client\"\n\n\
         [[object.method]]\nname = \"mark\"\nsince = \"0.2.0\"\n\n\
         [[object.method.param]]\nname = \"client\"\ntype = \"object:gnoblin.model.client\"\noptional = true\n";

    /// A parameter that names the core's `client` is the core's class in every language: the module imports it, refuses a value that is not
    /// an instance of it, and sends the handle as it is. Rust takes a reference to the core SDK's class.
    #[test]
    fn a_parameter_may_name_the_cores_client_in_every_language() {
        let dir = tempfile::tempdir().expect("a directory");
        let catalog = catalog_with_std(dir.path(), INSET_OF);
        let std = std_namespace(&catalog);
        let view = ScriptView::new(&catalog, &std);

        let js = object_js::render_js(&view).expect("the JavaScript renders");
        assert!(
            js.contains(r#"import { Client as ModelClient } from "gnoblin-core/generated/objects";"#),
            "{js}"
        );
        assert!(
            js.contains(r#"B.refTo(client, ModelClient, "a Client")"#),
            "the handle is checked against the core's class before it goes in: {js}"
        );
        assert!(
            js.contains(r#"B.opt(client, (value) => B.refTo(value, ModelClient, "a Client"))"#),
            "an optional one passes none through: {js}"
        );
        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(
            dts.contains(r#"import type { Client as ModelClient } from "gnoblin-core/generated/objects";"#),
            "{dts}"
        );
        assert!(dts.contains("insetOf(client: ModelClient): number;"), "{dts}");
        assert!(dts.contains("mark(client?: ModelClient): void;"), "{dts}");

        let py = object_py::render(&view, false).expect("the Python renders");
        assert!(
            py.contains("from gnoblin.objects.model import Client as _ModelClient"),
            "{py}"
        );
        assert!(py.contains(r#"_B.ref_to(client, _ModelClient, "a Client")"#), "{py}");
        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(
            stub.contains("from gnoblin.objects.model import Client as _ModelClient"),
            "{stub}"
        );
        assert!(
            stub.contains("def inset_of(self, client: _ModelClient) -> float"),
            "{stub}"
        );
        assert!(stub.contains("client: _ModelClient | None = None"), "{stub}");

        let rust = crate::object_api::render(&catalog, &std).expect("the typed API renders");
        assert!(
            rust.contains("pub fn inset_of(&self, client: &::gnoblin_core::objects::Client) -> go::Res<f64>"),
            "{rust}"
        );
        assert!(
            rust.contains("client: Option<&::gnoblin_core::objects::Client>"),
            "{rust}"
        );
        // The catalog table says the parameter is an object of the core's class, so the library that serves the method does not read it as
        // one of its own.
        let tables = crate::objects::render(&catalog, &std).expect("the tables render");
        assert!(tables.contains(r#"class: "gnoblin.model.client""#), "{tables}");
    }

    /// A result, a property and a signal field may name the core's class too: the library hands such an object out as an object of the core
    /// (`VALUE_FOREIGN_OBJECT`), and each language reads it in the core's namespace, as an instance of the core's class.
    #[test]
    fn a_result_a_property_and_a_signal_of_the_cores_class_are_the_cores_class() {
        let members = "[[object.method]]\nname = \"current\"\nsince = \"0.2.0\"\nreturns = \"object:gnoblin.model.client\"\n\n\
             [[object.property]]\nname = \"focused\"\ntype = \"object:gnoblin.model.client\"\nsince = \"0.2.0\"\n\n\
             [[object.property]]\nname = \"children\"\ntype = \"list<object:gnoblin.model.client>\"\nsince = \"0.2.0\"\n\n\
             [[object.signal]]\nname = \"changed\"\nsince = \"0.2.0\"\n\n\
             [[object.signal.field]]\nname = \"client\"\ntype = \"object:gnoblin.model.client\"\n";
        let dir = tempfile::tempdir().expect("a directory");
        let catalog = catalog_with_std(dir.path(), members);
        let std = std_namespace(&catalog);
        let view = ScriptView::new(&catalog, &std);

        let js = object_js::render_js(&view).expect("the JavaScript renders");
        assert!(
            js.contains(r#"B.wrap("gnoblin.model", "#) && js.contains(", ModelClient)"),
            "an object of the core is read in the core's namespace, as the core's class: {js}"
        );
        assert!(
            js.contains(r#"B.opt(value, (slot) => B.wrap("gnoblin.model", slot, ModelClient))"#),
            "a property may be none: {js}"
        );
        let dts = object_js::render_dts(&view).expect("declarations render");
        assert!(dts.contains("current(): ModelClient;"), "{dts}");
        assert!(dts.contains("ModelClient[]"), "a list of them: {dts}");

        let py = object_py::render(&view, false).expect("the Python renders");
        assert!(py.contains(r#"_B.wrap("gnoblin.model", "#), "{py}");
        assert!(py.contains(", _ModelClient)"), "{py}");
        let stub = object_py::render(&view, true).expect("the stub renders");
        assert!(stub.contains("def current(self) -> _ModelClient"), "{stub}");
        assert!(stub.contains("list[_ModelClient]"), "{stub}");

        let rust = crate::object_api::render(&catalog, &std).expect("the typed API renders");
        assert!(
            rust.contains("-> go::Res<::gnoblin_core::objects::Client>"),
            "a result is the core SDK's class: {rust}"
        );
        assert!(rust.contains("Vec<::gnoblin_core::objects::Client>"), "{rust}");
        assert!(
            rust.contains("impl go::typed::FromVal<Objects> for ::gnoblin_core::objects::Client")
                && rust.contains("::gnoblin_core::objects::for_generation(cx.api.ctx())")
                && rust.contains(r#"go::typed::foreign_in(value, "gnoblin.model")"#),
            "the core's class is read through the core's typed API for the value's generation: {rust}"
        );
    }

    /// A class of the other library that is not a handle cannot be a parameter: a record is a value the library would have to read field by
    /// field, and a service has no instance.
    #[test]
    fn a_record_or_a_service_of_the_core_is_not_a_parameter() {
        for (kind, class) in [("record", "gnoblin.model.rect"), ("service", "gnoblin.model.gnoblin")] {
            let dir = tempfile::tempdir().expect("a directory");
            let members = format!(
                "[[object.method]]\nname = \"measure\"\nsince = \"0.2.0\"\n\n[[object.method.param]]\nname = \"what\"\ntype = \"object:{class}\"\n"
            );
            let catalog = catalog_with_std(dir.path(), &members);
            let std = std_namespace(&catalog);
            let view = ScriptView::new(&catalog, &std);
            let refused = object_js::render_js(&view).expect_err(&format!("a {kind} is not a parameter"));
            assert!(
                refused.contains(kind) && refused.contains("only a handle"),
                "{kind}: {refused}"
            );
        }
    }

    /// `B.refTo` of the JavaScript runtime: an instance of the class goes in as its handle, and anything else is refused by name, so a
    /// handle of the std library (which numbers its objects as the core does) is never taken for a window. Skipped where there is no `node`.
    #[test]
    fn the_javascript_ref_to_takes_the_class_and_refuses_everything_else() {
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
// Each adoption is a distinct object to the base: the check adopts one slot number twice as two classes.
globalThis.__gnoblinObjects = { key: (() => { let n = 0; return () => String(++n); })(), release: () => {}, guard: () => ({}) };
const B = await import("./object_base.mjs");
class Client {}
class Other {}
const made = B.adopt(Object.create(Client.prototype), 7);
const refuses = (what, run, words) => {
  try { run(); } catch (error) {
    if (!(error instanceof TypeError) || !words.every((word) => error.message.includes(word))) throw new Error(`${what}: the error was ${error}`);
    return;
  }
  throw new Error(`${what}: nothing was refused`);
};
const sent = B.refTo(made, Client, "a Client");
if (sent.$t !== 3 || sent.$v !== 7) throw new Error(`the handle did not go in as it is: ${JSON.stringify(sent)}`);
refuses("another class", () => B.refTo(B.adopt(Object.create(Other.prototype), 7), Client, "a Client"), ["expected a Client", "Other"]);
refuses("a string id", () => B.refTo("12", Client, "a Client"), ["expected a Client", "string"]);
refuses("none", () => B.refTo(null, Client, "a Client"), ["expected a Client", "null"]);
refuses("a plain object", () => B.refTo({}, Client, "a Client"), ["expected a Client", "Object"]);
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

    /// `_B.ref_to` of the Python runtime, as `B.refTo` is. Skipped where there is no `python3`.
    #[test]
    fn the_python_ref_to_takes_the_class_and_refuses_everything_else() {
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
        fs::copy(
            workspace_root().join("crates/gnoblin-sdkgen/tests/python/fake_gnoblin_errors.py"),
            dir.path().join("gnoblin/errors.py"),
        )
        .expect("the errors stand-in");
        fs::write(package.join("__init__.py"), "").expect("the package");
        fs::write(package.join("_base.py"), &base.1).expect("the base");
        fs::write(dir.path().join("_gnoblin_objects.py"), "").expect("the native stand-in");
        fs::write(
            dir.path().join("check.py"),
            r#"
from gnoblin.objects import _base as B


class Client:
    def __init__(self, handle):
        self._h = handle


class Other:
    def __init__(self, handle):
        self._h = handle


def refuses(what, run, *words):
    try:
        run()
    except TypeError as error:
        assert all(word in str(error) for word in words), f"{what}: the error was {error}"
        return
    raise AssertionError(f"{what}: nothing was refused")


assert B.ref_to(Client(7), Client, "a Client") == 7, "the handle goes in as it is"
refuses("another class", lambda: B.ref_to(Other(7), Client, "a Client"), "expected a Client", "Other")
refuses("a string id", lambda: B.ref_to("12", Client, "a Client"), "expected a Client", "str")
refuses("none", lambda: B.ref_to(None, Client, "a Client"), "expected a Client", "NoneType")
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

    /// Each generated class carries the spelling of its members, which `B.raise` and `_B.named` read to word a refusal as the script wrote
    /// the member: the class and each member of its own, with the parameters and the record fields spelled otherwise.
    #[test]
    fn the_generated_classes_carry_the_spelling_of_their_members() {
        let files = rendered_files();
        let text = |path: &str| {
            files
                .iter()
                .find(|(file, _)| file == path)
                .unwrap_or_else(|| panic!("{path} is rendered"))
                .1
                .as_str()
        };
        let js = text("probe/objects.js");
        for wanted in [
            "const spell_probe = [\"probe\", { ",
            "\"last_colour\": [\"lastColour\"]",
            "\"on_tick\": [\"onTick\"]",
            "\"length_of\": [\"lengthOf\"]",
            "B.raise(error, this, \"Probe\", \"lastColour\", spell_probe)",
            "B.raise(error, undefined, \"Probe\", \"new\", spell_probe)",
            "B.raise(error, undefined, \"Probe\", \"clamp\", spell_probe)",
        ] {
            assert!(js.contains(wanted), "the JavaScript lacks `{wanted}`");
        }
        let py = text("probe/probe.py");
        for wanted in [
            "_spell_probe = (\"probe\", {",
            "\"last_colour\": (\"last_colour\", {})",
            "\"reach\": (\"reach\", {\"from\": \"from_\"})",
            "@_B.named(\"Probe\", \"reach\", spell=_spell_probe)",
            "@_B.named(\"Probe\", \"new\", True, spell=_spell_probe)",
        ] {
            assert!(py.contains(wanted), "the Python lacks `{wanted}`");
        }
        assert!(
            !text("probe/probe.pyi").contains("_spell_"),
            "a stub has no spelling table"
        );
    }

    /// `B.raise` of the JavaScript runtime words a refusal as the script wrote the member: the label (`class.member`), the record field or
    /// the parameter after it, and the target a library puts first in its own words. A name the table lacks, another class and an error
    /// that is not a refusal are left as they are. Skipped where there is no `node`.
    #[test]
    fn the_javascript_raise_writes_the_names_as_the_script_spells_them() {
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
const spell = ["keyboard", {
  binding_only: ["bindingOnly"],
  repeat_rate: ["repeatRate"],
  split_ratio: ["splitRatio", { max_size: "maxSize" }],
}];
const eq = (what, got, want) => {
  if (got !== want) throw new Error(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
const bad = (message, make = TypeError, member = "bindingOnly") => B.raise(new make(message), undefined, "Keyboard", member, spell);
eq("a property", bad("`keyboard.binding_only` must be a boolean, got an integer").message, "`Keyboard.bindingOnly` must be a boolean, got an integer");
eq("a parameter", bad("`keyboard.split_ratio`: `max_size` must be a number, got text").message, "`Keyboard.splitRatio`: `maxSize` must be a number, got text");
eq("a field of an element", bad("`keyboard.split_ratio`[2].max_size must be a number").message, "`Keyboard.splitRatio`[2].maxSize must be a number");
eq("a range", bad("`keyboard.repeat_rate` must be above 0", RangeError).message, "`Keyboard.repeatRate` must be above 0");
eq("the target of the library", bad("Keyboard#1.repeat_rate: must be a number, got 0").message, "Keyboard#1.repeatRate: must be a number, got 0");
eq("a member the table lacks", bad("`keyboard.nothing` must be a boolean").message, "`keyboard.nothing` must be a boolean");
eq("another class", bad("`mouse.binding_only` must be a boolean").message, "`mouse.binding_only` must be a boolean");
eq("a member of the prototype", bad("`keyboard.constructor` must be a boolean").message, "`keyboard.constructor` must be a boolean");
eq("words of the script", bad("dwindle.split_ratio.amount must be a finite number").message, "dwindle.split_ratio.amount must be a finite number");
const own = new Error("not ours");
eq("an error that is no refusal", B.raise(own, undefined, "Keyboard", "bindingOnly", spell), own);
const refused = new Error("Keyboard#1.repeat_rate: nothing `keyboard.binding_only` is read-only");
refused.code = "read_only";
refused.abi = -9;
const raised = B.raise(refused, { id: 1 }, "Keyboard", "repeatRate", spell);
eq("the target of a coded failure", raised.target, "Keyboard#1.repeatRate");
eq("the words of a coded failure", raised.message, "Keyboard#1.repeatRate: nothing `Keyboard.bindingOnly` is read-only");
eq("no table", B.raise(new TypeError("`keyboard.binding_only` is bad"), undefined, "Keyboard", "bindingOnly").message, "`keyboard.binding_only` is bad");
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

    /// `_B.named` of the Python runtime, as `B.raise` is: a refusal is worded with the attribute the class gives the member (`from_` for the
    /// member `from`), a library's failure and a bad argument alike. Skipped where there is no `python3`.
    #[test]
    fn the_python_named_writes_the_names_as_the_script_spells_them() {
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
        fs::copy(
            workspace_root().join("crates/gnoblin-sdkgen/tests/python/fake_gnoblin_errors.py"),
            dir.path().join("gnoblin/errors.py"),
        )
        .expect("the errors stand-in");
        fs::write(package.join("__init__.py"), "").expect("the package");
        fs::write(package.join("_base.py"), &base.1).expect("the base");
        fs::write(dir.path().join("_gnoblin_objects.py"), "").expect("the native stand-in");
        fs::write(
            dir.path().join("check.py"),
            r#"
from gnoblin.errors import GnoblinError
from gnoblin.objects import _base as B

spell = ("probe", {"reach": ("reach", {"from": "from_", "max_size": "max_size"}), "last_colour": ("last_colour", {})})


class Probe:
    id = 3

    @B.named("Probe", "reach", spell=spell)
    def bad_argument(self, message):
        raise TypeError(message)

    @B.named("Probe", "reach", spell=spell)
    def refusal(self, message):
        raise GnoblinError(message)

    @B.named("Probe", "reach")
    def untabled(self, message):
        raise TypeError(message)


def words(run, message, kind):
    try:
        run(message)
    except kind as error:
        return str(error), getattr(error, "target", None)
    raise AssertionError(f"{message}: nothing was raised")


probe = Probe()
assert words(probe.bad_argument, "`probe.reach`: `from` must be an object, got text", TypeError) == (
    "`Probe.reach`: `from_` must be an object, got text",
    None,
), "a parameter"
assert words(probe.bad_argument, "`probe.reach`[1].from must be text", TypeError)[0] == "`Probe.reach`[1].from_ must be text", "a field"
assert words(probe.bad_argument, "`probe.nothing` must be text", TypeError)[0] == "`probe.nothing` must be text", "a member the table lacks"
assert words(probe.bad_argument, "`other.reach` must be text", TypeError)[0] == "`other.reach` must be text", "another class"
assert words(probe.untabled, "`probe.reach` must be text", TypeError)[0] == "`probe.reach` must be text", "no table"
assert words(probe.refusal, "Probe#3.reach: nope `probe.last_colour` is read-only", GnoblinError) == (
    "Probe#3.reach: nope `Probe.last_colour` is read-only",
    "Probe#3.reach",
), "a failure of the library"
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

    /// The classes of the catalog `gnoblin.std` serves take the core's `Client` where the TypeScript library took a window: the members
    /// that take a client from the shell say so in every language, and none of them takes the node number as text any more.
    #[test]
    fn the_std_members_that_take_a_window_take_the_cores_client() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let std = spec
            .objects
            .namespaces()
            .into_iter()
            .find(|namespace| namespace.as_str() == "gnoblin.std")
            .expect("the std namespace");
        let view = ScriptView::new(&spec.objects, &std);
        let dts = object_js::render_dts(&view).expect("declarations render");
        let mut missing = Vec::new();
        for member in [
            "insetOf(client: ModelClient): FrameInset;",
            "reservedOf(client: ModelClient): FrameInset;",
            "frameOf(client: ModelClient)",
            "reframe(client: ModelClient): void;",
            "show(client: ModelClient, x?: number, y?: number)",
            "isAbove(client: ModelClient): boolean;",
            "setAbove(client: ModelClient, on: boolean): void;",
            "maximise(client: ModelClient): boolean;",
            "beginMove(client: ModelClient): boolean;",
            "contains(client: ModelClient): boolean;",
            "add(client: ModelClient): void;",
            "remove(client: ModelClient): boolean;",
            "ofClient(client: ModelClient)",
            "restoreClient(workspace: Workspace, client: ModelClient, arrived: boolean)",
            "showClient(client: ModelClient)",
            "isOpen(client: ModelClient): boolean;",
        ] {
            if !dts.contains(member) {
                missing.push(format!("declarations: {member}"));
            }
        }
        let py = object_py::render(&view, true).expect("the stub renders");
        for member in [
            "def inset_of(self, client: _ModelClient)",
            "def show(self, client: _ModelClient, x: float | None = None, y: float | None = None)",
            "def add(self, client: _ModelClient) -> None",
            "def of_client(self, client: _ModelClient)",
        ] {
            if !py.contains(member) {
                missing.push(format!("stub: {member}"));
            }
        }
        let rust = crate::object_api::render(&spec.objects, &std).expect("the typed API renders");
        for member in [
            "pub fn inset_of(&self, client: &::gnoblin_core::objects::Client)",
            "pub fn add(&self, client: &::gnoblin_core::objects::Client)",
            "pub fn of_client(&self, client: &::gnoblin_core::objects::Client)",
        ] {
            if !rust.contains(member) {
                missing.push(format!("typed API: {member}"));
            }
        }
        assert!(missing.is_empty(), "members missing:\n{}", missing.join("\n"));
        // The members a library calls on the shell (an interface method, a signal) hand the client over as the core's class too: the library
        // sends the core's object, and the shell's runtime reads it in the core's namespace.
        assert!(
            dts.lines()
                .any(|line| line.contains("clientAdded(") && line.contains("client: ModelClient")),
            "the layout interface hears the client as the core's `Client`"
        );
    }
}

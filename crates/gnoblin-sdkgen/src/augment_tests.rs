//! What the std catalog's `[[augment]]` becomes in every language, and what the generator refuses.
//!
//! The std library adds `workspaces`, `displays`, `scene`, `gestures` and `requestTransition` to the core's `gnoblin` object, `profiles`
//! to its output collections, and, as `receiver = true` augments that act on the object they are used on, `activeDisplay` to its input
//! contexts and `style` to its client collections. The core catalog never reads the std files, so a core-only shell has none of them: the
//! failure modes are that the core's files render the same bytes with and without std, that the members are installed by the std module
//! of each language and by nothing of the core's, that a receiver member is handed its object, that a missing target is refused with its
//! name, and that a name the core's class already has is refused with both names.

use std::fs;
use std::path::Path;

use gnoblin_schema::{CORE_NAMESPACE, Catalog};

use crate::{SdkSpec, files_of, render, surface, workspace_root};

fn spec() -> SdkSpec {
    SdkSpec::load(&workspace_root()).expect("the specification loads")
}

/// The text of one generated file.
fn generated(target: &str, path: &str) -> String {
    let spec = spec();
    let rendered = render(&spec, Some(target)).expect("the target renders");
    files_of(&rendered)
        .into_iter()
        .find(|(file, _)| file == path)
        .unwrap_or_else(|| panic!("{path} is not rendered"))
        .1
}

/// The members of the std library's augments, by the class that holds them. A receiver augment's members are all methods, which take the
/// object they act on first.
const MEMBERS: [(&str, &[&str], &[&str]); 6] = [
    (
        "gnoblin_augment",
        &["workspaces", "displays", "scene", "gestures", "extensions"],
        &["find_workspaces", "request_transition", "show"],
    ),
    ("output_collection_augment", &["profiles"], &[]),
    ("input_context_augment", &[], &["active_display"]),
    ("client_collection_augment", &[], &["style"]),
    (
        "client_augment",
        &[],
        &[
            "computed_style",
            "floating",
            "floating_bounds",
            "frame_inset",
            "move_to_output",
            "move_to_workspace",
            "move_with_keys",
            "output",
            "resize_with_keys",
            "set_floating",
            "set_floating_bounds",
            "style",
            "style_warnings",
            "workspace",
        ],
    ),
    ("output_augment", &[], &["area", "show", "workspace"]),
];

#[test]
fn the_std_catalog_holds_the_members_the_root_gains() {
    let spec = spec();
    let std = spec
        .objects
        .namespaces()
        .into_iter()
        .find(|n| n.as_str() == "gnoblin.std")
        .expect("std");
    for (class, properties, methods) in MEMBERS {
        let found = spec
            .objects
            .objects(&std)
            .find(|object| object.name == class)
            .unwrap_or_else(|| panic!("the std catalog has no class {class}"));
        let mut have: Vec<&str> = found.properties.iter().map(|property| property.name.as_str()).collect();
        have.sort_unstable();
        let mut want = properties.to_vec();
        want.sort_unstable();
        assert_eq!(have, want, "{class} properties");
        let mut have: Vec<&str> = found.methods.iter().map(|method| method.name.as_str()).collect();
        have.sort_unstable();
        assert_eq!(have, methods, "{class} methods");
    }
}

/// The core's catalog renders the same bytes whether or not the std files are read: nothing of std is in it.
#[test]
fn the_core_catalog_renders_the_same_without_the_std_files() {
    let spec = spec();
    let core_dir = workspace_root().join(crate::model::MODEL_DIRS[0]);
    let alone = Catalog::load_dirs(&[core_dir.as_path()], &[]).expect("the core catalog loads alone");
    let namespace = |catalog: &Catalog| {
        catalog
            .namespaces()
            .into_iter()
            .find(|namespace| namespace.as_str() == CORE_NAMESPACE)
            .expect("the core namespace")
    };
    assert_eq!(
        spec.objects.namespace_fingerprint(&namespace(&spec.objects)),
        alone.namespace_fingerprint(&namespace(&alone)),
        "the core fingerprint"
    );
    assert_eq!(
        crate::objects::render(&spec.objects, &namespace(&spec.objects)).expect("with std"),
        crate::objects::render(&alone, &namespace(&alone)).expect("alone"),
        "the core's static catalog"
    );
    assert_eq!(
        crate::object_api::render(&spec.objects, &namespace(&spec.objects)).expect("with std"),
        crate::object_api::render(&alone, &namespace(&alone)).expect("alone"),
        "the core's typed Rust API"
    );
}

#[test]
fn javascript_installs_the_members_from_the_std_module_and_the_core_module_names_none() {
    let std = generated("object-js", "js/native/gnoblin-std/generated/objects.js");
    for line in [
        "import * as Core from \"gnoblin-core/generated/objects\";",
        "B.augmentGet(Core.Gnoblin, \"workspaces\", () => GnoblinAugment.get().workspaces);",
        "B.augmentGet(Core.Gnoblin, \"displays\", () => GnoblinAugment.get().displays);",
        "B.augmentGet(Core.Gnoblin, \"scene\", () => GnoblinAugment.get().scene);",
        "B.augmentGet(Core.Gnoblin, \"gestures\", () => GnoblinAugment.get().gestures);",
        "B.augmentCall(Core.Gnoblin, \"requestTransition\", (_self, ...args) => GnoblinAugment.get().requestTransition(...args));",
        "B.augmentGet(Core.OutputCollection, \"profiles\", () => OutputCollectionAugment.get().profiles);",
        "B.augmentGet(Core.InputContext, \"activeDisplay\", (self) => InputContextAugment.get().activeDisplay(self));",
        "B.augmentGet(Core.ClientCollection, \"style\", (self) => ClientCollectionAugment.get().style(self));",
    ] {
        assert!(std.contains(line), "the std module lacks `{line}`");
    }
    let core = generated("object-js", "js/native/gnoblin-core/generated/objects.js");
    for word in ["Augment", "augment", "workspaces", "requestTransition", "gnoblin-std"] {
        assert!(!core.contains(word), "the core module names `{word}`");
    }
    let declarations = generated("object-types", "js/gnoblin-std/generated/objects.d.ts");
    assert!(declarations.contains("declare module \"gnoblin-core/generated/objects\" {"));
    assert!(declarations.contains("interface Gnoblin {"));
    assert!(declarations.contains("readonly workspaces: Workspaces;"));
    assert!(declarations.contains("interface OutputCollection {"));
    assert!(declarations.contains("readonly profiles: OutputProfiles;"));
    assert!(declarations.contains("interface InputContext {"));
    assert!(declarations.contains("readonly activeDisplay: Display | undefined;"));
    assert!(declarations.contains("interface ClientCollection {"));
    assert!(declarations.contains("readonly style: ClientStyle;"));
    let core = generated("object-types", "js/gnoblin-core/generated/objects.d.ts");
    assert!(
        !core.contains("workspaces"),
        "the core's declarations name a std member"
    );
}

#[test]
fn python_installs_the_members_from_the_std_module_and_the_core_module_names_none() {
    let std = generated("object-py", "python/gnoblin/objects/std.py");
    for line in [
        "from gnoblin.objects import model as _core",
        "_B.augment_property(_core.Gnoblin, \"workspaces\", lambda _obj: GnoblinAugment.get().workspaces)",
        "_B.augment_property(_core.Gnoblin, \"displays\", lambda _obj: GnoblinAugment.get().displays)",
        "_B.augment_property(_core.Gnoblin, \"scene\", lambda _obj: GnoblinAugment.get().scene)",
        "_B.augment_property(_core.Gnoblin, \"gestures\", lambda _obj: GnoblinAugment.get().gestures)",
        "_B.augment_method(_core.Gnoblin, \"request_transition\", lambda _obj, *args, **kwargs: GnoblinAugment.get().request_transition(*args, **kwargs))",
        "_B.augment_property(_core.OutputCollection, \"profiles\", lambda _obj: OutputCollectionAugment.get().profiles)",
        "_B.augment_property(_core.InputContext, \"active_display\", lambda obj: InputContextAugment.get().active_display(obj))",
        "_B.augment_property(_core.ClientCollection, \"style\", lambda obj: ClientCollectionAugment.get().style(obj))",
    ] {
        assert!(std.contains(line), "the std module lacks `{line}`");
    }
    let core = generated("object-py", "python/gnoblin/objects/model.py");
    for word in ["Augment", "augment", "workspaces", "request_transition"] {
        assert!(!core.contains(word), "the core module names `{word}`");
    }
}

#[test]
fn rust_gives_the_core_root_an_extension_trait_in_the_std_api_only() {
    let std = generated("object-api", "rust-objects/std_api.rs");
    assert!(
        std.contains("pub trait GnoblinStdExt {"),
        "the extension trait of the root"
    );
    assert!(std.contains("impl GnoblinStdExt for ::gnoblin_core::objects::Gnoblin {"));
    assert!(std.contains("pub trait OutputCollectionStdExt {"));
    assert!(std.contains("impl OutputCollectionStdExt for ::gnoblin_core::objects::OutputCollection {"));
    assert!(std.contains("fn workspaces(&self) -> crate::Res<Workspaces>;"));
    assert!(std.contains("fn request_transition(&self,"));
    assert!(std.contains("impl ClientCollectionStdExt for ::gnoblin_core::objects::ClientCollection {"));
    assert!(std.contains("fn style(&self) -> crate::Res<ClientStyle>;"));
    assert!(std.contains("ClientCollectionAugment::get(&super::std_objects(self.handle().api())?)?.style(self)"));
    let core = generated("object-api", "rust-objects/model_api.rs");
    assert!(!core.contains("StdExt"), "the core's API names a std trait");
    assert!(!core.contains("gnoblin_std"), "the core's API names the std crate");
}

/// The surface lock records the augment, with the members keyed by the core class they extend, and not the class the catalog lowered it to.
#[test]
fn the_surface_lock_records_the_augment_and_not_the_lowered_class() {
    let spec = spec();
    let units = surface::module_units(&spec);
    let entries: Vec<(&str, &str)> = units
        .iter()
        .flat_map(|unit| unit.entries.iter())
        .map(|entry| (entry.kind, entry.key.as_str()))
        .collect();
    for key in [
        "core.gnoblin",
        "core.gnoblin.workspaces",
        "core.gnoblin.displays",
        "core.gnoblin.scene",
        "core.gnoblin.gestures",
        "core.gnoblin.request_transition",
        "core.output_collection.profiles",
        "core.input_context.active_display",
        "core.client_collection.style",
    ] {
        assert!(
            entries.iter().any(|(_, found)| *found == key),
            "the lock has no entry for {key}: {entries:?}"
        );
    }
    assert!(
        !entries
            .iter()
            .any(|(_, key)| key.ends_with("_augment") || key.contains("_augment.")),
        "the lowered class is in a lock"
    );
}

/// A copy of the three catalogs the specification reads, in a directory of their own, for a test that changes one.
fn copy_catalogs(to: &Path) {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("a directory");
        for entry in fs::read_dir(from).expect("a catalog directory") {
            let entry = entry.expect("an entry");
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("a file");
            }
        }
    }
    let root = workspace_root();
    for dir in [
        crate::model::SCHEMA_DIR,
        crate::model::MODEL_DIRS[0],
        crate::model::MODEL_DIRS[1],
    ] {
        copy(&root.join(dir), &to.join(dir));
    }
}

/// The text of a std file with one more augment, which the specification is loaded with.
fn load_with(augment: &str) -> String {
    let root = tempfile::tempdir().expect("a directory");
    copy_catalogs(root.path());
    fs::write(
        root.path().join(crate::model::MODEL_DIRS[1]).join("zz_extra.toml"),
        format!("namespace = \"gnoblin.std\"\nversion = \"0.2.0\"\nlocal = true\n\n{augment}"),
    )
    .expect("the extra file");
    match SdkSpec::load(root.path()) {
        Ok(_) => String::new(),
        Err(error) => error,
    }
}

/// Failure mode: a target the core catalog lacks is refused, and the message names it.
#[test]
fn sdkgen_refuses_an_augment_whose_target_the_core_lacks() {
    let error = load_with(
        "[[augment]]\ntarget = \"core.no_such_class\"\nsince = \"0.2.0\"\n\n[[augment.property]]\nname = \"extra\"\ntype = \"string\"\nsince = \"0.2.0\"\nreadonly = true\n",
    );
    assert!(
        error.contains("core.no_such_class"),
        "the error names the target: {error:?}"
    );
}

/// Failure mode: a member the core's class has is refused, and the message names the member, the augment and the core class.
#[test]
fn sdkgen_refuses_an_augment_member_the_core_class_already_has() {
    for (name, kind) in [
        ("clients", "property"),
        ("find_clients", "method"),
        ("client_created", "signal"),
    ] {
        let augment = if kind == "method" {
            format!(
                "[[augment]]\ntarget = \"core.gnoblin\"\nsince = \"0.2.0\"\n\n[[augment.method]]\nname = \"{name}\"\nsince = \"0.2.0\"\n"
            )
        } else {
            format!(
                "[[augment]]\ntarget = \"core.gnoblin\"\nsince = \"0.2.0\"\n\n[[augment.property]]\nname = \"{name}\"\ntype = \"string\"\nsince = \"0.2.0\"\nreadonly = true\n"
            )
        };
        let error = load_with(&augment);
        assert!(
            error.contains(name)
                && error.contains("core.gnoblin")
                && error.contains(kind)
                && error.contains("client.toml"),
            "{kind} `{name}`: {error:?}"
        );
    }
}

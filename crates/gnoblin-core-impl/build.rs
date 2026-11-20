//! Link flags of the core library, and the schema catalog as static data.
//!
//! The library's identity is set here: the SONAME `libgnoblin-core.so.1` (Cargo derives the file name `libgnoblin_core_impl.so`
//! from the crate; the SONAME is a link argument and dist installs the file under the SONAME), `-z nodelete` (a core that is
//! never unloaded is what makes retired shell generations safe), and `-z defs` (an unresolved symbol fails the link, not
//! the first call). The export list is not a version script: rustc already restricts a cdylib to its `#[no_mangle]`
//! symbols, and a second script is rejected by the linker (spike result 1 in `design/modular-framework-spike.md`).
//!
//! The catalog the `catalog_*` functions answer from is generated from `gnoblin-schema`'s files: every class with its
//! properties, methods and signals, as `static` slices, plus a hash that `gnoblin_catalog_hash` returns.
//!
//! Last, the build tree is staged: `target/<profile>/libgnoblin-core.so.1` is made a link to the library Cargo uplifts as
//! `libgnoblin_core_impl.so`. The `gnoblin` binary records the SONAME and finds it through its `$ORIGIN` run path, so a
//! workspace build (and `cargo test --workspace`) leaves a tree whose binary runs. `cargo xtask stage` does the same
//! after building only this crate, checks the library, and is the supported way to stage for `cargo build -p gnoblin`.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_core_abi as abi;
use gnoblin_schema::{Catalog, DefaultValue, Namespace, Type};

fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,{}", abi::SONAME);
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,defs");

    stage_soname_link();
    // The catalog of `gnoblin.model` the object protocol serves, written into `OUT_DIR` (nothing of it is committed).
    gnoblin_sdkgen::emit_for_build(&["objects"]);
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let schema_dir = manifest.join("../gnoblin-schema/schema");
    println!("cargo:rerun-if-changed={}", schema_dir.display());
    println!("cargo:rerun-if-changed=build.rs");
    let catalog = Catalog::load_dir(&schema_dir)
        .unwrap_or_else(|error| panic!("the schema catalog at {} does not load: {error}", schema_dir.display()));

    let (data, hash) = render(&catalog);
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("catalog_data.rs");
    fs::write(
        &out,
        format!("{data}\npub(crate) const CATALOG_HASH: u64 = {hash:#018x};\n"),
    )
    .unwrap_or_else(|error| panic!("cannot write {}: {error}", out.display()));
}

/// Links `libgnoblin-core.so.1` to `libgnoblin_core_impl.so` in the profile directory this build writes to.
///
/// The link may dangle until Cargo has linked the library. This is a convenience: a layout other than
/// `<target>/<profile>/build/<package>/out` is left alone, and a failure is a warning, never a failed build.
fn stage_soname_link() {
    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    // `<profile dir>/build/<package>-<hash>/out`: the profile directory is where Cargo uplifts the library and writes the binaries.
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    if out_dir.ancestors().nth(2).and_then(Path::file_name) != Some("build".as_ref()) {
        return;
    }
    let link = profile_dir.join(abi::SONAME);
    let temporary = profile_dir.join(format!("{}.stage", abi::SONAME));
    let _ = fs::remove_file(&temporary);
    let linked =
        std::os::unix::fs::symlink("libgnoblin_core_impl.so", &temporary).and_then(|()| fs::rename(&temporary, &link));
    if let Err(error) = linked {
        println!(
            "cargo:warning=cannot stage {}: {error}; run `cargo xtask stage`",
            link.display()
        );
    }
}

/// The static catalog source and the catalog's fingerprint (`Catalog::fingerprint`, which the generated SDKs record too).
fn render(catalog: &Catalog) -> (String, u64) {
    let mut source = String::from("pub(crate) static CLASSES: &[ClassDef] = &[\n");
    // The tree namespace first, so a bare class name resolves there (`Catalog::namespaces` is sorted by name).
    let mut namespaces = catalog.namespaces();
    namespaces.sort_by_key(|namespace| (namespace.as_str() != "tree", namespace.as_str().to_owned()));
    for namespace in &namespaces {
        for object in catalog.objects(namespace) {
            let _ = writeln!(
                source,
                "    ClassDef {{ namespace: {:?}, name: {:?}, properties: &[",
                namespace.as_str(),
                object.name
            );
            for property in &object.properties {
                let kind = value_kind(catalog, namespace, &property.ty, property.secret);
                let flags = flag(property.animated, abi::MEMBER_ANIMATED)
                    | flag(property.readonly, abi::MEMBER_READONLY)
                    | flag(property.secret, abi::MEMBER_SECRET);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: {kind}, flags: {flags}, args: &[], arg_names: \"\", required: 0, default: {} }},",
                    property.name,
                    default_source(property.default.as_ref(), kind)
                );
            }
            source.push_str("    ], methods: &[\n");
            for method in &object.methods {
                let args: Vec<u32> = method
                    .params
                    .iter()
                    .map(|field| value_kind(catalog, namespace, &field.ty, field.secret))
                    .collect();
                let names: Vec<&str> = method.params.iter().map(|field| field.name.as_str()).collect();
                // The optional parameters are the last of a method's: the arguments a call must give are those before the first.
                let required = method.params.iter().take_while(|field| !field.optional).count();
                let flags = flag(method.is_sensitive(), abi::MEMBER_SECRET);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: 0, flags: {flags}, args: &{args:?}, arg_names: {:?}, required: {required}, default: DefaultDef::None }},",
                    method.name,
                    names.join(" ")
                );
            }
            source.push_str("    ], signals: &[\n");
            for signal in &object.signals {
                let args: Vec<u32> = signal
                    .fields
                    .iter()
                    .map(|field| value_kind(catalog, namespace, &field.ty, field.secret))
                    .collect();
                let names: Vec<&str> = signal.fields.iter().map(|field| field.name.as_str()).collect();
                let flags = flag(signal.is_sensitive(), abi::MEMBER_SECRET);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: 0, flags: {flags}, args: &{args:?}, arg_names: {:?}, required: {}, default: DefaultDef::None }},",
                    signal.name,
                    names.join(" "),
                    args.len()
                );
            }
            source.push_str("    ] },\n");
        }
    }
    source.push_str("];\n");
    (source, catalog.fingerprint())
}

/// The source of a property's default (`DefaultDef` in `catalog.rs`): what the property reads as before any write. A property
/// the schema gives no default reads as the empty value of its kind, which the reader supplies.
fn default_source(default: Option<&DefaultValue>, kind: u32) -> String {
    let Some(default) = default else {
        return "DefaultDef::None".to_owned();
    };
    match (default, kind) {
        (DefaultValue::Bool(value), abi::VALUE_BOOL) => format!("DefaultDef::Bool({value})"),
        (DefaultValue::Int(value), abi::VALUE_INT) => format!("DefaultDef::Int({value})"),
        (DefaultValue::Uint(value), abi::VALUE_UINT) => format!("DefaultDef::Uint({value})"),
        (DefaultValue::Int(value), abi::VALUE_UINT) if *value >= 0 => format!("DefaultDef::Uint({value})"),
        (DefaultValue::Uint(value), abi::VALUE_INT) if i64::try_from(*value).is_ok() => {
            format!("DefaultDef::Int({value})")
        }
        (DefaultValue::Double(value), abi::VALUE_DOUBLE) => format!("DefaultDef::Double({:?}_f64)", value),
        (DefaultValue::Int(value), abi::VALUE_DOUBLE) => format!("DefaultDef::Double({value}.0_f64)"),
        (DefaultValue::Uint(value), abi::VALUE_DOUBLE) => format!("DefaultDef::Double({value}.0_f64)"),
        (DefaultValue::Text(value), abi::VALUE_TEXT) => format!("DefaultDef::Text({value:?})"),
        (DefaultValue::List(items), abi::VALUE_LIST) => {
            let numbers: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    DefaultValue::Double(value) => Some(format!("{value:?}_f64")),
                    DefaultValue::Int(value) => Some(format!("{value}.0_f64")),
                    DefaultValue::Uint(value) => Some(format!("{value}.0_f64")),
                    _ => None,
                })
                .collect();
            format!("DefaultDef::Numbers(&[{}])", numbers.join(", "))
        }
        (DefaultValue::List(items), abi::VALUE_TEXTS) => {
            let texts: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    DefaultValue::Text(value) => Some(format!("{value:?}")),
                    _ => None,
                })
                .collect();
            format!("DefaultDef::Texts(&[{}])", texts.join(", "))
        }
        _ => "DefaultDef::None".to_owned(),
    }
}

fn flag(condition: bool, bit: u32) -> u32 {
    if condition { bit } else { 0 }
}

/// The ABI value kind a schema type travels as.
fn value_kind(catalog: &Catalog, namespace: &Namespace, ty: &Type, secret: bool) -> u32 {
    if secret {
        return abi::VALUE_SECRET;
    }
    match ty {
        Type::Bool => abi::VALUE_BOOL,
        Type::Int => abi::VALUE_INT,
        Type::Uint => abi::VALUE_UINT,
        Type::Double => abi::VALUE_DOUBLE,
        Type::Text => abi::VALUE_TEXT,
        Type::Bytes => abi::VALUE_BYTES,
        Type::ObjectRef(name) => {
            let record = catalog
                .objects(namespace)
                .any(|object| object.name == *name && object.record);
            if record { abi::VALUE_RECORD } else { abi::VALUE_NODE }
        }
        Type::List(inner) => match inner.as_ref() {
            Type::Text => abi::VALUE_TEXTS,
            Type::ObjectRef(name) if name == "effect" => abi::VALUE_EFFECTS,
            _ => abi::VALUE_LIST,
        },
        // The compositor's catalog never holds them: they belong to the namespaces a library serves.
        Type::Enum(_) => abi::VALUE_ENUM,
        Type::Callback(_) => abi::VALUE_CLOSURE,
    }
}

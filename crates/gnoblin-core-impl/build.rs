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
use gnoblin_schema::{Catalog, Namespace, Type};

fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,{}", abi::SONAME);
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,defs");

    stage_soname_link();
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

/// The static catalog source and the hash of its canonical text.
fn render(catalog: &Catalog) -> (String, u64) {
    let mut source = String::from("pub(crate) static CLASSES: &[ClassDef] = &[\n");
    let mut canonical = String::new();
    // The tree namespace first, so a bare class name resolves there (`Catalog::namespaces` is sorted by name).
    let mut namespaces = catalog.namespaces();
    namespaces.sort_by_key(|namespace| (namespace.as_str() != "tree", namespace.as_str().to_owned()));
    for namespace in &namespaces {
        for object in catalog.objects(namespace) {
            let _ = writeln!(canonical, "class {}.{}", namespace.as_str(), object.name);
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
                let _ = writeln!(canonical, "p {} {kind} {flags}", property.name);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: {kind}, flags: {flags}, args: &[] }},",
                    property.name
                );
            }
            source.push_str("    ], methods: &[\n");
            for method in &object.methods {
                let args: Vec<u32> = method
                    .params
                    .iter()
                    .map(|field| value_kind(catalog, namespace, &field.ty, field.secret))
                    .collect();
                let flags = flag(method.is_sensitive(), abi::MEMBER_SECRET);
                let _ = writeln!(canonical, "m {} {flags} {args:?}", method.name);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: 0, flags: {flags}, args: &{args:?} }},",
                    method.name
                );
            }
            source.push_str("    ], signals: &[\n");
            for signal in &object.signals {
                let args: Vec<u32> = signal
                    .fields
                    .iter()
                    .map(|field| value_kind(catalog, namespace, &field.ty, field.secret))
                    .collect();
                let flags = flag(signal.is_sensitive(), abi::MEMBER_SECRET);
                let _ = writeln!(canonical, "s {} {flags} {args:?}", signal.name);
                let _ = writeln!(
                    source,
                    "        MemberDef {{ name: {:?}, value_kind: 0, flags: {flags}, args: &{args:?} }},",
                    signal.name
                );
            }
            source.push_str("    ] },\n");
        }
    }
    source.push_str("];\n");
    (source, fnv1a64(canonical.as_bytes()))
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
    }
}

/// FNV-1a, 64 bit: a stable hash that needs no dependency.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

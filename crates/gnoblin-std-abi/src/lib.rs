//! The C ABI between a shell and `libgnoblin-std`.
//!
//! `libgnoblin-std.so.1` is the optional library above the core: the object model of a desktop (clients, collections,
//! workspaces, layouts). It is reached the way the core is, through one versioned table of function pointers behind a few
//! exported symbols (`exports.txt`), so a function added in a later minor is invisible to a shell built against an earlier one
//! instead of changing whether it loads. This crate is the std
//! half of its code.
//!
//! ## Why a crate of its own
//!
//! The std table could have been a module of `gnoblin-core-abi`. It is a separate crate because the dependency arrow of the
//! design points one way: std builds on core and core never names std. A core ABI crate that declared std's table would
//! compile std's types into the core library and put std's churn into the core's `abi.lock`. This crate depends on
//! `gnoblin-core-abi` for the handles, the string type, the error codes and the declaration macros, and it is the leaf both
//! `gnoblin-std-impl` (the library) and `gnoblin-std` (the SDK) compile. Its `abi.lock` is `crates/gnoblin-std-abi/abi.lock`,
//! checked by `cargo xtask abi-check` beside the core's, and `cargo xtask abi-header` renders `include/gnoblin/std.h`.
//!
//! ## Versioning
//!
//! [`ABI_MAJOR`] is the SONAME major and the std SDK crate's semver major. Within a major the table only grows. The ABI is
//! unstable until 0.2.0 ships, as the core's is.

pub mod layouts;
pub mod types;

#[cfg(feature = "build-support")]
pub mod build_support;

pub use types::*;

use gnoblin_core_abi::reflect::Reflection;

/// The std ABI major version: the number in the SONAME `libgnoblin-std.so.1`. With the test feature `next-major` it is 2 while the
/// SONAME stays `.1` (see `gnoblin_core_abi::ABI_MAJOR`).
pub const ABI_MAJOR: u32 = if cfg!(feature = "next-major") { 2 } else { 1 };

/// The std ABI minor version: the count of additive revisions within the major. With the test feature `next-minor` it is 1 and the
/// table carries one more function (`uptime_ms`).
pub const ABI_MINOR: u32 = if cfg!(feature = "next-minor") { 1 } else { 0 };

/// The library patch level: the `<patch>` of `libgnoblin-std.so.<major>.<minor>.<patch>`. A `next-minor` build is a patch ahead too.
pub const ABI_PATCH: u32 = if cfg!(feature = "next-minor") { 1 } else { 0 };

/// True when the crate was built with the test-only `next-minor` feature. `cargo xtask abi-check` refuses such a build.
pub const NEXT_MINOR_FEATURE: bool = cfg!(feature = "next-minor");

/// True when the crate was built with the test-only `next-major` feature. `cargo xtask abi-check` refuses such a build too.
pub const NEXT_MAJOR_FEATURE: bool = cfg!(feature = "next-major");

/// The SONAME of the std library, and so the name a std shell records as NEEDED.
pub const SONAME: &str = "libgnoblin-std.so.1";

/// The name the linker is told to link (`-lgnoblin-std`).
pub const LINK_NAME: &str = "gnoblin-std";

/// The environment variable that points the link at an installed std library instead of a stub.
pub const LIB_DIR_VAR: &str = "GNOBLIN_STD_LIB_DIR";

/// The direct exports of the std library, one per line. The stub library, `dist-check` and the header read this.
pub const EXPORTS_TEXT: &str = include_str!("../exports.txt");

/// The symbols in [`EXPORTS_TEXT`].
pub fn export_names() -> impl Iterator<Item = &'static str> {
    EXPORTS_TEXT.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// The std ABI of this build of the crate, as data for the `abi.lock` check and the header generator.
pub fn reflection() -> Reflection {
    Reflection {
        major: ABI_MAJOR,
        minor: ABI_MINOR,
        patch: ABI_PATCH,
        soname: SONAME,
        exports: types::EXPORTS,
        consts: types::CONSTS,
        aliases: types::ALIASES,
        callbacks: types::CALLBACKS,
        structs: types::STRUCTS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn the_table_starts_with_the_four_header_words() {
        assert_eq!(offset_of!(GnoblinStdApi, size), 0);
        assert_eq!(offset_of!(GnoblinStdApi, major), 4);
        assert_eq!(offset_of!(GnoblinStdApi, minor), 8);
        assert_eq!(offset_of!(GnoblinStdApi, reserved), 12);
        assert_eq!(offset_of!(GnoblinStdApi, open), gnoblin_core_abi::API_HEADER_BYTES);
    }

    #[test]
    fn the_export_list_matches_the_declared_signatures() {
        let declared: Vec<&str> = types::EXPORTS.iter().map(|export| export.symbol).collect();
        let listed: Vec<&str> = export_names().collect();
        assert_eq!(
            declared, listed,
            "exports.txt and the `abi_exports!` declarations differ"
        );
    }

    #[test]
    fn every_export_is_in_the_std_namespace_and_unique() {
        let names: Vec<&str> = export_names().collect();
        for name in &names {
            assert!(
                name.starts_with("gnoblin_std_"),
                "{name} is not in the gnoblin_std_ namespace, so it could collide with the core's exports"
            );
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "an export is listed twice");
        let core: Vec<&str> = gnoblin_core_abi::export_names().collect();
        assert!(
            names.iter().all(|name| !core.contains(name)),
            "an export is the core's too"
        );
    }

    #[test]
    fn the_soname_follows_the_core_naming_rule() {
        // The test knob `next-major` changes the version a library reports and not the file name shells record.
        assert_eq!(SONAME, format!("lib{LINK_NAME}.so.1"));
        assert_eq!(ABI_MAJOR, if NEXT_MAJOR_FEATURE { 2 } else { 1 });
    }

    #[test]
    fn only_structs_that_start_with_a_size_are_extensible() {
        for layout in types::STRUCTS {
            let size_first = layout.fields.first().is_some_and(|field| field.name == "size");
            assert_eq!(layout.is_extensible(), size_first, "{}", layout.name);
        }
        let by_name = |name: &str| types::STRUCTS.iter().find(|layout| layout.name == name).unwrap();
        assert!(by_name("GnoblinStdApi").is_extensible());
        assert!(by_name("GnoblinStdLayoutSpec").is_extensible());
        assert!(!by_name("GnoblinStdRect").is_extensible());
        assert!(!by_name("GnoblinStdMembers").is_extensible());
    }

    #[test]
    fn reflected_offsets_are_what_rustc_laid_out() {
        let api = types::STRUCTS
            .iter()
            .find(|layout| layout.name == "GnoblinStdApi")
            .unwrap();
        assert_eq!(api.size, size_of::<GnoblinStdApi>());
        let last = api.fields.last().unwrap();
        assert_eq!(last.offset + last.size, api.size, "the last field ends the struct");
    }

    #[test]
    fn every_constant_is_prefixed_so_the_header_cannot_clash_with_the_cores() {
        let core: Vec<&str> = gnoblin_core_abi::types::CONSTS.iter().map(|c| c.name).collect();
        for constant in types::CONSTS {
            assert!(constant.name.starts_with("STD_"), "{}", constant.name);
            assert!(!core.contains(&constant.name), "{} is the core's too", constant.name);
        }
    }

    #[test]
    fn every_struct_and_alias_is_in_the_gnoblin_std_namespace() {
        for layout in types::STRUCTS {
            assert!(layout.name.starts_with("GnoblinStd"), "{}", layout.name);
        }
        for alias in types::ALIASES {
            assert!(alias.name.starts_with("GnoblinStd"), "{}", alias.name);
        }
        for callback in types::CALLBACKS {
            assert!(callback.name.starts_with("GnoblinStd"), "{}", callback.name);
        }
    }
}

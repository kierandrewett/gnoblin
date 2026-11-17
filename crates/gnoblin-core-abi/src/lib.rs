//! The C ABI between a shell and `libgnoblin-core`.
//!
//! Rust has no stable ABI, so the boundary is plain C: `#[repr(C)]` types, integer error codes, and one versioned table
//! of function pointers (`GnoblinApi`) behind four exported symbols (`exports.txt`). Everything else is reached through
//! the table, so a function added in a later minor is invisible to a shell built against an earlier one instead of
//! changing whether it loads. `design/modular-framework.md` section 5 is the full contract; this crate is its code.
//!
//! The crate holds declarations and no behaviour. It is the one place the ABI is written down:
//!
//! - [`types`]: the structs, constants, callbacks and exports.
//! - [`reflect`]: the same declarations as data (layouts, offsets, signatures). `cargo xtask abi-check` compares it with
//!   `abi.lock`, and `cargo xtask abi-header` renders it as `include/gnoblin/core.h`. There is no `cbindgen`: the
//!   layouts are what rustc computed, so a generator that read source text could disagree with the compiler.
//! - `build_support` (feature `build-support`): what a `build.rs` calls to link the core without building it first.
//!
//! ## Versioning
//!
//! [`ABI_MAJOR`] is the SONAME major and the SDK crates' semver major. Within a major the ABI only grows: fields are
//! appended to `GnoblinApi`, new constants and exports are added, nothing is moved, retyped or removed. `abi.lock`
//! enforces that in CI. The ABI is unstable until 0.2.0 ships (decision D18); the lock's `released` line records
//! where the freeze starts.

#[macro_use]
mod macros;

pub mod reflect;
pub mod types;

#[cfg(feature = "build-support")]
pub mod build_support;

pub use types::*;

/// The ABI major version: the number in the SONAME `libgnoblin-core.so.1`.
///
/// With the test feature `next-major` this is 2 while the SONAME stays `.1`: a core that answers `gnoblin_abi_version` with
/// another major than the shells in the field were built for. The upgrade tests build it to show the loader refuses such a core's
/// shells with both versions named.
pub const ABI_MAJOR: u32 = if cfg!(feature = "next-major") { 2 } else { 1 };

/// The ABI minor version: the count of additive revisions within the major.
///
/// With the test feature `next-minor` this is 1 and the table carries one more function (`uptime_ms`); the upgrade
/// tests build a core and a shell that way, and the patch level moves with it ([`ABI_PATCH`]).
pub const ABI_MINOR: u32 = if cfg!(feature = "next-minor") { 1 } else { 0 };

/// The library patch level: the `<patch>` of `libgnoblin-core.so.<major>.<minor>.<patch>`. A `next-minor` build is also a
/// patch ahead, so the version it reports differs in both of the numbers a shell can read.
pub const ABI_PATCH: u32 = if cfg!(feature = "next-minor") { 1 } else { 0 };

/// The SONAME of the core library, and so the name a shell records as NEEDED.
pub const SONAME: &str = "libgnoblin-core.so.1";

/// True when the crate was built with the test-only `next-minor` feature. `cargo xtask abi-check` refuses such a build:
/// the lock describes the ABI that ships.
pub const NEXT_MINOR_FEATURE: bool = cfg!(feature = "next-minor");

/// True when the crate was built with the test-only `next-major` feature. `cargo xtask abi-check` refuses such a build too.
pub const NEXT_MAJOR_FEATURE: bool = cfg!(feature = "next-major");

/// The name the linker is told to link (`-lgnoblin-core`).
pub const LINK_NAME: &str = "gnoblin-core";

/// The symbol a native shell exports.
pub const SHELL_ENTRY_SYMBOL: &str = "gnoshell_entry_v1";

/// The symbol a language runtime exports.
pub const RUNTIME_ENTRY_SYMBOL: &str = "gnoruntime_entry_v1";

/// The file name of the runtime for `extension` (`js`): `libgnoblin-runtime-js.so`. A runtime is found by this name, in the
/// runtime directories, when no built-in extension names it.
#[must_use]
pub fn runtime_file_name(extension: &str) -> String {
    format!("libgnoblin-runtime-{extension}.so")
}

/// The direct exports of the core library, one per line. The stub library, `dist-check` and the header read this.
pub const EXPORTS_TEXT: &str = include_str!("../exports.txt");

/// The symbols in [`EXPORTS_TEXT`].
pub fn export_names() -> impl Iterator<Item = &'static str> {
    EXPORTS_TEXT.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// The first four fields of [`GnoblinApi`]: `size`, `major`, `minor`, `reserved`.
pub const API_HEADER_BYTES: usize = 16;

/// True when a table of `table_size` bytes holds a field that ends at `field_end` (the offset plus the size of the
/// field). This is the one test a caller uses to ask whether the core it runs on has a function.
pub const fn table_has(table_size: u32, field_end: usize) -> bool {
    table_size as usize >= field_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn the_table_starts_with_the_four_header_words() {
        assert_eq!(offset_of!(GnoblinApi, size), 0);
        assert_eq!(offset_of!(GnoblinApi, major), 4);
        assert_eq!(offset_of!(GnoblinApi, minor), 8);
        assert_eq!(offset_of!(GnoblinApi, reserved), 12);
        assert_eq!(offset_of!(GnoblinApi, ctx_info), API_HEADER_BYTES);
    }

    #[test]
    fn every_function_pointer_is_one_word() {
        assert_eq!(size_of::<unsafe extern "C" fn()>(), size_of::<usize>());
        assert_eq!(size_of::<Option<GnoblinDropFn>>(), size_of::<usize>());
    }

    #[test]
    fn handles_are_sixty_four_bits() {
        for size in [
            size_of::<GnoblinCtx>(),
            size_of::<GnoblinReg>(),
            size_of::<GnoblinNode>(),
            size_of::<GnoblinPending>(),
            size_of::<GnoblinProc>(),
        ] {
            assert_eq!(size, 8);
        }
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
    fn every_export_is_prefixed_and_unique() {
        let names: Vec<&str> = export_names().collect();
        for name in &names {
            assert!(name.starts_with("gnoblin_"), "{name} is not in the gnoblin_ namespace");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "an export is listed twice");
    }

    #[test]
    fn error_codes_are_distinct_and_negative() {
        let errors: Vec<i128> = types::CONSTS
            .iter()
            .filter(|constant| constant.name.starts_with("E_") && constant.name != "E_OK")
            .map(|constant| constant.value)
            .collect();
        assert!(errors.len() >= 14);
        let mut sorted = errors.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), errors.len(), "two error codes share a value");
        assert!(errors.iter().all(|value| *value < 0));
        assert_eq!(E_OK, 0);
    }

    #[test]
    fn only_structs_that_start_with_a_size_are_extensible() {
        for layout in types::STRUCTS {
            let size_first = layout.fields.first().is_some_and(|field| field.name == "size");
            assert_eq!(layout.is_extensible(), size_first, "{}", layout.name);
        }
        let by_name = |name: &str| types::STRUCTS.iter().find(|layout| layout.name == name).unwrap();
        assert!(by_name("GnoblinApi").is_extensible());
        assert!(by_name("GnoblinShellEntry").is_extensible());
        assert!(!by_name("GnoblinStr").is_extensible());
        assert!(!by_name("GnoblinBuf").is_extensible());
    }

    #[test]
    fn reflected_offsets_are_what_rustc_laid_out() {
        let api = types::STRUCTS
            .iter()
            .find(|layout| layout.name == "GnoblinApi")
            .unwrap();
        assert_eq!(api.size, size_of::<GnoblinApi>());
        let last = api.fields.last().unwrap();
        assert_eq!(last.offset + last.size, api.size, "the last field ends the struct");
        for pair in api.fields.windows(2) {
            assert!(
                pair[0].offset + pair[0].size <= pair[1].offset,
                "{} overlaps {}",
                pair[0].name,
                pair[1].name
            );
        }
    }

    #[test]
    fn table_has_is_the_append_only_probe() {
        let end = offset_of!(GnoblinApi, stats) + size_of::<unsafe extern "C" fn()>();
        assert!(table_has(size_of::<GnoblinApi>() as u32, end));
        assert!(!table_has(end as u32 - 1, end));
    }

    #[test]
    fn the_test_knobs_move_the_version_and_nothing_else() {
        assert_eq!(ABI_MAJOR, if NEXT_MAJOR_FEATURE { 2 } else { 1 });
        assert_eq!(ABI_MINOR, u32::from(NEXT_MINOR_FEATURE));
        assert_eq!(ABI_PATCH, u32::from(NEXT_MINOR_FEATURE));
        // The file name a shell records does not change: that is what lets a library of another major be put in a prefix and
        // found by shells built for the first.
        assert_eq!(SONAME, "libgnoblin-core.so.1");
    }

    #[test]
    fn the_next_minor_table_is_one_function_longer() {
        let table = size_of::<GnoblinApi>();
        // `report_config_rejected` is the last function of the ABI 1.0 table.
        let base = offset_of!(GnoblinApi, report_config_rejected) + size_of::<unsafe extern "C" fn()>();
        if cfg!(feature = "next-minor") {
            assert_eq!(ABI_MINOR, 1);
            assert_eq!(table, base + size_of::<usize>());
        } else {
            assert_eq!(ABI_MINOR, 0);
            assert_eq!(table, base);
        }
    }
}

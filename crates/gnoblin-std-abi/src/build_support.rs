//! What a `build.rs` calls to link `libgnoblin-std` without building it first.
//!
//! The mechanism is the core's (`gnoblin_core_abi::build_support`): a generated stub named like the library, carrying the SONAME
//! `libgnoblin-std.so.1` and defining the three exports, so the link records `NEEDED libgnoblin-std.so.1` and nothing else of
//! ours, and the dynamic loader resolves the name to the installed library at run time. `GNOBLIN_STD_LIB_DIR` points the link at
//! a real `libgnoblin-std.so` instead.

use gnoblin_core_abi::build_support::{StubSpec, link_library};

/// The stub of the std library.
pub fn stub_spec() -> StubSpec<'static> {
    StubSpec {
        link_name: crate::LINK_NAME,
        soname: crate::SONAME,
        symbols: crate::export_names().collect(),
        lib_dir_var: crate::LIB_DIR_VAR,
    }
}

/// Links the std library into the crate being built: the installed one when `GNOBLIN_STD_LIB_DIR` names a directory,
/// otherwise a generated stub. Call it from `build.rs`.
pub fn link_std() {
    link_library(&stub_spec());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_std_stub_defines_the_std_exports_and_not_the_cores() {
        let source = stub_spec().source();
        for symbol in crate::export_names() {
            assert!(
                source.contains(&format!("void {symbol}(void) {{}}\n")),
                "{symbol} missing"
            );
        }
        assert_eq!(
            source.lines().filter(|line| line.starts_with("void ")).count(),
            crate::export_names().count()
        );
        assert!(!source.contains("void gnoblin_get_api"));
    }

    #[test]
    fn the_std_stub_carries_the_std_soname() {
        let spec = stub_spec();
        assert_eq!(spec.soname, "libgnoblin-std.so.1");
        assert_eq!(spec.link_name, "gnoblin-std");
    }
}

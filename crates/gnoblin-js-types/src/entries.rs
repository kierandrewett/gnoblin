//! The package entries an editor reads: what a bare `gnoblin-core` or `gnoblin-std` is.

/// The text of the entry module of `package` as an editor and a type checker read it, or `None` for a name that is no package.
///
/// A shell that imports no deep module of the library runs on the native library, where the entry `gnoblin-core` is the
/// generated classes of the core (and the global `gnoblin`), and `gnoblin-std` is the generated classes of the std, whose
/// import also adds the std's members to the core's. The editor entry re-exports the same declarations, so a script that
/// names `Workspace` is checked against the class it runs. The global `gnoblin` is declared by `gnoblin.d.ts`, so the entry
/// has no counterpart of the runtime's assignment to it.
#[must_use]
pub fn package_entry(package: &str) -> Option<&'static str> {
    match package {
        "gnoblin-core" => Some("export * from \"gnoblin-core/generated/objects\";\n"),
        "gnoblin-std" => Some("import \"gnoblin-core\";\nexport * from \"gnoblin-std/generated/objects\";\n"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::package_entry;

    #[test]
    fn the_two_packages_have_an_entry_and_nothing_else_does() {
        assert!(package_entry("gnoblin-core").is_some_and(|text| text.contains("gnoblin-core/generated/objects")));
        assert!(package_entry("gnoblin-std").is_some_and(|text| text.contains("gnoblin-std/generated/objects")));
        assert_eq!(package_entry("gnoblin-std/scene"), None);
    }
}

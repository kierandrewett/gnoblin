// The members `gnoblin` has by hand, beside the generated object types.
//
// Included by `build.rs`, which refuses a schema whose object type would take one of
// these names, and by `src/bindings.rs`, which sets them and lists them in a refusal.
// One list, so the build check and the installer cannot disagree about what is taken.

/// Names on the `gnoblin` table that are not generated object types.
pub(crate) const HAND_WRITTEN: &[&str] = &["configure", "layout", "load", "timers"];

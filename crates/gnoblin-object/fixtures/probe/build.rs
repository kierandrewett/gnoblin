//! Renders the fixture's catalog and its typed API into `OUT_DIR` (nothing of them is committed).

fn main() {
    gnoblin_sdkgen::emit_catalog_for_build(&["schema"], "probe");
}

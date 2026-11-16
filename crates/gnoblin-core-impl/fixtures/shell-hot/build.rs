//! Bakes `FIXTURE_LABEL` into the shell, so building the same source with `v1`, `v2` and `bad` yields three distinct
//! generations of one shell. The default label is `v1`.

fn main() {
    println!("cargo:rerun-if-env-changed=FIXTURE_LABEL");
    let label = std::env::var("FIXTURE_LABEL").unwrap_or_else(|_| "v1".to_owned());
    println!("cargo:rustc-env=FIXTURE_LABEL={label}");
}

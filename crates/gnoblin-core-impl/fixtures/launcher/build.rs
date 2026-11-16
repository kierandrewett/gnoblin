//! Links the core through the stub, as the `gnoblin` launcher will: NEEDED `libgnoblin-core.so.1` once, and an `$ORIGIN`
//! run path so the staged core next to the binary is found. The stub also defines the test driver's one extra export.

fn main() {
    gnoblin_core_abi::build_support::link_core(&["gnoblin_test_main"]);
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
}

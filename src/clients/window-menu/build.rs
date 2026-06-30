// Compiles this client's own UI; shared components are imported by path from
// the runtime vendored Slint library.
fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../crates/gnoblin-runtime/vendor");
    slint_build::compile("ui/window-menu.slint").expect("compile ui/window-menu.slint");
}

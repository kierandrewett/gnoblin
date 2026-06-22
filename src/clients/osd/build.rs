// Compiles this client's own UI; shared components are imported by path from
// the shell-ui vendored Slint library.
fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../shell-ui/vendor");
    slint_build::compile("ui/osd.slint").expect("compile ui/osd.slint");
}

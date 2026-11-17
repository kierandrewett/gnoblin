//! Links the installed core without building it: the runtime asks the core for the text of the last failure
//! (`gnoblin_last_error`), as every shell does, so it records `NEEDED libgnoblin-core.so.1`. See
//! `gnoblin_core_abi::build_support`.

fn main() {
    gnoblin_core_abi::build_support::link_core(&[]);
}

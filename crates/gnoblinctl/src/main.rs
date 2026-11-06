//! The `gnoblinctl` binary.
//!
//! Everything is in the library so the tests drive the same parse, the same command
//! logic and the same exit codes without spawning a process.

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(gnoblinctl::main())
}

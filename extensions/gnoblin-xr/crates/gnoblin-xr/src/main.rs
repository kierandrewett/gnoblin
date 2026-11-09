//! `gnoblin-xr`: the out-of-process OpenXR extension for Gnoblin.
//!
//! This is a placeholder. It parses `--help` and `--version` only. It does not
//! connect to the owner socket, read the `room` node or submit OpenXR layers yet.

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Print the usage text and exit successfully.
    Help,
    /// Print the version and exit successfully.
    Version,
    /// Anything else: print the usage text to stderr and exit with status 2.
    Invalid(String),
}

fn parse_args<I>(mut args: I) -> Action
where
    I: Iterator<Item = String>,
{
    match (args.next().as_deref(), args.next()) {
        (Some("--help" | "-h"), None) => Action::Help,
        (Some("--version" | "-V"), None) => Action::Version,
        (Some(other), _) => Action::Invalid(other.to_owned()),
        (None, _) => Action::Help,
    }
}

const USAGE: &str = "usage: gnoblin-xr [--help | --version]

Placeholder: the OpenXR layer bridge is not implemented yet.
See extensions/gnoblin-xr/README.md.";

fn main() -> std::process::ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Action::Help => {
            println!("{USAGE}");
            std::process::ExitCode::SUCCESS
        }
        Action::Version => {
            println!("gnoblin-xr {}", env!("CARGO_PKG_VERSION"));
            std::process::ExitCode::SUCCESS
        }
        Action::Invalid(argument) => {
            eprintln!("gnoblin-xr: unknown argument `{argument}`\n{USAGE}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Action {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn help_long_and_short_flags_print_help() {
        assert_eq!(parse(&["--help"]), Action::Help);
        assert_eq!(parse(&["-h"]), Action::Help);
    }

    #[test]
    fn version_long_and_short_flags_print_version() {
        assert_eq!(parse(&["--version"]), Action::Version);
        assert_eq!(parse(&["-V"]), Action::Version);
    }

    #[test]
    fn no_arguments_prints_help() {
        assert_eq!(parse(&[]), Action::Help);
    }

    #[test]
    fn unknown_argument_is_invalid() {
        assert_eq!(parse(&["--serve"]), Action::Invalid("--serve".to_owned()));
    }

    #[test]
    fn extra_arguments_after_a_flag_are_invalid() {
        assert_eq!(parse(&["--help", "extra"]), Action::Invalid("--help".to_owned()));
    }
}

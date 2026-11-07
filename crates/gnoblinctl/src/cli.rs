//! The clap command tree, built from the generated table, and the parse of one
//! command line into an [`Invocation`].
//!
//! The tree is built with clap's builder API because it is generated: every object
//! the schema declares becomes a subcommand, every declared property becomes a
//! possible value of `get`'s `PROPERTY` argument, and every declared method a
//! possible value of `call`'s `METHOD`, so a member the schema does not declare is
//! refused by the parser with a usage error (exit 2) rather than sent to the
//! compositor to be refused there.
//!
//! Arguments are `NAME=VALUE` pairs typed by the schema rather than a generated
//! `--flag` per parameter. One shape covers every method the schema can ever
//! declare, keeps `call <method> [args]` as designed, and makes the type of each
//! argument come from the same table the graph validates against.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::builder::{PossibleValue, PossibleValuesParser};
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

use gnoblin_model::Fields;
use gnoblin_store::LogQuery;

use crate::args;
use crate::logs;
use crate::tree::{self, CliObject};

/// The shells `completions` writes a script for.
pub(crate) const SHELLS: &[&str] = &["bash", "zsh", "fish"];

/// The levels `log --level` accepts, as the writer names them.
pub(crate) const LEVELS: &[&str] = &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

/// What one command line asked for.
#[derive(Debug)]
pub(crate) enum Invocation {
    /// An object's subcommand.
    Object {
        object: &'static CliObject,
        action: ObjectAction,
        json: bool,
    },
    /// Decode frames from a recorded stream.
    IpcDump { source: DumpSource },
    /// Write a completion script.
    Completions { shell: Shell, output: Option<PathBuf> },
    /// Print the stored log records, or follow them.
    Log { request: logs::LogRequest },
}

/// The action taken on an object.
#[derive(Debug)]
pub(crate) enum ObjectAction {
    /// Every live object of the type, from the subscription snapshot.
    List,
    /// One object, or one of its properties.
    Get { id: u64, property: Option<String> },
    /// Observations for the type as they arrive.
    Watch { id: Option<u64>, count: Option<u64> },
    /// One method call and the operation it returns.
    Call {
        id: u64,
        method: &'static tree::CliMethod,
        arguments: Fields,
    },
}

/// Where `ipc dump` reads its frames.
#[derive(Debug)]
pub(crate) enum DumpSource {
    /// Standard input, so the command can sit at the end of a pipe.
    Stdin,
    /// A recorded stream on disk.
    File(PathBuf),
}

/// One of the shells `completions` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    /// The script generator for this shell.
    pub(crate) fn generator(self) -> clap_complete::Shell {
        match self {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Fish => clap_complete::Shell::Fish,
        }
    }

    /// The shell named on the command line.
    fn parse(text: &str) -> Shell {
        match text {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            other => unreachable!("clap accepted `{other}` as a shell"),
        }
    }
}

/// Why a command line was not accepted, and what to do about it.
///
/// `help` and `version` are not failures: clap renders them for a request that
/// asked for them, and the text belongs on standard output with exit 0. Every
/// other refusal is a usage error and belongs on standard error with exit 2.
#[derive(Debug)]
pub(crate) struct Usage {
    pub(crate) text: String,
    pub(crate) help: bool,
}

impl Usage {
    /// The exit code this refusal reports.
    pub(crate) fn exit_code(&self) -> u8 {
        if self.help { 0 } else { 2 }
    }
}

/// The whole tree.
pub(crate) fn command_tree() -> Command {
    let mut root = Command::new("gnoblinctl")
        .about("Command line client for the Gnoblin IPC.")
        .version(env!("CARGO_PKG_VERSION"))
        .after_help(format!(
            "Speaks IPC {} and nothing else: a compositor at another version is refused with both versions named.\n\
             The socket is $GNOBLIN_IPC_SOCKET, or $XDG_RUNTIME_DIR/gnoblin/ipc.sock.",
            tree::ipc_version()
        ))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print JSON instead of text, for scripts."),
        );
    for object in tree::OBJECTS {
        root = root.subcommand(object_command(object));
    }
    root.subcommand(ipc_command())
        .subcommand(completions_command())
        .subcommand(log_command())
}

/// One object's subcommands.
fn object_command(object: &'static CliObject) -> Command {
    let mut command = Command::new(object.command)
        .about(format!(
            "Objects of type `{}` in the `{}` namespace.",
            object.name, object.namespace
        ))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("List every live object of this type."));
    if !object.properties.is_empty() {
        command = command.subcommand(get_command(object));
    }
    command = command.subcommand(watch_command(object));
    if !object.methods.is_empty() {
        command = command.subcommand(call_command(object));
    }
    command
}

/// `get <ID> [PROPERTY]`.
fn get_command(object: &'static CliObject) -> Command {
    Command::new("get")
        .about("Print one object, or one property of it.")
        .arg(
            Arg::new("id")
                .value_name("ID")
                .required(true)
                .value_parser(value_parser!(u64))
                .help("The compositor's identity for the object."),
        )
        .arg(
            Arg::new("property")
                .value_name("PROPERTY")
                .value_parser(PossibleValuesParser::new(
                    object
                        .properties
                        .iter()
                        .map(|property| PossibleValue::new(property.name).help(property.ty)),
                ))
                .help("One declared property, instead of every one."),
        )
}

/// `watch [ID] [--count N]`.
fn watch_command(object: &'static CliObject) -> Command {
    let signals = if object.signals.is_empty() {
        "the schema declares no signals".to_owned()
    } else {
        let names: Vec<&str> = object.signals.iter().map(|signal| signal.name).collect();
        format!("declared signals: {}", names.join(", "))
    };
    Command::new("watch")
        .about(format!(
            "Print every observation of a `{}` as it arrives ({signals}).",
            object.name
        ))
        .arg(
            Arg::new("id")
                .value_name("ID")
                .value_parser(value_parser!(u64))
                .help("One object, instead of every object of this type."),
        )
        .arg(
            Arg::new("count")
                .long("count")
                .value_name("N")
                .value_parser(value_parser!(u64))
                .help("Stop after N observations, so a script does not have to kill the command."),
        )
}

/// `call <ID> <METHOD> [NAME=VALUE ...]`.
fn call_command(object: &'static CliObject) -> Command {
    Command::new("call")
        .about("Call a declared method and wait for its operation to finish.")
        .arg(
            Arg::new("id")
                .value_name("ID")
                .required(true)
                .value_parser(value_parser!(u64))
                .help("The compositor's identity for the object."),
        )
        .arg(
            Arg::new("method")
                .value_name("METHOD")
                .required(true)
                .value_parser(PossibleValuesParser::new(object.method_names()))
                .help("One declared method."),
        )
        .arg(
            Arg::new("arguments")
                .value_name("NAME=VALUE")
                .num_args(0..)
                .action(ArgAction::Append)
                .help("Every parameter the method declares, typed by the schema."),
        )
}

/// `ipc <COMMAND>`.
fn ipc_command() -> Command {
    Command::new("ipc")
        .about("Inspect the IPC itself.")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("dump")
                .about("Decode IPC frames into readable lines, with secret payloads dropped.")
                .arg(
                    Arg::new("source")
                        .value_name("FILE")
                        .default_value("-")
                        .value_parser(value_parser!(PathBuf))
                        .help("A recorded frame stream, or `-` to read a pipe on standard input."),
                ),
        )
}

/// `completions <SHELL> [--output PATH]`.
fn completions_command() -> Command {
    Command::new("completions")
        .about("Write a shell completion script for this tree.")
        .arg(
            Arg::new("shell")
                .value_name("SHELL")
                .required(true)
                .value_parser(PossibleValuesParser::new(SHELLS)),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help("Write to PATH instead of standard output; `build/completions/` is gitignored."),
        )
}

/// `log` with its filters, time bounds and follow options.
fn log_command() -> Command {
    Command::new("log")
        .about("Print the stored log records, oldest first, from the log store.")
        .arg(
            Arg::new("store")
                .long("store")
                .value_name("DIR")
                .value_parser(value_parser!(PathBuf))
                .help("The store directory to read; defaults to $XDG_STATE_HOME/gnoblin/store."),
        )
        .arg(
            Arg::new("level")
                .long("level")
                .value_name("LEVEL")
                .ignore_case(true)
                .value_parser(PossibleValuesParser::new(LEVELS))
                .help("Only records at this level or above."),
        )
        .arg(
            Arg::new("role")
                .long("role")
                .value_name("ROLE")
                .help("Only records written by this process role."),
        )
        .arg(
            Arg::new("target")
                .long("target")
                .value_name("TARGET")
                .help("Only records whose event target matches exactly."),
        )
        .arg(Arg::new("since").long("since").value_name("TIME").help(
            "Only records at or after this time: RFC 3339 (2026-11-06T09:00:00Z), or an age such as 10m, 2h or 1d.",
        ))
        .arg(
            Arg::new("until")
                .long("until")
                .value_name("TIME")
                .help("Only records before this time, in the same forms as --since."),
        )
        .arg(
            Arg::new("follow")
                .long("follow")
                .action(ArgAction::SetTrue)
                .conflicts_with("until")
                .help("Keep printing new records as they are written, until interrupted."),
        )
        .arg(
            Arg::new("interval")
                .long("interval")
                .value_name("DURATION")
                .requires("follow")
                .help("How often --follow polls the store: 500ms or 2s. Defaults to 500ms."),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .value_name("N")
                .default_value("100")
                .value_parser(value_parser!(u32).range(1..))
                .help("The most records to print, newest last."),
        )
}

/// Parse one command line, without the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Invocation, Usage> {
    let matches = match command_tree().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(error) => {
            return Err(Usage {
                text: error.render().to_string(),
                help: matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ),
            });
        }
    };
    let json = matches.get_flag("json");
    let (name, sub) = matches.subcommand().expect("a subcommand is required");
    let invocation = match name {
        "ipc" => ipc_invocation(sub)?,
        "completions" => completions_invocation(sub)?,
        "log" => log_invocation(sub, json)?,
        command => {
            let object = tree::CliObject::find(command).expect("every subcommand comes from the generated tree");
            object_invocation(object, sub, json)?
        }
    };
    Ok(invocation)
}

/// The `ipc` subcommands.
fn ipc_invocation(matches: &ArgMatches) -> Result<Invocation, Usage> {
    let (name, sub) = matches.subcommand().expect("a subcommand is required");
    debug_assert_eq!(name, "dump");
    let source = match sub.get_one::<PathBuf>("source") {
        Some(path) if path != Path::new("-") => DumpSource::File(path.clone()),
        _ => DumpSource::Stdin,
    };
    Ok(Invocation::IpcDump { source })
}

/// The `completions` subcommand.
fn completions_invocation(matches: &ArgMatches) -> Result<Invocation, Usage> {
    let shell = Shell::parse(matches.get_one::<String>("shell").expect("clap requires the shell"));
    let output = matches.get_one::<PathBuf>("output").cloned();
    Ok(Invocation::Completions { shell, output })
}

/// A refusal of the `log` options, with the message the parser produced.
fn log_usage(message: String) -> Usage {
    Usage {
        text: format!("error: {message}\n"),
        help: false,
    }
}

/// The `log` subcommand.
fn log_invocation(matches: &ArgMatches, json: bool) -> Result<Invocation, Usage> {
    let store = matches.get_one::<PathBuf>("store").cloned();
    let query = LogQuery {
        min_level: matches
            .get_one::<String>("level")
            .map(|level| level.to_ascii_uppercase()),
        role: matches.get_one::<String>("role").cloned(),
        target: matches.get_one::<String>("target").cloned(),
        limit: Some(
            *matches
                .get_one::<u32>("limit")
                .expect("clap supplies the default limit"),
        ),
        ..LogQuery::default()
    };
    let time = |name: &str| {
        matches
            .get_one::<String>(name)
            .map(|text| logs::parse_bound(text).map_err(log_usage))
            .transpose()
    };
    let since = time("since")?;
    let until = time("until")?;
    let follow = matches
        .get_flag("follow")
        .then(|| match matches.get_one::<String>("interval") {
            Some(text) => logs::parse_interval(text).map_err(log_usage),
            None => Ok(logs::DEFAULT_INTERVAL),
        })
        .transpose()?;
    Ok(Invocation::Log {
        request: logs::LogRequest {
            store,
            query,
            since,
            until,
            follow,
            json,
        },
    })
}

/// One object's action.
fn object_invocation(object: &'static CliObject, matches: &ArgMatches, json: bool) -> Result<Invocation, Usage> {
    let (name, sub) = matches.subcommand().expect("a subcommand is required");
    let action = match name {
        "list" => ObjectAction::List,
        "get" => {
            let id = *sub.get_one::<u64>("id").expect("clap requires the id");
            let property = sub.get_one::<String>("property").cloned();
            ObjectAction::Get { id, property }
        }
        "watch" => {
            let id = sub.get_one::<u64>("id").copied();
            let count = sub.get_one::<u64>("count").copied();
            ObjectAction::Watch { id, count }
        }
        "call" => {
            let id = *sub.get_one::<u64>("id").expect("clap requires the id");
            let method_name = sub.get_one::<String>("method").expect("clap requires the method");
            let method = object.method(method_name).expect("clap accepts only declared methods");
            let tokens: Vec<String> = sub
                .get_many::<String>("arguments")
                .map(|values| values.cloned().collect())
                .unwrap_or_default();
            let arguments = args::parse(method, &tokens).map_err(|text| Usage { text, help: false })?;
            ObjectAction::Call { id, method, arguments }
        }
        other => unreachable!("`{other}` is not a generated action"),
    };
    Ok(Invocation::Object { object, action, json })
}

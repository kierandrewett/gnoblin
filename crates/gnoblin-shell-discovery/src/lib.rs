//! Which shell Gnoblin runs.
//!
//! A pure library: no compositor, no loader, no process state beyond [`Environment::from_env`]. The launcher
//! uses it so a failure appears before a compositor starts, the core uses it for `--list-shells` and reload, and
//! `gnoblinctl` uses it for `shell list` and `config init`. The seven tiers, in precedence order:
//!
//! | Tier | Source |
//! | ---- | ------ |
//! | 1 | the command line: a `PATH`, or `--shell NAME` ([`Request`]) |
//! | 2 | `GNOBLIN_SHELL`: a path when it contains `/`, else a name |
//! | 3 | `$XDG_CONFIG_HOME/gnoblin/config.toml`: `[shell] default = "name"` or `[shell] path = "..."` |
//! | 4 | `$XDG_CONFIG_HOME/gnoblin/shell.{so,py,js,mjs,ts,mts}` |
//! | 5 | `$XDG_DATA_HOME/gnoblin/shells/` |
//! | 6 | each `$XDG_DATA_DIRS` entry's `gnoblin/shells/`, in order |
//! | 7 | the packaged default, `<prefix>/share/gnoblin/default-shell/shell.<ext>` |
//!
//! The first tier that yields exactly one shell wins. See [`discover`] for the rules and [`DiscoveryError`] for
//! the text.
//!
//! # What the launcher calls
//!
//! ```no_run
//! use gnoblin_shell_discovery::{Environment, Request, discover, list_shells};
//!
//! let mut environment = Environment::from_env();
//! // environment.module_dir = Some(core_libdir.join("gnoblin/shells")); // where the core library is mapped from
//!
//! // `gnoblin`, `gnoblin ./myshell.so` or `gnoblin --shell tiler`:
//! match discover(&environment, &Request::Discover) {
//!     Ok(shell) => {
//!         // shell.path is the file to load, shell.language says how, shell.isolate is `[shell] isolate`.
//!         for warning in &shell.warnings {
//!             eprintln!("gnoblin: warning: {warning}");
//!         }
//!     }
//!     Err(error) => {
//!         eprintln!("gnoblin: {error}"); // the whole block, then exit 1: the compositor never starts
//!         std::process::exit(1);
//!     }
//! }
//!
//! // `gnoblin --list-shells`:
//! println!("{}", list_shells(&environment, &Request::Discover).render());
//! ```
//!
//! Reload and `--watch` call [`discover`] again with the same request and compare the [`Selection`]; they watch
//! [`Selection::path`] and, when it is set, [`Selection::launcher`].

mod candidate;
mod discover;
mod environment;
mod error;
mod language;
mod launcher;
mod list;
mod shell_config;
mod tier;
mod trace;

pub use candidate::{Candidate, DEFAULT_NAME, Form, Target};
pub use discover::{Selection, Warning, discover};
pub use environment::{
    CONFIG_FILE_NAME, DEFAULT_SHELL_DIRECTORY, Environment, LUA_CONFIG_FILE, SHELL_STEM, SHELLS_DIRECTORY,
};
pub use error::{DiscoveryError, Installed, Remedies, Searched};
pub use language::{KNOWN_EXTENSIONS, Language};
pub use list::{Listing, Row, list_shells};
pub use shell_config::{ConfigError, ShellConfig, read_shell_config};
pub use tier::{Request, Source, Tier};
pub use trace::{Trace, TraceLine};

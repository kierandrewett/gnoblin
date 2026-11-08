//! Every `gnoblinctl` example in the CLI docs must name a command the built client has.
//!
//! The examples are the inline code spans and the fenced shell lines that begin with
//! `gnoblinctl`. Each one is walked through the real command tree: every subcommand on
//! the way must exist, every `--flag` must be declared by a command on that path, and a
//! positional value at a position that declares possible values (such as `window get`'s
//! property or `window call`'s method) must be one of them. Placeholders such as `ID`,
//! `FILE`, `[VALUE]`, `<name>`, `$session` or `"Build"` stand for values and are not
//! checked. A documented command the client does not build is a failure, so the docs
//! cannot promise a verb that is not there.

use clap::{Arg, Command};

/// The documents whose examples are checked, relative to the repository root: the command
/// reference, the CLI overview, and the guides and troubleshooting pages that show `gnoblinctl`.
const DOCS: &[&str] = &[
    "docs/gnoblinctl.md",
    "docs/cli/index.md",
    "docs/troubleshooting.md",
    "docs/config.md",
    "docs/build-a-desktop.md",
    "docs/bring-your-own-shell.md",
    "docs/guides/files_and_load_order.md",
    "docs/guides/shortcuts.md",
];

/// Fenced languages whose lines are shell commands.
const SHELL_FENCES: &[&str] = &["sh", "bash", "console", "shell", "zsh", "fish", ""];

/// The fewest examples the docs must hold. A parse that stops finding them fails loudly
/// rather than passing vacuously.
const MINIMUM_EXAMPLES: usize = 25;

/// One documented command line, with where it came from.
#[derive(Debug)]
struct Example {
    file: &'static str,
    line: usize,
    text: String,
}

/// Every `gnoblinctl` example in one document, in order.
fn examples(file: &'static str, source: &str) -> Vec<Example> {
    let mut found = Vec::new();
    let mut in_fence = false;
    let mut shell_fence = false;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        if let Some(language) = fence_language(raw) {
            if in_fence {
                in_fence = false;
                shell_fence = false;
            } else {
                in_fence = true;
                shell_fence = SHELL_FENCES.contains(&language);
            }
            continue;
        }
        if in_fence {
            if shell_fence && raw.contains("gnoblinctl") {
                let text = strip_prompt(raw);
                found.push(Example { file, line, text });
            }
            continue;
        }
        // Prose: the code spans of the line, odd segments of a split on backticks.
        for span in raw.split('`').skip(1).step_by(2) {
            if span.starts_with("gnoblinctl") {
                found.push(Example {
                    file,
                    line,
                    text: span.to_string(),
                });
            }
        }
    }
    found
}

/// The language of a code fence line, or `None` when the line is not a fence.
fn fence_language(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix("```")?;
    if rest.contains(char::is_whitespace) || rest.contains('`') {
        return None;
    }
    Some(rest)
}

/// The line without a leading shell prompt, the way the docs write it.
fn strip_prompt(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let stripped = trimmed
        .strip_prefix("$ ")
        .or_else(|| trimmed.strip_prefix("# "))
        .unwrap_or(trimmed);
    stripped.trim_end().to_string()
}

/// The arguments after `gnoblinctl`, stopped at the first shell operator. An example that
/// pipes or redirects keeps only the part that runs `gnoblinctl`.
fn arguments(text: &str) -> Vec<&str> {
    let Some(start) = text.find("gnoblinctl") else {
        return Vec::new();
    };
    let mut arguments = Vec::new();
    for word in text[start + "gnoblinctl".len()..].split_whitespace() {
        if word.starts_with(['|', '>', ';', '&', ')']) {
            break;
        }
        arguments.push(word);
    }
    arguments
}

/// A word that stands for a value rather than a command: uppercase placeholders such as
/// `ID`, angle or square brackets, a shell variable, or a quoted string.
fn is_placeholder(word: &str) -> bool {
    let uppercase = word.chars().any(|c| c.is_ascii_uppercase())
        && word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'));
    uppercase || word.contains(['<', '[', '$', '"', '\'', '…']) || word.starts_with('.')
}

/// Whether `flag` is declared with a value by a command on `path`, so the word after it is
/// the flag's value and not a positional.
fn takes_value(path: &[&Command], flag: &str) -> bool {
    path.iter().any(|command| {
        command
            .get_arguments()
            .any(|arg| arg.get_long() == Some(flag) && arg.get_action().takes_values())
    })
}

/// The path a command reads as in an error message.
fn describe(path: &[&Command]) -> String {
    path.iter().map(|c| c.get_name()).collect::<Vec<_>>().join(" ")
}

/// Walks one example through the command tree. `Ok` when every subcommand and flag resolves
/// and every positional value that declares possible values is one of them.
fn check(root: &Command, text: &str) -> Result<(), String> {
    let mut node = root;
    let mut path = vec![root];
    let mut flags: Vec<String> = Vec::new();
    // Words that are positional values of the command reached, in order.
    let mut values: Vec<&str> = Vec::new();
    let mut leaf = false;
    let mut words = arguments(text).into_iter();
    while let Some(word) = words.next() {
        if let Some(flag) = word.strip_prefix("--") {
            let name = flag.split('=').next().unwrap_or(flag);
            // A flag that takes a value consumes the next word, unless written as `--flag=VALUE`.
            if !flag.contains('=') && takes_value(&path, name) {
                words.next();
            }
            flags.push(name.to_string());
            continue;
        }
        if leaf {
            values.push(word);
            continue;
        }
        if word == "help" {
            // `help` names a command, not a subcommand of this tree, so the rest is not checked.
            leaf = true;
            continue;
        }
        if node.get_subcommands().next().is_none() {
            leaf = true;
            values.push(word);
            continue;
        }
        match node.get_subcommands().find(|sub| sub.get_name() == word) {
            Some(sub) => {
                node = sub;
                path.push(sub);
            }
            None if is_placeholder(word) => {
                leaf = true;
                values.push(word);
            }
            None => {
                return Err(format!("`{word}` is not a command of `{}`", describe(&path)));
            }
        }
    }
    let positionals: Vec<&Arg> = node.get_positionals().collect();
    for (index, word) in values.iter().enumerate() {
        if is_placeholder(word) {
            continue;
        }
        let Some(arg) = positionals.get(index) else {
            continue;
        };
        let possible: Vec<_> = arg.get_possible_values();
        if possible.is_empty() {
            continue;
        }
        if !possible.iter().any(|v| v.get_name() == *word) {
            let names: Vec<&str> = possible.iter().map(|v| v.get_name()).collect();
            return Err(format!(
                "`{word}` is not a value of `{}` (<{}>); it takes {}",
                describe(&path),
                arg.get_id(),
                names.join(", ")
            ));
        }
    }
    for flag in flags {
        if matches!(flag.as_str(), "help" | "version") {
            continue;
        }
        let declared = path
            .iter()
            .any(|command| command.get_arguments().any(|arg| arg.get_long() == Some(flag.as_str())));
        if !declared {
            return Err(format!("`--{flag}` is not an option of `{}`", describe(&path)));
        }
    }
    Ok(())
}

#[test]
fn every_documented_gnoblinctl_example_names_a_built_command() {
    let root = gnoblinctl::command_tree();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for file in DOCS {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
        let source = std::fs::read_to_string(format!("{path}{file}"))
            .unwrap_or_else(|error| panic!("{file} is readable: {error}"));
        for example in examples(file, &source) {
            checked += 1;
            if let Err(reason) = check(&root, &example.text) {
                failures.push(format!(
                    "{}:{}: {reason}\n    {}",
                    example.file, example.line, example.text
                ));
            }
        }
    }
    assert!(
        checked >= MINIMUM_EXAMPLES,
        "the extractor found only {checked} examples (need {MINIMUM_EXAMPLES}); the docs moved or the parse broke"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} documented examples name a command, flag or value the client does not build:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A connection that must never open: `config validate` reads a file and needs no session.
struct NoSession;

impl gnoblinctl::Connection for NoSession {
    type Transport = gnoblinctl::UnixTransport;

    fn connect(self) -> Result<gnoblin_client::Client<gnoblinctl::UnixTransport>, gnoblinctl::ConnectFailure> {
        panic!("`config validate` must not open a session connection")
    }
}

/// Run the CLI on `args` with no session, returning its exit code and both streams.
fn run_without_session(args: &[&str]) -> (u8, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = gnoblinctl::run(args.iter().map(std::ffi::OsString::from), NoSession, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("stdout is UTF-8"),
        String::from_utf8(err).expect("stderr is UTF-8"),
    )
}

/// The documented `config validate FILE` example, run for real: a valid file exits 0 and
/// prints `valid`, a file that does not parse exits 1 with `invalid-config`. Exit 2 would be
/// the clap usage error this test exists to catch, so it is refused by name.
#[test]
fn config_validate_runs_on_a_valid_and_an_invalid_file() {
    let dir = std::env::temp_dir().join(format!("gnoblinctl-docs-validate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let valid = dir.join("valid.lua");
    let invalid = dir.join("invalid.lua");
    std::fs::write(&valid, "-- an empty configuration\n").expect("the valid file is written");
    std::fs::write(&invalid, "this is not lua\n").expect("the invalid file is written");
    let valid_path = valid.to_string_lossy().into_owned();
    let invalid_path = invalid.to_string_lossy().into_owned();

    let (code, out, err) = run_without_session(&["config", "validate", &valid_path]);
    assert_eq!(code, 0, "a valid file exits 0; stderr: {err}");
    assert_eq!(out, "valid\n", "a valid file prints `valid`");

    let (code, out, err) = run_without_session(&["config", "validate", &invalid_path]);
    assert_eq!(code, 1, "an invalid file exits 1, not the usage exit 2; stderr: {err}");
    assert!(out.is_empty(), "an invalid file prints nothing on stdout: {out}");
    assert!(
        err.contains("invalid-config"),
        "the refusal names `invalid-config`: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

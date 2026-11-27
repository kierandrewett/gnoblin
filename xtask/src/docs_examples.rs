//! `cargo xtask docs-examples` and `cargo xtask docs-reference`: the two commands the documentation build runs.
//!
//! # `docs-examples`
//!
//! Checks every example of a `::: tabs` group (see `docs_tabs`) in its own language, against what an install gives that language. Nothing
//! runs: each check reads the SDK and says whether the example names it correctly.
//!
//! | Language   | The check                                                                                                     | What passing proves                                                                                             |
//! | ---------- | ------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
//! | TypeScript | `tsc --strict` over the examples, with the `tsconfig.json` and the declarations `dist install` writes           | each import resolves, each member exists, each argument and result has the declared type                         |
//! | Python     | `pyright` (standard mode) over the examples, with the stub package `dist install` writes                       | the same, against the `.pyi` stubs: modules, names, members, argument and result types                          |
//! | Rust       | `cargo check` of one module per example in a crate that depends on `gnoblin-core` and `gnoblin-std`            | the example parses, resolves and type- and borrow-checks against the SDK crates. It is not linked or run         |
//!
//! None of the three starts a shell. Whether an example does the right thing in a session is what the example shells of `examples/` and
//! their tests prove. A failure names the page and the line in it: `docs/shell-api/idle.md:14:9: typescript: error TS2339: ...`.
//!
//! A fence with `skip="reason"` stays on the page and out of the check; the run lists every skip with its reason.
//!
//! A TypeScript example marked `client` is a program outside a shell (a bridge client). It is checked with the Node declarations
//! (`@types/node`) and without the shell's, so `node:` imports and `process` resolve there and nowhere else.
//!
//! # `docs-reference`
//!
//! Writes what an install gives an editor (`dist::write_editor_tree`: the TypeScript library with its generated declarations, and the Python stub
//! package) into `infra/site/.generated/sdk` by default, with the `tsconfig.docs.json` that TypeDoc reads. The generated declarations carry the
//! descriptions of the schema files as doc comments, so TypeDoc (TypeScript) and Sphinx (Python) write the reference of each language from them;
//! rustdoc reads the Rust crates, which `gnoblin-sdkgen` writes the same way. The website build runs the three (`infra/site/scripts/reference.mjs`).
//! Nothing of it is committed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::docs_tabs::{self, Example, Language};

const USAGE: &str = "usage: cargo xtask docs-examples [--lang typescript|python|rust]... [--out DIR] [PAGE.md ...]
       cargo xtask docs-reference [--out DIR]";

/// Where the checks keep their scratch files, under the ignored `target/`.
const SCRATCH: &str = "target/docs-examples";
/// Where the declarations the reference tools read are written.
const REFERENCE_OUT: &str = "infra/site/.generated/sdk";
const SITE_BIN: &str = "infra/site/node_modules/.bin";

/// One thing a checker said about an example.
#[derive(Debug, PartialEq, Eq)]
struct Finding {
    /// The line of the example's code, counted from 1; 0 when the tool named none.
    line: usize,
    column: usize,
    message: String,
}

/// An example with the name its scratch file has.
struct Unit<'a> {
    id: String,
    example: &'a Example,
}

fn unit_id(example: &Example) -> String {
    let page = example.page.trim_end_matches(".md").replace(['/', '-', '.'], "_");
    format!("p_{page}_L{}", example.fence_line)
}

/// A tool of the documentation build: `$ENV`, then the website's `node_modules`, then the path.
fn find_tool(env: &str, name: &str, how: &str) -> Result<PathBuf, String> {
    // Absolute, so the `node_modules` beside it, and the `typeRoots` written from it, do not depend on the directory tsc reads a
    // relative path from. Symlinks stay: `tsc` links into `typescript/bin`, and its `node_modules` is the one beside the link.
    let found = if let Some(explicit) = std::env::var_os(env).filter(|value| !value.is_empty()) {
        let explicit = PathBuf::from(explicit);
        if explicit.is_file() {
            explicit
        } else {
            return Err(format!("{env} names {}, which is not a file", explicit.display()));
        }
    } else {
        let local = Path::new(SITE_BIN).join(name);
        if local.is_file() {
            local
        } else {
            let path = std::env::var_os("PATH").unwrap_or_default();
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| format!("`{name}` is not installed: {how}, or set {env} to it"))?
        }
    };
    std::path::absolute(&found).map_err(|error| format!("cannot resolve {}: {error}", found.display()))
}

fn pages_under(docs: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path != docs.join("public") {
                pages_under(docs, &path, out)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// The `docs-examples` command.
pub fn run_examples(args: &[String]) -> ExitCode {
    match examples_command(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("[docs-examples] {error}");
            ExitCode::from(2)
        }
    }
}

fn examples_command(args: &[String]) -> Result<ExitCode, String> {
    let mut languages: Vec<Language> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--lang" => {
                let word = iter.next().ok_or_else(|| format!("--lang needs a language\n{USAGE}"))?;
                let language = Language::ALL
                    .into_iter()
                    .find(|language| language.slug() == word)
                    .ok_or_else(|| format!("`{word}` is not a language: typescript, python or rust\n{USAGE}"))?;
                languages.push(language);
            }
            "--out" => {
                out = Some(PathBuf::from(
                    iter.next().ok_or_else(|| format!("--out needs a directory\n{USAGE}"))?,
                ))
            }
            flag if flag.starts_with("--") => return Err(format!("unexpected argument `{flag}`\n{USAGE}")),
            file => files.push(PathBuf::from(file)),
        }
    }
    if languages.is_empty() {
        languages = Language::ALL.to_vec();
    }

    let repo = std::env::current_dir().map_err(|error| format!("cannot read the working directory: {error}"))?;
    let docs = repo.join("docs");
    if files.is_empty() {
        pages_under(&docs, &docs, &mut files)?;
    }
    let mut examples: Vec<Example> = Vec::new();
    let mut problems = Vec::new();
    for file in &files {
        let path = if file.exists() { file.clone() } else { docs.join(file) };
        let absolute = path
            .canonicalize()
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let page = absolute
            .strip_prefix(&docs)
            .map_err(|_| format!("{} is not a page under docs/", file.display()))?
            .to_string_lossy()
            .into_owned();
        let text =
            fs::read_to_string(&absolute).map_err(|error| format!("cannot read {}: {error}", absolute.display()))?;
        let (found, mistakes) = docs_tabs::examples(&page, &text);
        examples.extend(found);
        problems.extend(mistakes);
    }
    examples.extend(schema_examples(&repo)?);
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("{problem}");
        }
        eprintln!(
            "[docs-examples] {} problem(s) in the markup of the tabs",
            problems.len()
        );
        return Ok(ExitCode::FAILURE);
    }
    if examples.is_empty() {
        println!("[docs-examples] no `::: tabs` groups in {} page(s)", files.len());
        return Ok(ExitCode::SUCCESS);
    }

    let scratch = out.unwrap_or_else(|| repo.join(SCRATCH));
    let mut failed = false;
    for example in examples.iter().filter(|example| example.skip.is_some()) {
        println!(
            "[docs-examples] skipped docs/{}:{} ({}): {}",
            example.page,
            example.fence_line,
            example.language.slug(),
            example.skip.as_deref().unwrap_or_default()
        );
    }
    let mut prepared = false;
    for language in languages {
        let units: Vec<Unit<'_>> = examples
            .iter()
            .filter(|example| example.language == language && example.skip.is_none())
            .map(|example| Unit {
                id: unit_id(example),
                example,
            })
            .collect();
        if units.is_empty() {
            println!("[docs-examples] {}: no examples to check", language.slug());
            continue;
        }
        let dir = scratch.join(language.slug());
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| format!("cannot clear {}: {error}", dir.display()))?;
        }
        fs::create_dir_all(&dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        if matches!(language, Language::TypeScript | Language::Python) && !prepared {
            let prefix = scratch.join("prefix");
            if prefix.exists() {
                fs::remove_dir_all(&prefix).map_err(|error| format!("cannot clear {}: {error}", prefix.display()))?;
            }
            crate::dist::write_editor_tree(&repo, &prefix)
                .map_err(|error| format!("cannot write the editor files: {error}"))?;
            prepared = true;
        }
        let findings = match language {
            Language::TypeScript => check_typescript(&repo, &scratch, &dir, &units)?,
            Language::Python => check_python(&scratch, &dir, &units)?,
            Language::Rust => check_rust(&repo, &dir, &units)?,
        };
        let mut bad = 0;
        for (id, found) in &findings {
            let Some(unit) = units.iter().find(|unit| &unit.id == id) else {
                continue;
            };
            bad += 1;
            for finding in found {
                let line = if finding.line == 0 {
                    unit.example.fence_line
                } else {
                    unit.example.fence_line + finding.line
                };
                println!(
                    "docs/{}:{line}:{}: {}: {}",
                    unit.example.page,
                    finding.column.max(1),
                    language.slug(),
                    finding.message
                );
            }
        }
        if let Some(other) = findings.get("") {
            bad += 1;
            for finding in other {
                println!("{}: {}", language.slug(), finding.message);
            }
        }
        println!(
            "[docs-examples] {}: {} example(s) checked, {bad} failed",
            language.slug(),
            units.len()
        );
        failed |= bad > 0;
    }
    Ok(if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// The findings of one run, by the id of the example they belong to. Findings that belong to no example (the tool itself failed) are under `""`.
type Findings = BTreeMap<String, Vec<Finding>>;

fn check_typescript(repo: &Path, scratch: &Path, dir: &Path, units: &[Unit<'_>]) -> Result<Findings, String> {
    let tsc = find_tool("GNOBLIN_TSC", "tsc", "run `yarn install` in infra/site")?;
    let (client, shell): (Vec<&Unit<'_>>, Vec<&Unit<'_>>) = units.iter().partition(|unit| unit.example.client);
    let mut findings = Findings::new();
    if !shell.is_empty() {
        // The shell's programs: the installed configuration, which names no Node types, and the SDK declarations beside them.
        let prefix = scratch.join("prefix/share/gnoblin/types");
        let mut files = vec![serde_json::Value::String(
            prefix.join("gnoblin.d.ts").display().to_string(),
        )];
        files.extend(write_units(dir, &shell)?);
        let config = dir.join("tsconfig.json");
        let document = serde_json::json!({
            "extends": prefix.join("tsconfig.json").display().to_string(),
            "files": files,
        });
        write_config(&config, &document)?;
        merge_findings(&mut findings, run_tsc(&tsc, repo, &config)?);
    }
    if !client.is_empty() {
        // A program outside a shell (a bridge client) runs under Node. Its declarations are `@types/node`, found in the `node_modules`
        // that holds `tsc`, and none of the shell's declarations are in its program.
        let types = tsc
            .parent()
            .and_then(Path::parent)
            .map(|modules| modules.join("@types"))
            .ok_or_else(|| format!("cannot find node_modules beside {}", tsc.display()))?;
        if !types.join("node").is_dir() {
            return Err(format!(
                "@types/node is not in {}: run `yarn install` in infra/site",
                types.display()
            ));
        }
        let config = dir.join("tsconfig.client.json");
        let document = serde_json::json!({
            "compilerOptions": {
                "target": "es2023",
                "lib": ["es2023"],
                "module": "esnext",
                "moduleResolution": "bundler",
                "strict": true,
                "types": ["node"],
                "typeRoots": [types.display().to_string()],
            },
            "files": write_units(dir, &client)?,
        });
        write_config(&config, &document)?;
        merge_findings(&mut findings, run_tsc(&tsc, repo, &config)?);
    }
    Ok(findings)
}

/// Writes each example as a module of its own, so two examples that declare the same name do not meet. Returns the paths for `files`.
fn write_units(dir: &Path, units: &[&Unit<'_>]) -> Result<Vec<serde_json::Value>, String> {
    units
        .iter()
        .map(|unit| {
            let path = dir.join(format!("{}.ts", unit.id));
            fs::write(&path, format!("{}\nexport {{}};\n", unit.example.code))
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            Ok(serde_json::Value::String(path.display().to_string()))
        })
        .collect()
}

fn write_config(path: &Path, document: &serde_json::Value) -> Result<(), String> {
    fs::write(path, document.to_string()).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn merge_findings(into: &mut Findings, from: Findings) {
    for (id, found) in from {
        into.entry(id).or_default().extend(found);
    }
}

/// Runs `tsc -p config` in the repository and returns what it says about the examples, by unit id.
fn run_tsc(tsc: &Path, repo: &Path, config: &Path) -> Result<Findings, String> {
    let output = Command::new(tsc)
        .args(["--noEmit", "--pretty", "false", "-p"])
        .arg(config)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", tsc.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut findings = parse_tsc(&text);
    // The stock library is type-checked beside the examples, and it does not check clean unless `gnoblin-std` is in the same program (the
    // std library declares the `workspace` its `host/gnoblin.ts` reads). That is the library's own business, `cargo test` of its crate
    // holds it, and an example is not at fault: only what a diagnostic says about an example fails the run.
    if let Some(other) = findings.get_mut("") {
        let before = other.len();
        other.retain(|finding| !finding.message.contains("prefix/share/gnoblin/"));
        if other.len() < before {
            println!(
                "[docs-examples] typescript: {} diagnostic(s) in the stock library files are not the examples' and are left out",
                before - other.len()
            );
        }
        if other.is_empty() {
            findings.remove("");
        }
    }
    if !output.status.success() && findings.is_empty() && !text.contains("error TS") {
        findings.entry(String::new()).or_default().push(Finding {
            line: 0,
            column: 0,
            message: format!("tsc failed: {text}{}", String::from_utf8_lossy(&output.stderr)),
        });
    }
    Ok(findings)
}

/// `tsc --pretty false` writes `path(line,col): error TSnnnn: message`, and indents what continues a message.
fn parse_tsc(text: &str) -> Findings {
    let mut findings = Findings::new();
    let mut last: Option<(String, usize)> = None;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some((id, index)) = &last
                && let Some(finding) = findings.get_mut(id).and_then(|found| found.get_mut(*index))
            {
                finding.message.push(' ');
                finding.message.push_str(line.trim());
            }
            continue;
        }
        let parsed = line.split_once("): ").and_then(|(head, message)| {
            let (path, position) = head.rsplit_once('(')?;
            let (line, column) = position.split_once(',')?;
            Some((
                path,
                line.parse::<usize>().ok()?,
                column.parse::<usize>().ok()?,
                message,
            ))
        });
        let Some((path, line_number, column, message)) = parsed else {
            if !line.trim().is_empty() {
                let found = findings.entry(String::new()).or_default();
                found.push(Finding {
                    line: 0,
                    column: 0,
                    message: line.to_owned(),
                });
                last = Some((String::new(), found.len() - 1));
            }
            continue;
        };
        let id = Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let known = id.starts_with("p_");
        let key = if known { id } else { String::new() };
        let found = findings.entry(key.clone()).or_default();
        found.push(Finding {
            line: if known { line_number } else { 0 },
            column,
            message: if known {
                message.to_owned()
            } else {
                format!("{path}({line_number},{column}): {message}")
            },
        });
        last = Some((key, found.len() - 1));
    }
    findings
}

fn check_python(scratch: &Path, dir: &Path, units: &[Unit<'_>]) -> Result<Findings, String> {
    let pyright = find_tool("GNOBLIN_PYRIGHT", "pyright", "run `yarn install` in infra/site")?;
    let stubs = scratch.join("prefix/share/gnoblin/types/python");
    let sources = dir.join("src");
    fs::create_dir_all(&sources).map_err(|error| format!("cannot create {}: {error}", sources.display()))?;
    for unit in units {
        let path = sources.join(format!("{}.py", unit.id));
        fs::write(&path, &unit.example.code).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    let config = dir.join("pyrightconfig.json");
    let document = serde_json::json!({
        "include": ["src"],
        "extraPaths": [stubs.display().to_string()],
        "pythonVersion": "3.11",
        "typeCheckingMode": "standard",
        "reportMissingModuleSource": "none",
    });
    fs::write(&config, document.to_string()).map_err(|error| format!("cannot write {}: {error}", config.display()))?;
    let output = Command::new(&pyright)
        .args(["--outputjson", "-p"])
        .arg(dir)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", pyright.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut findings = parse_pyright(&text)?;
    if !output.status.success() && findings.is_empty() {
        findings.entry(String::new()).or_default().push(Finding {
            line: 0,
            column: 0,
            message: format!("pyright failed: {text}{}", String::from_utf8_lossy(&output.stderr)),
        });
    }
    Ok(findings)
}

/// `pyright --outputjson`: every error of `generalDiagnostics`, in the example it names. Lines and columns count from 0 there.
fn parse_pyright(text: &str) -> Result<Findings, String> {
    let document: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("pyright wrote no JSON ({error}): {text}"))?;
    let mut findings = Findings::new();
    for diagnostic in document["generalDiagnostics"].as_array().into_iter().flatten() {
        if diagnostic["severity"] != "error" {
            continue;
        }
        let file = diagnostic["file"].as_str().unwrap_or_default();
        let id = Path::new(file)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let known = id.starts_with("p_");
        let line = diagnostic["range"]["start"]["line"].as_u64().unwrap_or(0) as usize + 1;
        let column = diagnostic["range"]["start"]["character"].as_u64().unwrap_or(0) as usize + 1;
        let rule = diagnostic["rule"]
            .as_str()
            .map(|rule| format!(" ({rule})"))
            .unwrap_or_default();
        let message = format!(
            "{}{rule}",
            diagnostic["message"].as_str().unwrap_or_default().replace('\n', " ")
        );
        findings
            .entry(if known { id } else { String::new() })
            .or_default()
            .push(Finding {
                line: if known { line } else { 0 },
                column,
                message: if known {
                    message
                } else {
                    format!("{file}:{line}:{column}: {message}")
                },
            });
    }
    Ok(findings)
}

fn check_rust(repo: &Path, dir: &Path, units: &[Unit<'_>]) -> Result<Findings, String> {
    let sources = dir.join("src/examples");
    fs::create_dir_all(&sources).map_err(|error| format!("cannot create {}: {error}", sources.display()))?;
    let mut lib = String::from(
        "//! Written by `cargo xtask docs-examples`: one module for each Rust example of the documentation.\n#![allow(dead_code, unused, clippy::all)]\n\n",
    );
    for unit in units {
        let path = sources.join(format!("{}.rs", unit.id));
        fs::write(&path, &unit.example.code).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        lib.push_str(&format!("#[path = \"examples/{}.rs\"]\nmod {};\n", unit.id, unit.id));
    }
    fs::write(dir.join("src/lib.rs"), lib).map_err(|error| format!("cannot write the crate: {error}"))?;
    let crates = repo.join("crates");
    let manifest = format!(
        "[package]\nname = \"gnoblin-docs-examples\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\ngnoblin-core = {{ path = {core:?} }}\ngnoblin-std = {{ path = {std:?} }}\nserde_json = \"1\"\n\n# A workspace of its own: the crate is built beside the repository, not in it.\n[workspace]\n",
        core = crates.join("gnoblin-core").display().to_string(),
        std = crates.join("gnoblin-std").display().to_string(),
    );
    fs::write(dir.join("Cargo.toml"), manifest).map_err(|error| format!("cannot write the manifest: {error}"))?;
    // The same versions of every dependency as the workspace builds with.
    fs::copy(repo.join("Cargo.lock"), dir.join("Cargo.lock"))
        .map_err(|error| format!("cannot copy Cargo.lock: {error}"))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args([
            "check",
            "--lib",
            "--color",
            "never",
            "--message-format",
            "short",
            "--manifest-path",
        ])
        .arg(dir.join("Cargo.toml"))
        .current_dir(repo);
    if std::env::var_os("CARGO_TARGET_DIR").is_none() {
        // The checks share the target directory of the workspace, so the SDK crates build once.
        command.env("CARGO_TARGET_DIR", repo.join("target/docs-examples-build"));
    }
    let output = command.output().map_err(|error| format!("cannot run cargo: {error}"))?;
    let text = String::from_utf8_lossy(&output.stderr);
    let mut findings = parse_rustc(&text);
    if !output.status.success() && findings.is_empty() {
        findings.entry(String::new()).or_default().push(Finding {
            line: 0,
            column: 0,
            message: format!("cargo check failed: {text}"),
        });
    }
    Ok(findings)
}

/// `cargo check --message-format short` writes `path:line:col: error[E0425]: message` for each diagnostic, and a final summary line.
fn parse_rustc(text: &str) -> Findings {
    let mut findings = Findings::new();
    for line in text.lines() {
        let Some((position, message)) = line.split_once(": error") else {
            continue;
        };
        let mut parts = position.rsplitn(3, ':');
        let (Some(column), Some(row), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let id = Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let known = id.starts_with("p_");
        findings
            .entry(if known { id } else { String::new() })
            .or_default()
            .push(Finding {
                line: if known { row.parse().unwrap_or(0) } else { 0 },
                column: column.parse().unwrap_or(0),
                message: if known {
                    format!("error{message}")
                } else {
                    line.to_owned()
                },
            });
    }
    findings
}

/// The `docs-reference` command.
pub fn run_reference(args: &[String]) -> ExitCode {
    match reference_command(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[docs-reference] {error}");
            ExitCode::from(2)
        }
    }
}

fn reference_command(args: &[String]) -> Result<(), String> {
    let mut out = PathBuf::from(REFERENCE_OUT);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => out = PathBuf::from(iter.next().ok_or_else(|| format!("--out needs a directory\n{USAGE}"))?),
            other => return Err(format!("unexpected argument `{other}`\n{USAGE}")),
        }
    }
    // The directory is this command's: a file a renamed class left behind goes with it.
    if out.exists() {
        fs::remove_dir_all(&out).map_err(|error| format!("cannot clear {}: {error}", out.display()))?;
    }
    let repo = std::env::current_dir().map_err(|error| format!("cannot read the working directory: {error}"))?;
    crate::dist::write_editor_tree(&repo, &out).map_err(|error| format!("cannot write the declarations: {error}"))?;
    // TypeDoc reads the two entry modules the installed `tsconfig.json` maps the package names to, and that file lists none of them.
    let types = out.join("share/gnoblin/types");
    let config = serde_json::json!({
        "extends": "./tsconfig.json",
        "files": ["./gnoblin.d.ts", "./gnoblin-core/index.ts", "./gnoblin-std/index.ts"],
    });
    let path = types.join("tsconfig.docs.json");
    fs::write(&path, format!("{config:#}\n")).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    println!("[docs-reference] declarations written to {}", out.display());
    Ok(())
}

/// The examples the schema files carry (`example` on a class, property, method or signal of a library's catalog), as the examples of a page
/// are: each is checked in its own language. They are named `schema:<namespace>.<class>[.<member>]` where a page's name goes.
fn schema_examples(repo: &Path) -> Result<Vec<Example>, String> {
    let spec = gnoblin_sdkgen::SdkSpec::load(repo)?;
    let mut found = Vec::new();
    let mut add = |name: String, notes: &gnoblin_schema::Notes| {
        let Some(example) = &notes.example else { return };
        for (language, code) in [
            (Language::TypeScript, &example.typescript),
            (Language::Python, &example.python),
            (Language::Rust, &example.rust),
        ] {
            if let Some(code) = code {
                found.push(Example {
                    page: name.clone(),
                    fence_line: 0,
                    language,
                    code: code.clone(),
                    skip: None,
                    client: false,
                });
            }
        }
    };
    for schema in spec.objects.schemas().iter().filter(|schema| schema.local) {
        for object in schema.objects.iter().filter(|object| object.augment_of.is_none()) {
            let at = |member: Option<&str>| match member {
                Some(member) => format!("schema:{}.{}.{member}", schema.namespace, object.name),
                None => format!("schema:{}.{}", schema.namespace, object.name),
            };
            add(at(None), &object.notes);
            for property in &object.properties {
                add(at(Some(&property.name)), &property.notes);
            }
            for method in &object.methods {
                add(at(Some(&method.name)), &method.notes);
            }
            for signal in &object.signals {
                add(at(Some(&signal.name)), &signal.notes);
            }
        }
    }
    Ok(found)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tsc_error_is_found_in_its_example_and_a_continuation_joins_it() {
        let text = "/t/ts/p_shell_api_idle_L9.ts(3,5): error TS2339: Property 'x' does not exist on type 'Y'.\n  The type is Y.\n/t/prefix/lib.d.ts(1,1): error TS1: other\n";
        let findings = parse_tsc(text);
        let mine = &findings["p_shell_api_idle_L9"];
        assert_eq!((mine[0].line, mine[0].column), (3, 5));
        assert!(mine[0].message.contains("does not exist") && mine[0].message.contains("The type is Y."));
        assert!(findings[""][0].message.contains("lib.d.ts(1,1)"));
    }

    #[test]
    fn a_pyright_error_counts_from_one_and_a_warning_is_not_a_failure() {
        let json = r#"{"generalDiagnostics":[
            {"file":"/t/src/p_a_L2.py","severity":"error","message":"Cannot access attribute \"x\"","rule":"reportAttributeAccessIssue","range":{"start":{"line":4,"character":2}}},
            {"file":"/t/src/p_a_L2.py","severity":"warning","message":"w","range":{"start":{"line":0,"character":0}}}]}"#;
        let findings = parse_pyright(json).expect("JSON");
        assert_eq!(findings["p_a_L2"].len(), 1);
        assert_eq!((findings["p_a_L2"][0].line, findings["p_a_L2"][0].column), (5, 3));
    }

    #[test]
    fn a_rustc_diagnostic_is_found_in_its_example() {
        let text = "src/examples/p_a_L7.rs:3:9: error[E0425]: cannot find value `x` in this scope\nerror: could not compile `gnoblin-docs-examples` (lib) due to 1 previous error\n";
        let findings = parse_rustc(text);
        assert_eq!((findings["p_a_L7"][0].line, findings["p_a_L7"][0].column), (3, 9));
    }
}

//! `config migrate`: an `init.lua` becomes an `init.ts`.
//!
//! The conversion is `gnoblin-config-migrate`, a pure function of the Lua text. This module is the I/O around it: it
//! finds the file, reads it and the files it loads, refuses to overwrite, writes `init.ts` and the editor files, loads
//! the result once with the runtime's own `defineConfig` so a file that would not load is said so, and prints the
//! report. It links no Lua host: the converter reads Lua as text and never runs it, so the command keeps working when
//! the Lua shell is gone.
//!
//! Exit codes: 0 when everything converted, 1 when the file was written but something was left out (or the converted
//! file does not load), 2 when nothing was written because the command line or the files were wrong.

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use gnoblin_cli_style::{ErrorReport, LineOutcome, OutputTheme, error_block, kv_list, section, status_line};
use gnoblin_config::SearchPaths;
use gnoblin_config_migrate::{
    ConfigSourceLoader, MigrateSource, MigrationOutput, glob_matches, is_glob, migrate, resolve_load_path,
};
use gnoblin_js_types::{TSCONFIG_FILE, TYPES_FILE, ensure_user_files};
use serde_json::json;

use crate::js;

/// The file `config migrate` reads when none is named, and the name Lua's config entry point has.
const LUA_INIT: &str = "init.lua";

/// The shell file that sits beside `init.lua`: `seat.data.window_rules` and the other seat settings live there.
const LUA_SHELL: &str = "shell.lua";

/// The file it writes unless told otherwise.
const TS_INIT: &str = "init.ts";

/// The entry the TypeScript shell falls back to when there is no `init.ts`; a written `init.ts` would shadow it.
const JS_INIT: &str = "init.js";

/// How deep a `**` glob looks.
const MAX_GLOB_DEPTH: usize = 16;

/// What the command line asked for.
#[derive(Debug)]
pub(crate) struct MigrateRequest {
    pub file: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub force: bool,
    pub dry_run: bool,
    pub json: bool,
}

/// The `init.lua` the Lua shell would read: `GNOBLIN_CONFIG` when it is a Lua file, else `init.lua` in the config
/// directory.
fn located_init_lua(paths: &SearchPaths) -> Result<PathBuf, String> {
    if let Some(path) = &paths.override_path {
        return if path.extension().is_some_and(|extension| extension == "lua") {
            Ok(path.clone())
        } else {
            Err(format!(
                "GNOBLIN_CONFIG names {}, which is not a Lua file",
                path.display()
            ))
        };
    }
    js::default_config_dir(paths)
        .map(|dir| dir.join(LUA_INIT))
        .ok_or_else(|| "neither XDG_CONFIG_HOME nor HOME is set, so there is no config directory".to_owned())
}

/// Reads one Lua file for the converter.
fn read_source(path: &Path, name: &str) -> Result<MigrateSource, String> {
    fs::read_to_string(path)
        .map(|text| MigrateSource {
            name: name.to_owned(),
            text,
        })
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// The files a `gnoblin.load` names, found under the directory of the config.
struct DirLoader {
    root: PathBuf,
}

impl DirLoader {
    /// Every file below `dir` that is at most `depth` levels down, as a path relative to the config directory.
    fn walk(dir: &Path, relative: &str, depth: usize, found: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let child = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if path.is_dir() {
                if depth > 1 {
                    Self::walk(&path, &child, depth - 1, found);
                }
            } else if path.is_file() {
                found.push(child);
            }
        }
    }
}

impl ConfigSourceLoader for DirLoader {
    fn expand(&self, from: &str, pattern: &str) -> Result<Vec<MigrateSource>, String> {
        let resolved = resolve_load_path(from, pattern);
        if resolved.starts_with('/') {
            if is_glob(&resolved) {
                return Err(format!(
                    "{resolved} is a glob outside the config directory, which the converter does not follow"
                ));
            }
            return read_source(Path::new(&resolved), &resolved).map(|source| vec![source]);
        }
        if !is_glob(&resolved) {
            let path = self.root.join(&resolved);
            if !path.is_file() {
                return Err(format!("{resolved} does not exist"));
            }
            return read_source(&path, &resolved).map(|source| vec![source]);
        }
        // Start at the longest directory the pattern names without a wildcard, and look no deeper than it can match.
        let segments: Vec<&str> = resolved.split('/').collect();
        let literal: Vec<&str> = segments
            .iter()
            .copied()
            .take_while(|segment| !is_glob(segment))
            .collect();
        let prefix = literal.join("/");
        let depth = if segments.contains(&"**") {
            MAX_GLOB_DEPTH
        } else {
            segments.len() - literal.len()
        };
        let mut found = Vec::new();
        Self::walk(&self.root.join(&prefix), &prefix, depth, &mut found);
        found.retain(|name| glob_matches(&resolved, name));
        // A glob is sorted before it is read, so the load order never depends on the directory's.
        found.sort();
        found
            .iter()
            .map(|name| read_source(&self.root.join(name), name))
            .collect()
    }
}

/// An error block on standard error, and exit 2.
fn fail(err: &mut dyn Write, summary: String, why: Option<String>, hint: &str) -> u8 {
    let theme = js::theme(io::stderr().is_terminal());
    let report = ErrorReport {
        summary,
        why,
        hints: vec![hint.to_owned()],
    };
    let _ = writeln!(err, "{}", error_block(&theme, &report));
    2
}

/// `config migrate [FILE] [-o OUT] [--force] [--dry-run]`.
pub(crate) fn run(request: &MigrateRequest, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let file = match &request.file {
        Some(file) => file.clone(),
        None => match located_init_lua(&SearchPaths::from_env()) {
            Ok(file) => file,
            Err(message) => return fail(err, message, None, "name the file: `gnoblinctl config migrate FILE`"),
        },
    };
    if !file.is_file() {
        return fail(
            err,
            format!("{} is not a file, so there is nothing to convert", file.display()),
            None,
            "name the Lua file: `gnoblinctl config migrate FILE`",
        );
    }
    let dir = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let name = file
        .file_name()
        .map_or_else(|| LUA_INIT.to_owned(), |name| name.to_string_lossy().into_owned());
    let target = request.out.clone().unwrap_or_else(|| dir.join(TS_INIT));
    if !request.dry_run
        && !request.force
        && let Some(refusal) = overwrite_refusal(&target)
    {
        return fail(
            err,
            format!("{} already exists, so nothing was written", refusal.path.display()),
            Some(refusal.why),
            "name another file with `-o OUT`, or run `gnoblinctl config migrate --force` to replace it",
        );
    }

    let entry = match read_source(&file, &name) {
        Ok(entry) => entry,
        Err(message) => return fail(err, message, None, "check that the file is readable UTF-8 text"),
    };
    // A shell file beside init.lua carries the window rules and the seat settings.
    let shell = dir.join(LUA_SHELL);
    let mut followers = Vec::new();
    if name == LUA_INIT && shell.is_file() {
        match read_source(&shell, LUA_SHELL) {
            Ok(source) => followers.push(source),
            Err(message) => return fail(err, message, None, "check that the file is readable UTF-8 text"),
        }
    }
    let output = match migrate(&entry, &followers, &DirLoader { root: dir.clone() }) {
        Ok(output) => output,
        Err(error) => {
            return fail(
                err,
                error.to_string(),
                Some("the Lua shell could not load this file either".to_owned()),
                "fix the syntax error and run the command again",
            );
        }
    };
    // The converted text is loaded once with the runtime's own `defineConfig`, before it is written.
    let load_problem = js::validate_define_config(&output.typescript).err();

    if request.dry_run {
        return report_dry_run(request, &output, load_problem.as_deref(), out, err);
    }
    if let Err(error) = write_converted(&target, &output.typescript, request.force) {
        return fail(
            err,
            format!("cannot write {}: {error}", target.display()),
            None,
            "check that the directory is writable, or name another file with `-o OUT`",
        );
    }
    let target_dir = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let written = match ensure_user_files(&target_dir) {
        Ok(written) => written,
        Err(error) => {
            return fail(
                err,
                format!(
                    "{} was written, but the editor files were not: {error}",
                    target.display()
                ),
                None,
                "check that the directory is writable, then run `gnoblinctl types init`",
            );
        }
    };
    report_written(request, &output, load_problem.as_deref(), &target, &written, out, err)
}

/// Why a file in the way is not written over.
struct WriteRefusal {
    path: PathBuf,
    why: String,
}

fn overwrite_refusal(target: &Path) -> Option<WriteRefusal> {
    if target.exists() {
        return Some(WriteRefusal {
            path: target.to_path_buf(),
            why: "the shell would keep reading it".to_owned(),
        });
    }
    // A `init.ts` written beside an `init.js` is read instead of it, as `config init` also refuses.
    let sibling = target.with_file_name(JS_INIT);
    if target.file_name().is_some_and(|name| name == TS_INIT) && sibling.exists() {
        return Some(WriteRefusal {
            path: sibling,
            why: format!("an {TS_INIT} written here would be read instead of it"),
        });
    }
    None
}

/// Writes the converted file, refusing to replace one unless `force`.
fn write_converted(target: &Path, text: &str, force: bool) -> io::Result<()> {
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(target)?.write_all(text.as_bytes())
}

/// The three steps after a conversion, for the person and for `--json`.
fn next_steps(target: &Path, lua: &Path) -> Vec<String> {
    vec![
        format!(
            "Review {}: each UNMAPPED comment is something to port by hand.",
            target.display()
        ),
        format!("Check it with `gnoblinctl check --types {}`.", target.display()),
        format!(
            "Log out and back in. From the next session the TypeScript shell runs and {} takes precedence over {}.",
            target.display(),
            lua.display()
        ),
    ]
}

fn converted_pairs(output: &MigrationOutput) -> Vec<(&'static str, usize)> {
    let counts = &output.report.converted;
    [
        ("settings", counts.settings),
        ("workspaces", counts.workspaces),
        ("keys", counts.keys),
        ("autostart", counts.autostart),
        ("rules", counts.rules),
        ("workspace sets", counts.workspace_sets),
        ("options", counts.options),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect()
}

/// The report for a person: what converted, what was left out and where to read about it.
fn report_text(output: &MigrationOutput, load_problem: Option<&str>, theme: &OutputTheme) -> String {
    let mut text = String::new();
    let pairs = converted_pairs(output);
    text.push_str(&section(theme, "Converted"));
    text.push('\n');
    if pairs.is_empty() {
        text.push_str("nothing: the files hold no setting, shortcut, workspace, autostart entry or rule\n");
    } else {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(label, count)| ((*label).to_owned(), count.to_string()))
            .collect();
        let borrowed: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        text.push_str(&kv_list(theme, &borrowed));
        text.push('\n');
    }
    let unmapped = &output.report.unmapped;
    if !unmapped.is_empty() {
        text.push('\n');
        text.push_str(&section(theme, &format!("Left out ({})", unmapped.len())));
        text.push('\n');
        for item in unmapped {
            text.push_str(&status_line(
                theme,
                LineOutcome::Warn,
                &format!("{}:{}  {}", item.file, item.line, item.summary),
            ));
            text.push_str(&format!("\n      {}\n      see {}\n", item.reason, item.pointer));
        }
    }
    for note in &output.report.notes {
        text.push_str(&status_line(theme, LineOutcome::Info, note));
        text.push('\n');
    }
    if let Some(problem) = load_problem {
        text.push('\n');
        text.push_str(&status_line(
            theme,
            LineOutcome::Fail,
            &format!("the converted file does not load: {problem}"),
        ));
        text.push_str("\n      this is a fault in the converter: edit the file by hand, and report it\n");
    }
    text
}

fn exit_code(output: &MigrationOutput, load_problem: Option<&str>) -> u8 {
    u8::from(!output.report.unmapped.is_empty() || load_problem.is_some())
}

fn json_report(output: &MigrationOutput, load_problem: Option<&str>) -> serde_json::Value {
    let counts = &output.report.converted;
    json!({
        "sources": output.report.sources,
        "converted": {
            "settings": counts.settings,
            "workspaces": counts.workspaces,
            "keys": counts.keys,
            "autostart": counts.autostart,
            "rules": counts.rules,
            "workspace_sets": counts.workspace_sets,
            "options": counts.options,
        },
        "unmapped": output.report.unmapped.iter().map(|item| json!({
            "file": item.file,
            "line": item.line,
            "summary": item.summary,
            "reason": item.reason,
            "pointer": item.pointer,
        })).collect::<Vec<_>>(),
        "notes": output.report.notes,
        "load_problem": load_problem,
    })
}

/// `--dry-run`: the TypeScript on standard output, and the report beside it on standard error. With `--json` standard
/// output is one document that carries the TypeScript.
fn report_dry_run(
    request: &MigrateRequest,
    output: &MigrationOutput,
    load_problem: Option<&str>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let code = exit_code(output, load_problem);
    let result = if request.json {
        let mut document = json_report(output, load_problem);
        document["dry_run"] = json!(true);
        document["written"] = json!(false);
        document["typescript"] = json!(output.typescript);
        writeln!(out, "{document}")
    } else {
        let theme = js::theme(io::stderr().is_terminal());
        let report = report_text(output, load_problem, &theme);
        out.write_all(output.typescript.as_bytes()).and_then(|()| {
            writeln!(
                err,
                "{}\n\n{report}\nRun again without --dry-run to write init.ts.",
                status_line(&theme, LineOutcome::Info, "dry run: nothing was written"),
            )
        })
    };
    if result.is_err() {
        return 2;
    }
    code
}

/// The report after `init.ts` and the editor files are written.
fn report_written(
    request: &MigrateRequest,
    output: &MigrationOutput,
    load_problem: Option<&str>,
    target: &Path,
    written: &gnoblin_js_types::JsWritten,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let code = exit_code(output, load_problem);
    let lua = request.file.clone().unwrap_or_else(|| target.with_file_name(LUA_INIT));
    let next = next_steps(target, &lua);
    let target_dir = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let result = if request.json {
        let mut document = json_report(output, load_problem);
        document["dry_run"] = json!(false);
        document["written"] = json!(true);
        document["init"] = json!(target);
        document["types"] = json!(written.types);
        document["tsconfig"] = json!(target_dir.join(TSCONFIG_FILE));
        document["tsconfig_created"] = json!(written.tsconfig.is_some());
        document["next"] = json!(next);
        writeln!(out, "{document}")
    } else {
        let theme = js::theme(io::stdout().is_terminal());
        let tsconfig = match &written.tsconfig {
            Some(path) => (LineOutcome::Ok, format!("{TSCONFIG_FILE} created: {}", path.display())),
            None => (
                LineOutcome::Info,
                format!(
                    "{TSCONFIG_FILE} kept, it already exists: {}",
                    target_dir.join(TSCONFIG_FILE).display()
                ),
            ),
        };
        let steps: String = next
            .iter()
            .enumerate()
            .map(|(number, step)| format!("\n  {}. {step}", number + 1))
            .collect();
        writeln!(
            out,
            "{}\n{}\n{}\n\n{}\nNext:{steps}",
            status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{TS_INIT} written: {}", target.display())
            ),
            status_line(
                &theme,
                LineOutcome::Ok,
                &format!("{TYPES_FILE} refreshed: {}", written.types.display())
            ),
            status_line(&theme, tsconfig.0, &tsconfig.1),
            report_text(output, load_problem, &theme),
        )
    };
    if result.is_err() {
        let _ = writeln!(err, "gnoblinctl: cannot write the report");
        return 2;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_located_file_is_init_lua_in_the_config_directory_or_the_override() {
        let paths = SearchPaths {
            home: Some(PathBuf::from("/home/u")),
            config_home: None,
            override_path: None,
        };
        assert_eq!(
            located_init_lua(&paths),
            Ok(PathBuf::from("/home/u/.config/gnoblin/init.lua"))
        );
        let configured = SearchPaths {
            override_path: Some(PathBuf::from("/x/my.lua")),
            ..paths.clone()
        };
        assert_eq!(located_init_lua(&configured), Ok(PathBuf::from("/x/my.lua")));
        let not_lua = SearchPaths {
            override_path: Some(PathBuf::from("/x/init.ts")),
            ..paths
        };
        assert!(located_init_lua(&not_lua).is_err());
        assert!(located_init_lua(&SearchPaths::default()).is_err());
    }
}

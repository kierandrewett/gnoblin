//! The converter's walk over the statements of a Lua config.
//!
//! Nothing here runs Lua. A top-level statement is read for what it says: `gnoblin.configure` with a table that can
//! be evaluated statically, `gnoblin.load` of the files beside it, a `local` or a global bound to a static value, and
//! in a shell file the `seat.data` assignments that have a TypeScript equivalent. What converts is handed to
//! [`crate::emit`], which prints it as statements against the specification's objects. Every other statement is left
//! out of the output and written there as an `UNMAPPED` comment, and listed in the report with its line and a pointer
//! into the migration page. A value that is part of a table and cannot be evaluated is treated the same way, setting
//! by setting, so one computed setting does not cost the settings around it.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use full_moon::ast::{Assignment, Block, FunctionCall, LocalAssignment, Stmt, Var};
use full_moon::node::Node;

use crate::chord::{ChordTranslation, canonical_trigger, translate_chord};
use crate::emit::{self, Head, KeyDef, Needs, Placed, RuleDef, Test, WorkspaceDef};
use crate::lua::{self, LuaField, LuaKind, LuaScope, Lv, SourceFile, Span};
use crate::names;
use crate::pattern::{PatternTranslation, translate_lua_pattern};
use crate::{
    ConfigSourceLoader, ConvertedCounts, MigrateError, MigrateSource, MigrationOutput, MigrationReport, UnmappedItem,
};

/// Where the migration page says what each construct becomes.
const CONSTRUCTS: &str = "docs/migrating-from-lua.md#what-each-construct-becomes";

/// Where the migration page says what the converter cannot do.
const LIMITS: &str = "docs/migrating-from-lua.md#what-the-converter-cannot-do";

/// Where the migration page lists the settings that have no TypeScript home yet.
const DROPPED: &str = "docs/migrating-from-lua.md#settings-with-no-typescript-home-yet";

/// How many files a `gnoblin.load` may nest, the limit the Lua host had.
const MAX_LOAD_DEPTH: usize = 32;

/// How many lines of a left-out statement are copied into its comment.
const MAX_COMMENT_LINES: usize = 10;

/// The longest summary the report prints for a left-out construct.
const MAX_SUMMARY: usize = 52;

/// The workspaces the stock desktop declares, which a config that declares none keeps.
const STOCK_WORKSPACES: &[&str] = &["1", "2", "3", "4"];

/// Which section a shortcut came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeySection {
    Shortcuts,
    Keybindings,
}

impl KeySection {
    fn name(self) -> &'static str {
        match self {
            KeySection::Shortcuts => "shortcuts",
            KeySection::Keybindings => "keybindings",
        }
    }
}

/// One named shortcut, its fields merged across the files that mention it.
struct ShortcutDef {
    section: KeySection,
    name: String,
    span: Span,
    binding: Option<(String, Span)>,
    command: Option<Vec<String>>,
    release: bool,
    enabled: bool,
}

/// A rule that has been read, and where its workspace was named, for the check against the declared workspaces.
struct RuleDraft {
    def: RuleDef,
    line: usize,
    workspace_span: Option<Span>,
}

/// A group of statements that is worked out at the end and printed where its first member was met.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Batch {
    Workspaces,
    Keys,
    Rules,
}

/// One thing the output holds, in the order it was met.
enum Piece {
    /// `From FILE.` ahead of the first thing a file contributed.
    Header(Rc<str>),
    /// Comment lines: what was left out, with where and why.
    Note(Vec<String>),
    /// Statements worked out at the end.
    Batch(Batch),
}

pub(crate) struct Engine<'a> {
    loader: &'a dyn ConfigSourceLoader,
    sources: Vec<String>,
    unmapped: Vec<UnmappedItem>,
    notes: Vec<String>,
    /// What the output holds, in the order it was first met.
    entries: Vec<Piece>,
    /// The file the last `From ...` comment named.
    header: Option<Rc<str>>,
    globals: HashMap<String, Lv>,
    /// The files being read, outermost first, so a load that names one of them is refused.
    stack: Vec<String>,
    workspaces: Option<Vec<Placed<WorkspaceDef>>>,
    shortcuts: Vec<ShortcutDef>,
    rules: Option<Vec<Placed<RuleDraft>>>,
    /// What `seat.data.alerts` said, when it said anything.
    alerts: Option<bool>,
}

impl<'a> Engine<'a> {
    pub(crate) fn new(loader: &'a dyn ConfigSourceLoader) -> Self {
        Engine {
            loader,
            sources: Vec::new(),
            unmapped: Vec::new(),
            notes: Vec::new(),
            entries: Vec::new(),
            header: None,
            globals: HashMap::new(),
            stack: Vec::new(),
            workspaces: None,
            shortcuts: Vec::new(),
            rules: None,
            alerts: None,
        }
    }

    /// Reads one source file. A syntax error in the entry file is an error; in a file it loaded or a shell file it is
    /// one left-out construct, so the rest of the config still converts.
    pub(crate) fn run_source(&mut self, source: &MigrateSource, entry: bool) -> Result<(), MigrateError> {
        let name: Rc<str> = Rc::from(source.name.as_str());
        self.sources.push(source.name.clone());
        let ast = match lua::parse(&source.text) {
            Ok(ast) => ast,
            Err((line, message)) => {
                if entry {
                    return Err(MigrateError::Syntax {
                        file: source.name.clone(),
                        line,
                        message,
                    });
                }
                let span = Span {
                    file: Rc::clone(&name),
                    line,
                    text: "the whole file".to_owned(),
                };
                self.leave_out(&name, &span, &format!("the file does not parse: {message}"), LIMITS);
                return Ok(());
            }
        };
        let file = SourceFile {
            name: Rc::clone(&name),
            source: &source.text,
        };
        self.stack.push(source.name.clone());
        let mut locals = HashMap::new();
        self.exec_block(&file, &mut locals, ast.nodes());
        self.stack.pop();
        Ok(())
    }

    // --- Reporting ---------------------------------------------------------------------------------------------

    /// Lists `span` as left out and returns the comment lines that mark its place in the output.
    fn report(&mut self, span: &Span, reason: &str, pointer: &'static str) -> Vec<String> {
        let item = UnmappedItem {
            file: span.file.to_string(),
            line: span.line,
            summary: summary(&span.text),
            reason: reason.to_owned(),
            pointer,
        };
        if !self.unmapped.contains(&item) {
            self.unmapped.push(item);
        }
        comment_lines(span, reason)
    }

    /// Lists `span` as left out and writes its comment among the statements of the config.
    fn leave_out(&mut self, file: &Rc<str>, span: &Span, reason: &str, pointer: &'static str) {
        let lines = self.report(span, reason, pointer);
        self.push_entry(file, Piece::Note(lines));
    }

    // --- The output --------------------------------------------------------------------------------------------

    /// Adds an entry, ahead of it a comment naming the file when that is not the file of the last one.
    fn push_entry(&mut self, file: &Rc<str>, entry: Piece) {
        if self.header.as_ref() != Some(file) {
            self.header = Some(Rc::clone(file));
            self.entries.push(Piece::Header(Rc::clone(file)));
        }
        self.entries.push(entry);
    }

    /// Fixes where a group stands: where its first member was met.
    fn reserve(&mut self, file: &Rc<str>, group: Batch) {
        if !self
            .entries
            .iter()
            .any(|entry| matches!(entry, Piece::Batch(known) if *known == group))
        {
            self.push_entry(file, Piece::Batch(group));
        }
    }

    // --- Statements --------------------------------------------------------------------------------------------

    fn exec_block(&mut self, file: &SourceFile<'_>, locals: &mut HashMap<String, Lv>, block: &Block) {
        for stmt in block.stmts() {
            self.exec_stmt(file, locals, stmt);
        }
        if let Some(last) = block.last_stmt() {
            let span = file.span(last);
            self.leave_out(
                &file.name,
                &span,
                "a `return` in a config file hands a value to the host: port it by hand",
                LIMITS,
            );
        }
    }

    fn exec_stmt(&mut self, file: &SourceFile<'_>, locals: &mut HashMap<String, Lv>, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionCall(call) => self.exec_call(file, locals, call),
            Stmt::LocalAssignment(local) => self.exec_local(file, locals, stmt, local),
            Stmt::Assignment(assignment) => self.exec_assignment(file, locals, stmt, assignment),
            Stmt::FunctionDeclaration(_) | Stmt::LocalFunction(_) => {
                self.leave_out_stmt(file, stmt, "a function is code: port it by hand", LIMITS);
            }
            Stmt::If(_) => self.leave_out_stmt(file, stmt, "an `if` is decided when the config runs", LIMITS),
            Stmt::NumericFor(_) | Stmt::GenericFor(_) | Stmt::While(_) | Stmt::Repeat(_) => {
                self.leave_out_stmt(file, stmt, "a loop runs when the config runs", LIMITS);
            }
            Stmt::Do(_) => self.leave_out_stmt(file, stmt, "a `do` block is code: port it by hand", LIMITS),
            _ => self.leave_out_stmt(file, stmt, "a statement the converter does not read", LIMITS),
        }
    }

    fn leave_out_stmt(&mut self, file: &SourceFile<'_>, node: &impl Node, reason: &str, pointer: &'static str) {
        let span = file.span(node);
        self.leave_out(&file.name, &span, reason, pointer);
    }

    fn exec_call(&mut self, file: &SourceFile<'_>, locals: &HashMap<String, Lv>, call: &FunctionCall) {
        let Some((path, args)) = lua::call_target(call, file) else {
            let span = file.span(call);
            let reason = if span.text.contains(":bind(") {
                "a key bound to a Lua function: bind it with `input.bind` and a function"
            } else {
                "a call runs code when the config runs: port it by hand"
            };
            self.leave_out(&file.name, &span, reason, LIMITS);
            return;
        };
        let joined = path.join(".");
        let span = file.span(call);
        match joined.as_str() {
            "gnoblin.configure" => {
                let args = lua::lower_args(
                    file,
                    &LuaScope {
                        locals,
                        globals: &self.globals,
                    },
                    args,
                );
                match args.as_slice() {
                    [only] if only.fields().is_some() => self.configure(file, only),
                    [only] => {
                        let reason = match &only.kind {
                            LuaKind::Failed(reason) => reason.clone(),
                            _ => format!("`gnoblin.configure` takes a table, not {}", only.describe()),
                        };
                        self.leave_out(&file.name, &span, &reason, LIMITS);
                    }
                    _ => self.leave_out(&file.name, &span, "`gnoblin.configure` takes one table", LIMITS),
                }
            }
            "gnoblin.load" => {
                let args = lua::lower_args(
                    file,
                    &LuaScope {
                        locals,
                        globals: &self.globals,
                    },
                    args,
                );
                match args.as_slice() {
                    [only] if only.as_str().is_some() => {
                        let pattern = only.as_str().unwrap_or_default().to_owned();
                        self.load(file, &span, &pattern);
                    }
                    _ => self.leave_out(
                        &file.name,
                        &span,
                        "`gnoblin.load` takes one literal path or glob",
                        LIMITS,
                    ),
                }
            }
            other => {
                let reason = call_reason(other);
                self.leave_out(&file.name, &span, reason, LIMITS);
            }
        }
    }

    /// `gnoblin.load(pattern)`: reads the files the pattern names, in place and in the order the loader gives.
    fn load(&mut self, file: &SourceFile<'_>, span: &Span, pattern: &str) {
        let files = match self.loader.expand(&file.name, pattern) {
            Ok(files) => files,
            Err(message) => {
                self.leave_out(&file.name, span, &message, LIMITS);
                return;
            }
        };
        if files.is_empty() {
            self.notes.push(format!(
                "{}:{}: gnoblin.load(\"{pattern}\") matches no file, as in Lua, where that is not an error",
                span.file, span.line
            ));
        }
        for loaded in files {
            if self.stack.contains(&loaded.name) {
                let reason = format!("{} loads itself", loaded.name);
                self.leave_out(&file.name, span, &reason, LIMITS);
            } else if self.stack.len() >= MAX_LOAD_DEPTH {
                let reason = format!("more than {MAX_LOAD_DEPTH} files are loaded inside each other");
                self.leave_out(&file.name, span, &reason, LIMITS);
            } else if let Err(error) = self.run_source(&loaded, false) {
                // `run_source` fails only for an entry file, which a loaded file is not.
                self.leave_out(&file.name, span, &error.to_string(), LIMITS);
            }
        }
    }

    fn exec_local(
        &mut self,
        file: &SourceFile<'_>,
        locals: &mut HashMap<String, Lv>,
        stmt: &Stmt,
        local: &LocalAssignment,
    ) {
        let expressions: Vec<_> = local.expressions().iter().collect();
        let mut failure: Option<String> = None;
        for (index, token) in local.names().iter().enumerate() {
            let name = token.token().to_string();
            let value = match expressions.get(index) {
                None => Lv {
                    span: file.span(token),
                    kind: LuaKind::Nil,
                },
                Some(expr) => {
                    let scope = LuaScope {
                        locals: &*locals,
                        globals: &self.globals,
                    };
                    lua::lower(file, &scope, expr)
                }
            };
            if let LuaKind::Failed(reason) = &value.kind {
                failure.get_or_insert_with(|| reason.clone());
                locals.remove(&name);
            } else {
                locals.insert(name, value);
            }
        }
        if let Some(reason) = failure {
            self.leave_out_stmt(file, stmt, &reason, LIMITS);
        }
    }

    fn exec_assignment(
        &mut self,
        file: &SourceFile<'_>,
        locals: &HashMap<String, Lv>,
        stmt: &Stmt,
        assignment: &Assignment,
    ) {
        let variables: Vec<&Var> = assignment.variables().iter().collect();
        let expressions: Vec<_> = assignment.expressions().iter().collect();
        let (&[variable], &[expr]) = (variables.as_slice(), expressions.as_slice()) else {
            self.leave_out_stmt(file, stmt, "a multiple assignment is not read", LIMITS);
            return;
        };
        let value = {
            let scope = LuaScope {
                locals,
                globals: &self.globals,
            };
            lua::lower(file, &scope, expr)
        };
        let span = file.span(stmt);
        if let LuaKind::Failed(reason) = &value.kind {
            if let Var::Name(token) = variable {
                self.globals.remove(&token.token().to_string());
            }
            self.leave_out(&file.name, &span, reason, LIMITS);
            return;
        }
        match variable {
            Var::Name(token) => {
                self.globals.insert(token.token().to_string(), value);
            }
            Var::Expression(target) => {
                let Some(path) = lua::dotted_path(target.prefix(), target.suffixes(), file) else {
                    self.leave_out(
                        &file.name,
                        &span,
                        "an assignment to something that is not a plain name",
                        LIMITS,
                    );
                    return;
                };
                self.assign_path(file, &span, &path, &value);
            }
            _ => self.leave_out(&file.name, &span, "an assignment the converter does not read", LIMITS),
        }
    }

    /// `seat.data.window_rules = ...` and the other assignments of a shell file that have something to convert to.
    fn assign_path(&mut self, file: &SourceFile<'_>, span: &Span, path: &[String], value: &Lv) {
        let parts: Vec<&str> = path.iter().map(String::as_str).collect();
        match parts.as_slice() {
            ["seat", "data", "window_rules"] => self.set_rules(file, span, value),
            ["seat", "data", "workspace_sets"] => self.drop_workspace_sets(file, span, value),
            ["seat", "data", "alerts"] => match value.as_bool() {
                Some(flag) => self.alerts = Some(flag),
                None => {
                    let reason = format!("alerts is true or false, not {}", value.describe());
                    self.leave_out(&file.name, span, &reason, CONSTRUCTS);
                }
            },
            ["seat", "edge_tiling" | "drag_constraint"] => {
                let reason = format!("`{}` has no TypeScript home yet: {}", path.join("."), names::SNAPPING);
                self.leave_out(&file.name, span, &reason, DROPPED);
            }
            ["seat", "data", "prompts"] => self.leave_out(
                &file.name,
                span,
                "prompts are drawn by the compositor and have no switch",
                CONSTRUCTS,
            ),
            _ => {
                let reason = format!(
                    "`{}` is not read: a shell file converts only window_rules and alerts",
                    path.join(".")
                );
                self.leave_out(&file.name, span, &reason, CONSTRUCTS);
            }
        }
    }

    /// `seat.data.workspace_sets`: the specification has no sets, so each one is reported where it stands.
    fn drop_workspace_sets(&mut self, file: &SourceFile<'_>, span: &Span, value: &Lv) {
        let Some(items) = value.elements() else {
            self.leave_out(&file.name, span, "workspace_sets is a list of sets", CONSTRUCTS);
            return;
        };
        for item in items {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => format!("a workspace set has no TypeScript home yet: {}", names::WORKSPACE_SETS),
            };
            self.leave_out(&file.name, &item.span, &reason, DROPPED);
        }
    }

    // --- gnoblin.configure -------------------------------------------------------------------------------------

    fn configure(&mut self, file: &SourceFile<'_>, table: &Lv) {
        for field in table.fields().unwrap_or_default() {
            self.section(file, field);
        }
    }

    fn field_left_out(&mut self, file: &SourceFile<'_>, field: &LuaField, reason: &str) {
        self.leave_out(&file.name, &field.span, reason, CONSTRUCTS);
    }

    fn section(&mut self, file: &SourceFile<'_>, field: &LuaField) {
        match &field.value.kind {
            LuaKind::Failed(reason) => {
                self.leave_out(&file.name, &field.span, reason, LIMITS);
                return;
            }
            LuaKind::Nil => return,
            _ => {}
        }
        let key = field.key.as_str();
        if names::is_section(key) {
            let Some(settings) = field.value.fields() else {
                let reason = format!("`{key}` is a table of settings, not {}", field.value.describe());
                self.field_left_out(file, field, &reason);
                return;
            };
            for setting in settings {
                self.drop_setting(file, key, &setting.key, key, setting);
            }
            return;
        }
        match key {
            "workspaces" => self.section_workspaces(file, field),
            "autostart" => self.drop_autostart(file, field),
            "shortcuts" => self.section_shortcuts(file, field, KeySection::Shortcuts),
            "keybindings" => self.section_shortcuts(file, field, KeySection::Keybindings),
            "touchpad-gestures" => self.drop_touchpad_gestures(file, field),
            "animations" | "portals" => {
                let reason =
                    format!("`{key}` is carried in the document and read by nothing, so there is nothing to set");
                self.field_left_out(file, field, &reason);
            }
            "shell" => self.field_left_out(file, field, "`shell` is not a setting: the shell is `init.ts` now"),
            _ => {
                let reason = format!("`{key}` is not a section of the config document");
                self.field_left_out(file, field, &reason);
            }
        }
    }

    /// One setting of a section that has no TypeScript home yet, and every setting under it. `top` is the first key
    /// under the section, which decides why; `path` is where the setting is, for the message.
    fn drop_setting(&mut self, file: &SourceFile<'_>, section: &str, top: &str, path: &str, field: &LuaField) {
        let here = format!("{path}.{}", field.key);
        match &field.value.kind {
            LuaKind::Nil => {}
            LuaKind::Failed(reason) => self.leave_out(&file.name, &field.span, reason, LIMITS),
            LuaKind::Table(nested) => {
                for setting in nested {
                    self.drop_setting(file, section, top, &here, setting);
                }
            }
            LuaKind::List(items) if items.is_empty() => {}
            _ => {
                let reason = format!(
                    "`{here}` has no TypeScript home yet: {}",
                    names::follow_up(section, top)
                );
                self.leave_out(&file.name, &field.span, &reason, DROPPED);
            }
        }
    }

    /// `autostart`: the specification has no startup list, so every entry is reported where it stands. An entry with
    /// `enable = false` starts nothing in Lua, so nothing is lost by leaving it out.
    fn drop_autostart(&mut self, file: &SourceFile<'_>, field: &LuaField) {
        let entries: Vec<(&str, &Lv, &Span)> = if let Some(fields) = field.value.fields() {
            fields
                .iter()
                .map(|entry| (entry.key.as_str(), &entry.value, &entry.span))
                .collect()
        } else if let Some(items) = field.value.elements() {
            items.iter().map(|item| ("", item, &item.span)).collect()
        } else {
            self.field_left_out(file, field, "`autostart` is a table of entries, each with a command");
            return;
        };
        for (name, value, span) in entries {
            if let LuaKind::Failed(reason) = &value.kind {
                self.leave_out(&file.name, span, reason, LIMITS);
                continue;
            }
            let disabled = value.fields().is_some_and(|settings| {
                settings
                    .iter()
                    .any(|setting| setting.key == "enable" && setting.value.as_bool() == Some(false))
            });
            if disabled {
                self.notes.push(format!(
                    "{}:{}: the autostart entry \"{name}\" is disabled (enable = false) and was left out",
                    span.file, span.line
                ));
                continue;
            }
            let what = if name.is_empty() {
                "an autostart command".to_owned()
            } else {
                format!("`autostart.{name}`")
            };
            let reason = format!("{what} has no TypeScript home yet: {}", names::AUTOSTART);
            self.leave_out(&file.name, span, &reason, DROPPED);
        }
    }

    /// `touchpad-gestures`: a drawn path has no trigger in the specification, so each gesture is reported.
    fn drop_touchpad_gestures(&mut self, file: &SourceFile<'_>, field: &LuaField) {
        let Some(items) = field.value.elements() else {
            self.field_left_out(file, field, "`touchpad-gestures` is a list of gestures");
            return;
        };
        for item in items {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => format!(
                    "a touchpad gesture has no TypeScript home yet: {}",
                    names::TOUCHPAD_GESTURES
                ),
            };
            let pointer = if matches!(item.kind, LuaKind::Failed(_)) {
                LIMITS
            } else {
                DROPPED
            };
            self.leave_out(&file.name, &item.span, &reason, pointer);
        }
    }

    // --- workspaces --------------------------------------------------------------------------------------------

    fn section_workspaces(&mut self, file: &SourceFile<'_>, field: &LuaField) {
        let Some(items) = field.value.elements() else {
            self.field_left_out(file, field, "`workspaces` is a list of tables with an id and a name");
            return;
        };
        let mut out = Vec::with_capacity(items.len());
        let mut seen: Vec<String> = Vec::new();
        for item in items {
            out.push(self.workspace(item, &mut seen));
        }
        self.reserve(&file.name, Batch::Workspaces);
        self.workspaces = Some(out);
    }

    fn workspace(&mut self, item: &Lv, seen: &mut Vec<String>) -> Placed<WorkspaceDef> {
        let Some(fields) = item.fields() else {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a workspace is a table with an id and a name".to_owned(),
            };
            return Placed::Note(self.report(&item.span, &reason, CONSTRUCTS));
        };
        let mut name: Option<String> = None;
        let mut notes: Vec<Vec<String>> = Vec::new();
        let mut id: Option<String> = None;
        for field in fields {
            match (field.key.as_str(), &field.value.kind) {
                ("id", LuaKind::Str(text)) if !text.is_empty() => id = Some(text.clone()),
                ("name", LuaKind::Str(text)) => name = Some(text.clone()),
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => notes.push(self.report(&field.span, reason, LIMITS)),
                ("id" | "name", _) => {
                    let reason = format!("the {} of a workspace is a non-empty string", field.key);
                    notes.push(self.report(&field.span, &reason, CONSTRUCTS));
                }
                _ => {
                    let reason = format!("a workspace has an id and a name, not `{}`", field.key);
                    notes.push(self.report(&field.span, &reason, CONSTRUCTS));
                }
            }
        }
        let Some(id) = id else {
            return Placed::Note(self.report(&item.span, "the workspace has no id, so it is left out", CONSTRUCTS));
        };
        if seen.contains(&id) {
            let reason = format!("the id \"{id}\" is already declared, and an id is unique");
            return Placed::Note(self.report(&item.span, &reason, CONSTRUCTS));
        }
        seen.push(id.clone());
        Placed::Def(WorkspaceDef { id, name, notes })
    }

    // --- shortcuts ---------------------------------------------------------------------------------------------

    fn section_shortcuts(&mut self, file: &SourceFile<'_>, field: &LuaField, section: KeySection) {
        let Some(entries) = field.value.fields() else {
            let reason = format!("`{}` is a table of shortcuts by name", section.name());
            self.field_left_out(file, field, &reason);
            return;
        };
        for entry in entries {
            self.shortcut(file, entry, section);
        }
        if !entries.is_empty() {
            self.reserve(&file.name, Batch::Keys);
        }
    }

    fn shortcut(&mut self, file: &SourceFile<'_>, entry: &LuaField, section: KeySection) {
        if entry.key.is_empty() || matches!(entry.value.kind, LuaKind::Failed(_)) {
            let reason = match &entry.value.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a shortcut needs a name".to_owned(),
            };
            self.leave_out(&file.name, &entry.span, &reason, LIMITS);
            return;
        }
        let Some(fields) = entry.value.fields() else {
            let reason = match section {
                KeySection::Shortcuts => "a shortcut is a table with a binding and a command",
                KeySection::Keybindings => {
                    "a keybindings entry that is a Lua function or a compositor action has no equivalent: \
                     bind the key with `input.bind` and a function"
                }
            };
            self.field_left_out(file, entry, reason);
            return;
        };
        let at = match self
            .shortcuts
            .iter()
            .position(|def| def.section == section && def.name == entry.key)
        {
            Some(at) => at,
            None => {
                self.shortcuts.push(ShortcutDef {
                    section,
                    name: entry.key.clone(),
                    span: entry.span.clone(),
                    binding: None,
                    command: None,
                    release: false,
                    enabled: true,
                });
                self.shortcuts.len() - 1
            }
        };
        self.shortcuts[at].span = entry.span.clone();
        for setting in fields {
            match (setting.key.as_str(), &setting.value.kind) {
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => self.leave_out(&file.name, &setting.span, reason, LIMITS),
                ("binding", LuaKind::Str(chord)) => {
                    self.shortcuts[at].binding = Some((chord.clone(), setting.span.clone()));
                }
                ("command", _) => {
                    if let Some(argv) = self.shortcut_command(file, &setting.value) {
                        self.shortcuts[at].command = Some(argv);
                    }
                }
                ("trigger", LuaKind::Str(edge)) if edge == "press" || edge == "release" => {
                    self.shortcuts[at].release = edge == "release";
                }
                ("enable", LuaKind::Bool(flag)) => self.shortcuts[at].enabled = *flag,
                (key @ ("binding" | "trigger" | "enable"), _) => {
                    let reason = format!("`{key}` is not the kind of value a shortcut takes");
                    self.field_left_out(file, setting, &reason);
                }
                (key, _) => {
                    let reason = format!("`{key}` is not read: a shortcut has binding, command, trigger and enable");
                    self.field_left_out(file, setting, &reason);
                }
            }
        }
    }

    /// The argv of a shortcut's command: a list of strings, or one word.
    fn shortcut_command(&mut self, file: &SourceFile<'_>, value: &Lv) -> Option<Vec<String>> {
        match &value.kind {
            LuaKind::Str(word) if !word.is_empty() && !word.contains(char::is_whitespace) => Some(vec![word.clone()]),
            LuaKind::Str(_) => {
                self.leave_out(
                    &file.name,
                    &value.span,
                    "a command line with spaces is split by the shell, not by Gnoblin: write the argv as a list",
                    CONSTRUCTS,
                );
                None
            }
            LuaKind::List(items) => {
                let argv: Option<Vec<String>> = items.iter().map(|item| item.as_str().map(str::to_owned)).collect();
                match argv {
                    Some(argv) if argv.first().is_some_and(|program| !program.is_empty()) => Some(argv),
                    _ => {
                        self.leave_out(
                            &file.name,
                            &value.span,
                            "a command is a list of strings, the program first",
                            CONSTRUCTS,
                        );
                        None
                    }
                }
            }
            _ => {
                let reason = format!(
                    "a command is a list of strings or a command line, not {}",
                    value.describe()
                );
                self.leave_out(&file.name, &value.span, &reason, CONSTRUCTS);
                None
            }
        }
    }

    // --- window rules ------------------------------------------------------------------------------------------

    fn set_rules(&mut self, file: &SourceFile<'_>, span: &Span, value: &Lv) {
        let Some(items) = value.elements() else {
            self.leave_out(&file.name, span, "window_rules is a list of rules", CONSTRUCTS);
            return;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(self.rule(item));
        }
        self.reserve(&file.name, Batch::Rules);
        self.rules = Some(out);
    }

    fn rule(&mut self, item: &Lv) -> Placed<RuleDraft> {
        let Some(fields) = item.fields() else {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a rule is a table".to_owned(),
            };
            return Placed::Note(self.report(&item.span, &reason, CONSTRUCTS));
        };
        let mut conditions: Vec<(&'static str, Test)> = Vec::new();
        let mut workspace: Option<(String, Span)> = None;
        let mut maximised: Option<bool> = None;
        let mut fullscreen: Option<bool> = None;
        let mut opacity: Option<f64> = None;
        let mut has_query = false;
        let mut notes: Vec<Vec<String>> = Vec::new();
        for field in fields {
            match (field.key.as_str(), &field.value.kind) {
                (_, LuaKind::Nil) => {}
                ("match", _) => match self.rule_match(item, field, &mut conditions) {
                    Ok(found) => has_query = found,
                    Err(note) => return Placed::Note(note),
                },
                (_, LuaKind::Failed(reason)) => notes.push(self.report(&field.span, reason, LIMITS)),
                ("workspace", LuaKind::Str(id)) if !id.is_empty() => {
                    workspace = Some((id.clone(), field.span.clone()));
                }
                ("workspace", LuaKind::Table(_)) => {
                    match field.value.fields().and_then(|f| f.iter().find(|x| x.key == "id")) {
                        Some(LuaField {
                            value:
                                Lv {
                                    kind: LuaKind::Str(id), ..
                                },
                            ..
                        }) if !id.is_empty() => workspace = Some((id.clone(), field.span.clone())),
                        _ => {
                            let reason = "workspace is a table with the id of a workspace";
                            notes.push(self.report(&field.span, reason, CONSTRUCTS));
                        }
                    }
                }
                ("maximised", LuaKind::Bool(flag)) => maximised = Some(*flag),
                ("fullscreen", LuaKind::Bool(flag)) => fullscreen = Some(*flag),
                ("secure", LuaKind::Bool(_)) => {
                    let reason = format!("`secure` has no TypeScript home yet: {}", names::RULE_FIELD);
                    notes.push(self.report(&field.span, &reason, DROPPED));
                }
                ("opacity", LuaKind::Int(_) | LuaKind::Float(_)) => {
                    let number = match field.value.kind {
                        LuaKind::Int(number) => number as f64,
                        LuaKind::Float(number) => number,
                        _ => 0.0,
                    };
                    if (0.0..=1.0).contains(&number) {
                        opacity = Some(number);
                    } else {
                        notes.push(self.report(&field.span, "opacity is from 0 to 1", CONSTRUCTS));
                    }
                }
                (key, _) => {
                    let reason = if matches!(key, "maximised" | "fullscreen" | "secure" | "workspace" | "opacity") {
                        format!("`{key}` is not the kind of value a rule takes")
                    } else {
                        format!("the rule field `{key}` has no equivalent")
                    };
                    notes.push(self.report(&field.span, &reason, CONSTRUCTS));
                }
            }
        }
        if !has_query {
            let note = self.report(
                &item.span,
                "the rule has no match the converter could carry over",
                CONSTRUCTS,
            );
            return Placed::Note(note);
        }
        if workspace.is_none() && maximised.is_none() && fullscreen.is_none() && opacity.is_none() {
            let note = self.report(
                &item.span,
                "the rule sets nothing the converter could carry over",
                CONSTRUCTS,
            );
            return Placed::Note(note);
        }
        let (workspace, workspace_span) = match workspace {
            Some((id, span)) => (Some(id), Some(span)),
            None => (None, None),
        };
        Placed::Def(RuleDraft {
            def: RuleDef {
                origin: format!("{}:{}", item.span.file, item.span.line),
                conditions,
                workspace,
                maximised,
                fullscreen,
                opacity,
                notes,
            },
            line: item.span.line,
            workspace_span,
        })
    }

    /// The query of a rule. A key it cannot carry over leaves the whole rule out: a rule with a looser match would act
    /// on windows the author never named. `Ok` says whether the query holds a key; `Err` is the comment for the rule.
    fn rule_match(
        &mut self,
        rule: &Lv,
        field: &LuaField,
        conditions: &mut Vec<(&'static str, Test)>,
    ) -> Result<bool, Vec<String>> {
        let Some(keys) = field.value.fields() else {
            let why = match &field.value.kind {
                LuaKind::Failed(why) => why.clone(),
                _ => "`match` is a table".to_owned(),
            };
            let reason = format!("the rule at line {} was left out: {why}", rule.span.line);
            return Err(self.report(&field.span, &reason, CONSTRUCTS));
        };
        let mut found = false;
        for key in keys {
            let left_out = |this: &mut Self, why: &str| {
                let reason = format!("the rule at line {} was left out: {why}", rule.span.line);
                this.report(&key.span, &reason, CONSTRUCTS)
            };
            match (key.key.as_str(), &key.value.kind) {
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => {
                    let reason = format!("the rule at line {} was left out: {reason}", rule.span.line);
                    return Err(self.report(&key.span, &reason, LIMITS));
                }
                (name @ ("app_id" | "title"), LuaKind::Str(pattern)) => {
                    let target = if name == "app_id" { "appId" } else { "title" };
                    match translate_lua_pattern(pattern) {
                        PatternTranslation::Unmapped(why) => {
                            let why = format!("the pattern \"{pattern}\" cannot be translated exactly: {why}");
                            return Err(left_out(self, &why));
                        }
                        translated => {
                            found = true;
                            if let Some(test) = Test::of(&translated) {
                                conditions.push((target, test));
                            }
                        }
                    }
                }
                ("focused" | "workspace_id", _) => {
                    let why = format!(
                        "`match.{}` has to be checked again whenever the focus or the workspace changes, and a listener hears a window once",
                        key.key
                    );
                    return Err(left_out(self, &why));
                }
                (other, _) => {
                    let why = if matches!(other, "app_id" | "title") {
                        format!("`match.{other}` is not the kind of value a rule takes")
                    } else {
                        format!("the match key `{other}` has no equivalent")
                    };
                    return Err(left_out(self, &why));
                }
            }
        }
        Ok(found)
    }

    // --- the end -----------------------------------------------------------------------------------------------

    /// Works out the groups that depend on each other and returns the TypeScript and the report.
    pub(crate) fn finish(mut self, entry: &str) -> MigrationOutput {
        let mut counts = ConvertedCounts::default();
        let mut needs = Needs::default();
        let (workspace_lines, declared, own_workspaces) = self.finish_workspaces(&mut counts, &mut needs);
        let (key_lines, taken) = self.finish_keys(&mut counts, &mut needs);
        let rule_lines = self.finish_rules(&declared, &mut counts, &mut needs);
        if self.alerts.is_some() {
            counts.options += 1;
        }
        // The items that wait for the end (a duplicate trigger, a rule for an undeclared workspace) are listed where
        // they stand in the source, not after everything met on the way.
        let sources = &self.sources;
        self.unmapped.sort_by_key(|item| {
            let file = sources
                .iter()
                .position(|source| *source == item.file)
                .unwrap_or(usize::MAX);
            (file, item.line)
        });
        let stock = emit::stock(&mut needs, own_workspaces, self.alerts.unwrap_or(true), &taken);
        let body = self.body(workspace_lines, key_lines, rule_lines);
        let typescript = emit::document(
            &Head {
                entry,
                sources: &self.sources,
                unmapped: self.unmapped.len(),
            },
            &needs,
            stock,
            body,
        );
        MigrationOutput {
            typescript,
            report: MigrationReport {
                sources: self.sources,
                converted: counts,
                unmapped: self.unmapped,
                notes: self.notes,
            },
        }
    }

    /// What the Lua files said, as blocks of lines in the order it was met, a file's name over its first block. Notes
    /// that follow one another stay in one block, so a section left out whole reads as one list.
    fn body(&mut self, workspaces: Vec<String>, keys: Vec<String>, rules: Vec<String>) -> Vec<Vec<String>> {
        let mut workspaces = Some(workspaces);
        let mut keys = Some(keys);
        let mut rules = Some(rules);
        let mut blocks: Vec<Vec<String>> = Vec::new();
        let mut header: Option<String> = None;
        let mut after_note = false;
        for entry in std::mem::take(&mut self.entries) {
            let (lines, is_note) = match entry {
                Piece::Header(file) => {
                    header = Some(format!("// From {file}."));
                    continue;
                }
                Piece::Note(lines) => (emit::comment(&lines), true),
                Piece::Batch(Batch::Workspaces) => (workspaces.take().unwrap_or_default(), false),
                Piece::Batch(Batch::Keys) => (keys.take().unwrap_or_default(), false),
                Piece::Batch(Batch::Rules) => (rules.take().unwrap_or_default(), false),
            };
            if lines.is_empty() {
                continue;
            }
            match blocks.last_mut() {
                Some(last) if is_note && after_note && header.is_none() => last.extend(lines),
                _ => {
                    let mut block = Vec::with_capacity(lines.len() + 1);
                    block.extend(header.take());
                    block.extend(lines);
                    blocks.push(block);
                }
            }
            after_note = is_note;
        }
        blocks
    }

    /// The statements for the declared workspaces, the ids the config declares, and whether it declares any.
    fn finish_workspaces(
        &mut self,
        counts: &mut ConvertedCounts,
        needs: &mut Needs,
    ) -> (Vec<String>, Vec<String>, bool) {
        let items = self.workspaces.take().unwrap_or_default();
        let ids: Vec<String> = items
            .iter()
            .filter_map(|item| match item {
                Placed::Def(def) => Some(def.id.clone()),
                Placed::Note(_) => None,
            })
            .collect();
        counts.workspaces = ids.len();
        let lines = emit::workspaces(&items, needs);
        if ids.is_empty() {
            let stock = STOCK_WORKSPACES.iter().map(|id| (*id).to_owned()).collect();
            (lines, stock, false)
        } else {
            (lines, ids, true)
        }
    }

    /// The statements for the shortcuts, and the canonical triggers they bind.
    fn finish_keys(&mut self, counts: &mut ConvertedCounts, needs: &mut Needs) -> (Vec<String>, BTreeSet<String>) {
        let defs = std::mem::take(&mut self.shortcuts);
        let mut slots: Vec<Placed<KeyDef>> = Vec::new();
        let mut bound: Vec<(String, usize)> = Vec::new();
        let mut taken: BTreeSet<String> = BTreeSet::new();
        for def in defs {
            if !def.enabled {
                self.notes.push(format!(
                    "{}:{}: {} \"{}\" is disabled (enable = false) and was left out",
                    def.span.file,
                    def.span.line,
                    def.section.name(),
                    def.name
                ));
                continue;
            }
            let Some((chord, chord_span)) = def.binding.clone() else {
                let reason = format!("{} \"{}\" has no binding", def.section.name(), def.name);
                slots.push(Placed::Note(self.report(&def.span, &reason, CONSTRUCTS)));
                continue;
            };
            let Some(argv) = def.command.clone() else {
                let reason = format!(
                    "{} \"{}\" has no command the converter could carry over",
                    def.section.name(),
                    def.name
                );
                slots.push(Placed::Note(self.report(&def.span, &reason, CONSTRUCTS)));
                continue;
            };
            if def.release {
                let reason = "a shortcut on key release has no equivalent: a key acts when it is pressed";
                slots.push(Placed::Note(self.report(&def.span, reason, CONSTRUCTS)));
                continue;
            }
            let trigger = match translate_chord(&chord) {
                ChordTranslation::Trigger(trigger) => trigger,
                ChordTranslation::Unmapped(why) => {
                    let reason = format!("the chord \"{chord}\" has no exact trigger: {why}");
                    slots.push(Placed::Note(self.report(&chord_span, &reason, CONSTRUCTS)));
                    continue;
                }
            };
            let canonical = canonical_trigger(&trigger);
            if let Some((_, line)) = bound.iter().find(|(known, _)| *known == canonical) {
                let reason = format!("it binds {trigger}, which line {line} binds too, and a trigger is bound once");
                slots.push(Placed::Note(self.report(&chord_span, &reason, CONSTRUCTS)));
                continue;
            }
            bound.push((canonical.clone(), chord_span.line));
            taken.insert(canonical);
            slots.push(Placed::Def(KeyDef { trigger, argv }));
        }
        counts.keys = slots.iter().filter(|slot| matches!(slot, Placed::Def(_))).count();
        (emit::keys(&slots, needs), taken)
    }

    /// The statements for the rules that convert; a rule for a workspace the config does not declare is left out.
    fn finish_rules(&mut self, declared: &[String], counts: &mut ConvertedCounts, needs: &mut Needs) -> Vec<String> {
        let drafts = self.rules.take().unwrap_or_default();
        let mut slots: Vec<Placed<RuleDef>> = Vec::new();
        for draft in drafts {
            match draft {
                Placed::Note(lines) => slots.push(Placed::Note(lines)),
                Placed::Def(draft) => {
                    if let (Some(id), Some(span)) = (&draft.def.workspace, &draft.workspace_span)
                        && !declared.contains(id)
                    {
                        let reason = format!(
                            "the rule at line {} was left out: it moves windows to workspace \"{id}\", which the config does not declare",
                            draft.line
                        );
                        slots.push(Placed::Note(self.report(span, &reason, CONSTRUCTS)));
                        slots.extend(draft.def.notes.into_iter().map(Placed::Note));
                        continue;
                    }
                    counts.rules += 1;
                    slots.push(Placed::Def(draft.def));
                }
            }
        }
        emit::rules(&slots, needs)
    }
}

/// The comment lines that stand where `span` was: where it was, what it said, and why it was left out.
fn comment_lines(span: &Span, reason: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut text = span.text.lines();
    let first = text.next().unwrap_or_default().trim_end();
    lines.push(format!("UNMAPPED ({}:{}): {first}", span.file, span.line));
    let rest: Vec<&str> = text.collect();
    for line in rest.iter().take(MAX_COMMENT_LINES - 1) {
        lines.push((*line).trim_end().to_owned());
    }
    if rest.len() > MAX_COMMENT_LINES - 1 {
        lines.push(format!("... {} more lines", rest.len() - (MAX_COMMENT_LINES - 1)));
    }
    lines.push(format!("-> {reason}"));
    lines
}

/// The first line of a construct, cut to a length a report line can hold.
fn summary(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default().trim();
    let more = lines.next().is_some();
    let mut out: String = first.chars().take(MAX_SUMMARY).collect();
    if first.chars().count() > MAX_SUMMARY || more {
        out.push_str(" ...");
    }
    out
}

/// What a call that is not `configure` or `load` is, in a sentence.
fn call_reason(name: &str) -> &'static str {
    match name {
        "gnoblin.on" => {
            "an event handler runs code: write a `ClientListener` and attach it with `gnoblin.clients.addListener`"
        }
        "require" => "a required module is another file: convert it too and `import` it",
        n if n.starts_with("gnoblin.timers.") => "a timer runs code: use `gnoblin.after`",
        n if n.starts_with("gnoblin.process.") => "a process call runs code: use `gnoblin.spawn`",
        _ => "a call runs code when the config runs: port it by hand",
    }
}

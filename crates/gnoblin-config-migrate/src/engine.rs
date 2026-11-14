//! The converter's walk over the statements of a Lua config.
//!
//! Nothing here runs Lua. A top-level statement is read for what it says: `gnoblin.configure` with a table that can
//! be evaluated statically, `gnoblin.load` of the files beside it, a `local` or a global bound to a static value, and
//! in a shell file the `seat.data` assignments that have a `defineConfig` key. Every other statement is left out of the
//! output and written there as an `UNMAPPED` comment, and listed in the report with its line and a pointer into the
//! migration page. A value that is part of a table and cannot be evaluated is treated the same way, field by field,
//! so one computed setting does not cost the settings around it.

use std::collections::HashMap;
use std::rc::Rc;

use full_moon::ast::{Assignment, Block, FunctionCall, LocalAssignment, Stmt, Var};
use full_moon::node::Node;

use crate::chord::{ChordTranslation, translate_chord};
use crate::lua::{self, LuaField, LuaKind, LuaScope, Lv, SourceFile, Span};
use crate::names;
use crate::pattern::{PatternTranslation, translate_lua_pattern};
use crate::value::{self, COMMENT, Val};
use crate::{
    ConfigSourceLoader, ConvertedCounts, MigrateError, MigrateSource, MigrationOutput, MigrationReport, UnmappedItem,
};

/// Where the migration page says what each construct becomes.
const CONSTRUCTS: &str = "docs/migrating-from-lua.md#what-each-construct-becomes";

/// Where the migration page says what the converter cannot do.
const LIMITS: &str = "docs/migrating-from-lua.md#what-the-converter-cannot-do";

/// How many files a `gnoblin.load` may nest, the limit the Lua host had.
const MAX_LOAD_DEPTH: usize = 32;

/// How many lines of a left-out statement are copied into its comment.
const MAX_COMMENT_LINES: usize = 10;

/// The longest summary the report prints for a left-out construct.
const MAX_SUMMARY: usize = 52;

/// The keys of a rule in the order they are written.
const RULE_ORDER: &[&str] = &[
    "appId",
    "title",
    "focused",
    "onWorkspace",
    "workspace",
    "maximised",
    "fullscreen",
    "secure",
    "style",
];

/// The modifiers in the order a trigger is compared, so `Shift+Super+q` and `Super+Shift+q` are one trigger.
const MODIFIER_ORDER: &[&str] = &["Shift", "Ctrl", "Alt", "Meta", "Super", "Hyper"];

/// The workspaces the stock config declares, which a config that declares none keeps.
const STOCK_WORKSPACES: &[&str] = &["1", "2", "3", "4"];

/// A command to start: a command line the runtime splits on spaces, or the program and its arguments.
#[derive(Clone, Debug, PartialEq)]
enum StartCommand {
    Line(String),
    Argv(Vec<String>),
}

impl StartCommand {
    /// What identifies the command: the program and its arguments.
    fn identity(&self) -> Vec<String> {
        match self {
            StartCommand::Line(line) => line.split_whitespace().map(str::to_owned).collect(),
            StartCommand::Argv(argv) => argv.clone(),
        }
    }

    fn value(&self) -> Val {
        match self {
            StartCommand::Line(line) => Val::Str(line.clone()),
            StartCommand::Argv(argv)
                if argv.len() == 1 && !argv[0].is_empty() && !argv[0].contains(char::is_whitespace) =>
            {
                Val::Str(argv[0].clone())
            }
            StartCommand::Argv(argv) => Val::List(argv.iter().cloned().map(Val::Str).collect()),
        }
    }
}

/// One `autostart` entry: from the table keyed by name, or from the list.
struct AutostartItem {
    name: Option<String>,
    span: Span,
    command: Option<StartCommand>,
    enabled: bool,
}

/// What the `autostart` section has said so far. A list replaces a table and the other way round, and tables merge
/// by name, as the document's layers do.
enum AutostartState {
    Unset,
    List(Vec<AutostartItem>),
    Table(Vec<AutostartItem>),
}

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

/// One window rule that has been read.
struct RuleDef {
    span: Span,
    entries: Vec<(&'static str, Val)>,
    workspace: Option<(String, Span)>,
}

/// A rule of the list, or the comment that stands where one was left out.
enum RuleOut {
    Rule(RuleDef),
    Note(Vec<String>),
}

/// One workspace set that has been read.
struct SetDef {
    span: Span,
    name: String,
    entries: Vec<(String, Option<String>)>,
}

/// A set of the list, or the comment that stands where one was left out.
enum SetOut {
    Set(SetDef),
    Note(Vec<String>),
}

pub(crate) struct Engine<'a> {
    loader: &'a dyn ConfigSourceLoader,
    sources: Vec<String>,
    unmapped: Vec<UnmappedItem>,
    notes: Vec<String>,
    /// The entries of the `defineConfig` object, in the order they first appeared.
    root: Vec<(String, Val)>,
    /// The file the last `From ...` comment named.
    header: Option<Rc<str>>,
    globals: HashMap<String, Lv>,
    /// The files being read, outermost first, so a load that names one of them is refused.
    stack: Vec<String>,
    workspaces: Option<Vec<Val>>,
    autostart: AutostartState,
    shortcuts: Vec<ShortcutDef>,
    rules: Option<Vec<RuleOut>>,
    sets: Option<Vec<SetOut>>,
}

impl<'a> Engine<'a> {
    pub(crate) fn new(loader: &'a dyn ConfigSourceLoader) -> Self {
        Engine {
            loader,
            sources: Vec::new(),
            unmapped: Vec::new(),
            notes: Vec::new(),
            root: Vec::new(),
            header: None,
            globals: HashMap::new(),
            stack: Vec::new(),
            workspaces: None,
            autostart: AutostartState::Unset,
            shortcuts: Vec::new(),
            rules: None,
            sets: None,
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

    /// Lists `span` as left out and writes its comment among the entries of the config.
    fn leave_out(&mut self, file: &Rc<str>, span: &Span, reason: &str, pointer: &'static str) {
        let lines = self.report(span, reason, pointer);
        self.push_root(file, COMMENT, Val::Note(lines));
    }

    /// A value of a section as a `Val`: a comment for what cannot be converted.
    fn convert_value(&mut self, lv: &Lv) -> Val {
        match &lv.kind {
            LuaKind::Str(text) => Val::Str(text.clone()),
            LuaKind::Int(number) => Val::Int(*number),
            LuaKind::Float(number) => Val::Float(*number),
            LuaKind::Bool(flag) => Val::Bool(*flag),
            LuaKind::Nil => Val::Note(self.report(&lv.span, "nil leaves a hole in a list", CONSTRUCTS)),
            LuaKind::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.convert_value(item));
                }
                Val::List(out)
            }
            LuaKind::Table(fields) => {
                let mut entries = Vec::with_capacity(fields.len());
                for field in fields {
                    if let Some(entry) = self.field_entry(field) {
                        entries.push(entry);
                    }
                }
                Val::Map(entries)
            }
            LuaKind::Failed(reason) => Val::Note(self.report(&lv.span, reason, LIMITS)),
        }
    }

    /// One field of a section as an entry of the output, or `None` for a field set to `nil`.
    fn field_entry(&mut self, field: &LuaField) -> Option<(String, Val)> {
        match &field.value.kind {
            LuaKind::Failed(reason) => {
                let lines = self.report(&field.span, reason, LIMITS);
                Some((COMMENT.to_owned(), Val::Note(lines)))
            }
            LuaKind::Nil => None,
            _ => {
                let key = names::setting_name(&field.key);
                if key == field.key && field.key.bytes().any(|b| b.is_ascii_uppercase()) {
                    self.notes.push(format!(
                        "{}:{}: the key \"{}\" is written in lower case by defineConfig",
                        field.span.file, field.span.line, field.key
                    ));
                }
                Some((key, self.convert_value(&field.value)))
            }
        }
    }

    // --- The output object -------------------------------------------------------------------------------------

    /// Adds an entry to the object, ahead of it a comment naming the file when that is not the file of the last one.
    fn push_root(&mut self, file: &Rc<str>, key: &str, value: Val) {
        if self.header.as_ref() != Some(file) {
            self.header = Some(Rc::clone(file));
            self.root
                .push((COMMENT.to_owned(), Val::Note(vec![format!("From {file}.")])));
        }
        self.root.push((key.to_owned(), value));
    }

    /// Sets a key of the object: merged into the value it has, or replacing it, or added at the end.
    fn set_root(&mut self, file: &Rc<str>, key: &str, value: Val, merge: bool) {
        match self.root.iter().position(|(k, _)| k == key) {
            Some(at) if merge => {
                let existing = std::mem::replace(&mut self.root[at].1, Val::Bool(false));
                self.root[at].1 = value::merge(existing, value);
            }
            Some(at) => self.root[at].1 = value,
            None => self.push_root(file, key, value),
        }
    }

    /// Fixes where a key that is worked out at the end stands: where it first appeared.
    fn reserve(&mut self, file: &Rc<str>, key: &str) {
        if !self.root.iter().any(|(k, _)| k == key) {
            self.push_root(file, key, Val::List(Vec::new()));
        }
    }

    /// Puts the worked-out value of a reserved key in its place, or takes the place away when there is none.
    fn settle(&mut self, key: &str, value: Option<Val>) {
        let Some(at) = self.root.iter().position(|(k, _)| k == key) else {
            return;
        };
        match value {
            Some(value) => self.root[at].1 = value,
            None => {
                self.root.remove(at);
            }
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
                "a key bound to a Lua function: write a function action in `keys`"
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

    /// `seat.data.window_rules = ...` and the other assignments of a shell file that have a `defineConfig` key.
    fn assign_path(&mut self, file: &SourceFile<'_>, span: &Span, path: &[String], value: &Lv) {
        let parts: Vec<&str> = path.iter().map(String::as_str).collect();
        match parts.as_slice() {
            ["seat", "data", "window_rules"] => self.set_rules(file, span, value),
            ["seat", "data", "workspace_sets"] => self.set_workspace_sets(file, span, value),
            ["seat", "data", "alerts"] => self.set_flag(file, span, "alerts", value),
            ["seat", "edge_tiling"] => self.set_flag(file, span, "edgeTiling", value),
            ["seat", "drag_constraint"] => match value.as_str() {
                Some(mode @ ("work_area" | "minimum_strip")) => {
                    self.set_root(&file.name, "dragConstraint", Val::Str(mode.to_owned()), false);
                }
                _ => self.leave_out(
                    &file.name,
                    span,
                    "drag_constraint is work_area or minimum_strip",
                    CONSTRUCTS,
                ),
            },
            ["seat", "data", "prompts"] => self.leave_out(
                &file.name,
                span,
                "prompts are drawn by the compositor and have no switch",
                CONSTRUCTS,
            ),
            _ => {
                let reason = format!("`{}` has no `defineConfig` key", path.join("."));
                self.leave_out(&file.name, span, &reason, CONSTRUCTS);
            }
        }
    }

    fn set_flag(&mut self, file: &SourceFile<'_>, span: &Span, key: &str, value: &Lv) {
        match value.as_bool() {
            Some(flag) => self.set_root(&file.name, key, Val::Bool(flag), false),
            None => {
                let reason = format!("{key} is true or false, not {}", value.describe());
                self.leave_out(&file.name, span, &reason, CONSTRUCTS);
            }
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
        if let Some(ts_key) = names::section_key(key) {
            if field.value.fields().is_none() {
                let reason = format!("`{key}` is a table of settings, not {}", field.value.describe());
                self.field_left_out(file, field, &reason);
                return;
            }
            let value = self.convert_value(&field.value);
            self.set_root(&file.name, ts_key, value, true);
            return;
        }
        match key {
            "workspaces" => self.section_workspaces(file, field),
            "autostart" => self.section_autostart(file, field),
            "shortcuts" => self.section_shortcuts(file, field, KeySection::Shortcuts),
            "keybindings" => self.section_shortcuts(file, field, KeySection::Keybindings),
            "touchpad-gestures" => {
                if field.value.elements().is_none() {
                    self.field_left_out(file, field, "`touchpad-gestures` is a list of gestures");
                    return;
                }
                let value = self.convert_value(&field.value);
                self.set_root(&file.name, "touchpadGestures", value, false);
            }
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
        self.reserve(&file.name, "workspaces");
        self.workspaces = Some(out);
    }

    fn workspace(&mut self, item: &Lv, seen: &mut Vec<String>) -> Val {
        let Some(fields) = item.fields() else {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a workspace is a table with an id and a name".to_owned(),
            };
            return Val::Note(self.report(&item.span, &reason, CONSTRUCTS));
        };
        let mut entries: Vec<(String, Val)> = Vec::new();
        let mut id: Option<String> = None;
        for field in fields {
            match (field.key.as_str(), &field.value.kind) {
                ("id", LuaKind::Str(text)) if !text.is_empty() => id = Some(text.clone()),
                ("name", LuaKind::Str(text)) => entries.push(("name".to_owned(), Val::Str(text.clone()))),
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => {
                    let lines = self.report(&field.span, reason, LIMITS);
                    entries.push((COMMENT.to_owned(), Val::Note(lines)));
                }
                ("id" | "name", _) => {
                    let reason = format!("the {} of a workspace is a non-empty string", field.key);
                    let lines = self.report(&field.span, &reason, CONSTRUCTS);
                    entries.push((COMMENT.to_owned(), Val::Note(lines)));
                }
                _ => {
                    let reason = format!("a workspace has an id and a name, not `{}`", field.key);
                    let lines = self.report(&field.span, &reason, CONSTRUCTS);
                    entries.push((COMMENT.to_owned(), Val::Note(lines)));
                }
            }
        }
        let Some(id) = id else {
            return Val::Note(self.report(&item.span, "the workspace has no id, so it is left out", CONSTRUCTS));
        };
        if seen.contains(&id) {
            let reason = format!("the id \"{id}\" is already declared, and an id is unique");
            return Val::Note(self.report(&item.span, &reason, CONSTRUCTS));
        }
        seen.push(id.clone());
        entries.insert(0, ("id".to_owned(), Val::Str(id)));
        Val::Map(entries)
    }

    // --- autostart ---------------------------------------------------------------------------------------------

    fn section_autostart(&mut self, file: &SourceFile<'_>, field: &LuaField) {
        let Some(fields) = field.value.fields() else {
            // A list of commands.
            match field.value.elements() {
                Some(items) => self.autostart_list(file, items),
                None => self.field_left_out(file, field, "`autostart` is a table of entries, each with a command"),
            }
            return;
        };
        if fields.is_empty() {
            return;
        }
        let mut table = match std::mem::replace(&mut self.autostart, AutostartState::Unset) {
            AutostartState::Table(items) => items,
            _ => Vec::new(),
        };
        for entry in fields {
            self.autostart_entry(file, entry, &mut table);
        }
        self.autostart = AutostartState::Table(table);
        self.reserve(&file.name, "autostart");
    }

    fn autostart_list(&mut self, file: &SourceFile<'_>, items: &[Lv]) {
        let mut list = Vec::new();
        for item in items {
            let command = match &item.kind {
                LuaKind::Str(line) if !line.trim().is_empty() => Some(StartCommand::Line(line.clone())),
                LuaKind::Failed(reason) => {
                    self.leave_out(&file.name, &item.span, reason, LIMITS);
                    None
                }
                _ => self.command_of(file, item),
            };
            if let Some(command) = command {
                list.push(AutostartItem {
                    name: None,
                    span: item.span.clone(),
                    command: Some(command),
                    enabled: true,
                });
            }
        }
        self.autostart = AutostartState::List(list);
        self.reserve(&file.name, "autostart");
    }

    fn autostart_entry(&mut self, file: &SourceFile<'_>, entry: &LuaField, table: &mut Vec<AutostartItem>) {
        if entry.key.is_empty() || matches!(entry.value.kind, LuaKind::Failed(_)) {
            let reason = match &entry.value.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "an autostart entry needs a name".to_owned(),
            };
            self.leave_out(&file.name, &entry.span, &reason, LIMITS);
            return;
        }
        let Some(fields) = entry.value.fields() else {
            self.field_left_out(file, entry, "an autostart entry is a table with a command");
            return;
        };
        let at = match table
            .iter()
            .position(|item| item.name.as_deref() == Some(entry.key.as_str()))
        {
            Some(at) => at,
            None => {
                table.push(AutostartItem {
                    name: Some(entry.key.clone()),
                    span: entry.span.clone(),
                    command: None,
                    enabled: true,
                });
                table.len() - 1
            }
        };
        table[at].span = entry.span.clone();
        for setting in fields {
            match (setting.key.as_str(), &setting.value.kind) {
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => self.leave_out(&file.name, &setting.span, reason, LIMITS),
                ("command", _) => {
                    if let Some(command) = self.command_of(file, &setting.value) {
                        table[at].command = Some(command);
                    }
                }
                ("enable", LuaKind::Bool(flag)) => table[at].enabled = *flag,
                ("when", LuaKind::Str(when)) if when == "on_login" => {}
                (key, _) => {
                    let reason = format!("`{key}` is not read: an autostart entry has a command and `enable`");
                    self.field_left_out(file, setting, &reason);
                }
            }
        }
    }

    /// A command given as an argv, or as one command line.
    fn command_of(&mut self, file: &SourceFile<'_>, value: &Lv) -> Option<StartCommand> {
        match &value.kind {
            LuaKind::Str(line) if !line.trim().is_empty() => Some(StartCommand::Line(line.clone())),
            LuaKind::List(items) => {
                let argv: Option<Vec<String>> = items.iter().map(|item| item.as_str().map(str::to_owned)).collect();
                match argv {
                    Some(argv) if argv.first().is_some_and(|program| !program.is_empty()) => {
                        Some(StartCommand::Argv(argv))
                    }
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
            self.reserve(&file.name, "keys");
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
                     write a function action in `keys`"
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
            _ => match self.command_of(file, value) {
                Some(StartCommand::Argv(argv)) => Some(argv),
                _ => None,
            },
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
        self.reserve(&file.name, "rules");
        self.rules = Some(out);
    }

    fn rule(&mut self, item: &Lv) -> RuleOut {
        let Some(fields) = item.fields() else {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a rule is a table".to_owned(),
            };
            return RuleOut::Note(self.report(&item.span, &reason, CONSTRUCTS));
        };
        let mut entries: Vec<(&'static str, Val)> = Vec::new();
        let mut workspace: Option<(String, Span)> = None;
        let mut has_query = false;
        for field in fields {
            match (field.key.as_str(), &field.value.kind) {
                (_, LuaKind::Nil) => {}
                ("match", _) => match self.rule_match(item, field, &mut entries) {
                    Ok(found) => has_query = found,
                    Err(note) => return RuleOut::Note(note),
                },
                (_, LuaKind::Failed(reason)) => {
                    self.report(&field.span, reason, LIMITS);
                }
                ("workspace", LuaKind::Str(id)) if !id.is_empty() => {
                    entries.push(("workspace", Val::Str(id.clone())));
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
                        }) if !id.is_empty() => {
                            entries.push(("workspace", Val::Str(id.clone())));
                            workspace = Some((id.clone(), field.span.clone()));
                        }
                        _ => self.field_dropped(field, "workspace is a table with the id of a workspace"),
                    }
                }
                (key @ ("maximised" | "fullscreen" | "secure"), LuaKind::Bool(flag)) => {
                    let key = static_key(key);
                    entries.push((key, Val::Bool(*flag)));
                }
                ("opacity", LuaKind::Int(_) | LuaKind::Float(_)) => {
                    let opacity = match field.value.kind {
                        LuaKind::Int(number) => number as f64,
                        LuaKind::Float(number) => number,
                        _ => 0.0,
                    };
                    if (0.0..=1.0).contains(&opacity) {
                        let value = if opacity.fract() == 0.0 {
                            Val::Int(opacity as i64)
                        } else {
                            Val::Float(opacity)
                        };
                        entries.push(("style", Val::Map(vec![("opacity".to_owned(), value)])));
                    } else {
                        self.field_dropped(field, "opacity is from 0 to 1");
                    }
                }
                (key, _) => {
                    let reason = if matches!(key, "maximised" | "fullscreen" | "secure" | "workspace" | "opacity") {
                        format!("`{key}` is not the kind of value a rule takes")
                    } else {
                        format!("the rule field `{key}` has no equivalent")
                    };
                    self.field_dropped(field, &reason);
                }
            }
        }
        if !has_query {
            let note = self.report(
                &item.span,
                "the rule has no match the converter could carry over",
                CONSTRUCTS,
            );
            return RuleOut::Note(note);
        }
        if !entries
            .iter()
            .any(|(key, _)| !matches!(*key, "appId" | "title" | "focused" | "onWorkspace"))
        {
            let note = self.report(
                &item.span,
                "the rule sets nothing the converter could carry over",
                CONSTRUCTS,
            );
            return RuleOut::Note(note);
        }
        entries.sort_by_key(|(key, _)| RULE_ORDER.iter().position(|ordered| ordered == key));
        RuleOut::Rule(RuleDef {
            span: item.span.clone(),
            entries,
            workspace,
        })
    }

    /// Lists a rule field as left out. The rule keeps its other fields: a field of an action only narrows the rule.
    fn field_dropped(&mut self, field: &LuaField, reason: &str) {
        self.report(&field.span, reason, CONSTRUCTS);
    }

    /// The query of a rule. A key it cannot carry over leaves the whole rule out: a rule with a looser match would act
    /// on windows the author never named. `Ok` says whether the query holds a key; `Err` is the comment for the rule.
    fn rule_match(
        &mut self,
        rule: &Lv,
        field: &LuaField,
        entries: &mut Vec<(&'static str, Val)>,
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
                            if let Some(code) = translated.typescript() {
                                entries.push((target, Val::Raw(code)));
                                found = true;
                            }
                        }
                    }
                }
                ("focused", LuaKind::Bool(flag)) => {
                    entries.push(("focused", Val::Bool(*flag)));
                    found = true;
                }
                ("workspace_id", LuaKind::Str(id)) if !id.is_empty() => {
                    entries.push(("onWorkspace", Val::Str(id.clone())));
                    found = true;
                }
                (other, _) => {
                    let why = if matches!(other, "app_id" | "title" | "focused" | "workspace_id") {
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

    // --- workspace sets ----------------------------------------------------------------------------------------

    fn set_workspace_sets(&mut self, file: &SourceFile<'_>, span: &Span, value: &Lv) {
        let Some(items) = value.elements() else {
            self.leave_out(&file.name, span, "workspace_sets is a list of sets", CONSTRUCTS);
            return;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(self.workspace_set(item));
        }
        self.reserve(&file.name, "workspaceSets");
        self.sets = Some(out);
    }

    fn workspace_set(&mut self, item: &Lv) -> SetOut {
        let Some(fields) = item.fields() else {
            let reason = match &item.kind {
                LuaKind::Failed(reason) => reason.clone(),
                _ => "a workspace set is a table with a name and entries".to_owned(),
            };
            return SetOut::Note(self.report(&item.span, &reason, CONSTRUCTS));
        };
        let mut name: Option<String> = None;
        let mut entries: Vec<(String, Option<String>)> = Vec::new();
        for field in fields {
            match (field.key.as_str(), &field.value.kind) {
                ("name", LuaKind::Str(text)) => name = Some(text.clone()),
                ("entries", _) => match self.set_entries(&field.value) {
                    Ok(list) => entries = list,
                    Err(why) => {
                        let reason = format!("the set at line {} was left out: {why}", item.span.line);
                        return SetOut::Note(self.report(&field.span, &reason, CONSTRUCTS));
                    }
                },
                (_, LuaKind::Nil) => {}
                (_, LuaKind::Failed(reason)) => {
                    let reason = format!("the set at line {} was left out: {reason}", item.span.line);
                    return SetOut::Note(self.report(&field.span, &reason, LIMITS));
                }
                (key, _) => {
                    let reason = format!(
                        "the set at line {} was left out: a set has a name and entries, not `{key}`",
                        item.span.line
                    );
                    return SetOut::Note(self.report(&field.span, &reason, CONSTRUCTS));
                }
            }
        }
        match name {
            Some(name) => SetOut::Set(SetDef {
                span: item.span.clone(),
                name,
                entries,
            }),
            None => SetOut::Note(self.report(&item.span, "the set has no name, so it is left out", CONSTRUCTS)),
        }
    }

    /// The `{ workspace, output? }` entries of a set.
    fn set_entries(&mut self, value: &Lv) -> Result<Vec<(String, Option<String>)>, String> {
        let Some(items) = value.elements() else {
            return Err("`entries` is a list".to_owned());
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Some(fields) = item.fields() else {
                return Err("an entry is a table with a workspace".to_owned());
            };
            let mut workspace: Option<String> = None;
            let mut output: Option<String> = None;
            for field in fields {
                match (field.key.as_str(), &field.value.kind) {
                    ("workspace", LuaKind::Str(id)) => workspace = Some(id.clone()),
                    ("workspace", LuaKind::Int(id)) => workspace = Some(id.to_string()),
                    ("output", LuaKind::Str(name)) => output = Some(name.clone()),
                    (_, LuaKind::Nil) => {}
                    (key, _) => {
                        return Err(format!(
                            "`{key}` is not an entry key: an entry has a workspace and an output"
                        ));
                    }
                }
            }
            match workspace {
                Some(workspace) => out.push((workspace, output)),
                None => return Err("an entry has no workspace".to_owned()),
            }
        }
        Ok(out)
    }

    // --- the end -----------------------------------------------------------------------------------------------

    /// Works out the keys that depend on each other and returns the TypeScript and the report.
    pub(crate) fn finish(mut self, entry: &str) -> MigrationOutput {
        let mut counts = ConvertedCounts::default();
        let declared = self.finish_workspaces(&mut counts);
        self.finish_autostart(&mut counts);
        let uses_spawn = self.finish_keys(&mut counts);
        self.finish_rules(&declared, &mut counts);
        self.finish_sets(&declared, &mut counts);
        for (key, value) in &self.root {
            if names::SECTIONS.iter().any(|(_, ts)| ts == key) {
                counts.settings += value::count_settings(value);
            } else if key == "touchpadGestures" {
                counts.settings += value.element_count();
            } else if matches!(key.as_str(), "alerts" | "edgeTiling" | "dragConstraint") {
                counts.options += 1;
            }
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
        let typescript = self.print(entry, uses_spawn);
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

    /// The workspace ids the converted config declares.
    fn finish_workspaces(&mut self, counts: &mut ConvertedCounts) -> Vec<String> {
        let items = self.workspaces.take().unwrap_or_default();
        let ids: Vec<String> = items
            .iter()
            .filter_map(|item| item.get("id").and_then(Val::as_str).map(str::to_owned))
            .collect();
        counts.workspaces = ids.len();
        self.settle(
            "workspaces",
            if items.is_empty() { None } else { Some(Val::List(items)) },
        );
        if ids.is_empty() {
            STOCK_WORKSPACES.iter().map(|id| (*id).to_owned()).collect()
        } else {
            ids
        }
    }

    fn finish_autostart(&mut self, counts: &mut ConvertedCounts) {
        let mut items = match std::mem::replace(&mut self.autostart, AutostartState::Unset) {
            AutostartState::Unset => Vec::new(),
            AutostartState::List(items) => items,
            // A table is read in name order, as the document holds it.
            AutostartState::Table(mut items) => {
                items.sort_by(|a, b| a.name.cmp(&b.name));
                items
            }
        };
        let mut seen: Vec<(Vec<String>, usize)> = Vec::new();
        let mut out: Vec<Val> = Vec::new();
        for item in items.drain(..) {
            if !item.enabled {
                continue;
            }
            let Some(command) = item.command else {
                let what = item.name.as_deref().unwrap_or("the entry");
                let reason = format!("{what} has no command, so nothing starts");
                out.push(Val::Note(self.report(&item.span, &reason, CONSTRUCTS)));
                continue;
            };
            let identity = command.identity();
            if let Some((_, line)) = seen.iter().find(|(known, _)| *known == identity) {
                let reason = format!("it starts the same command as line {line}, and a command is listed once");
                out.push(Val::Note(self.report(&item.span, &reason, CONSTRUCTS)));
                continue;
            }
            seen.push((identity, item.span.line));
            out.push(command.value());
        }
        counts.autostart = seen.len();
        self.settle("autostart", if out.is_empty() { None } else { Some(Val::List(out)) });
    }

    /// Writes the shortcuts as `keys`. Returns whether any of them calls `spawn`.
    fn finish_keys(&mut self, counts: &mut ConvertedCounts) -> bool {
        let defs = std::mem::take(&mut self.shortcuts);
        let mut keys: Vec<(String, Val)> = Vec::new();
        let mut bound: Vec<(String, usize)> = Vec::new();
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
                keys.push((
                    COMMENT.to_owned(),
                    Val::Note(self.report(&def.span, &reason, CONSTRUCTS)),
                ));
                continue;
            };
            let Some(argv) = def.command.clone() else {
                let reason = format!(
                    "{} \"{}\" has no command the converter could carry over",
                    def.section.name(),
                    def.name
                );
                keys.push((
                    COMMENT.to_owned(),
                    Val::Note(self.report(&def.span, &reason, CONSTRUCTS)),
                ));
                continue;
            };
            if def.release {
                let reason = "a shortcut on key release has no equivalent: a key acts when it is pressed";
                keys.push((
                    COMMENT.to_owned(),
                    Val::Note(self.report(&def.span, reason, CONSTRUCTS)),
                ));
                continue;
            }
            let trigger = match translate_chord(&chord) {
                ChordTranslation::Trigger(trigger) => trigger,
                ChordTranslation::Unmapped(why) => {
                    let reason = format!("the chord \"{chord}\" has no exact trigger: {why}");
                    keys.push((
                        COMMENT.to_owned(),
                        Val::Note(self.report(&chord_span, &reason, CONSTRUCTS)),
                    ));
                    continue;
                }
            };
            let canonical = canonical_trigger(&trigger);
            if let Some((_, line)) = bound.iter().find(|(known, _)| *known == canonical) {
                let reason = format!("it binds {trigger}, which line {line} binds too, and a trigger is bound once");
                keys.push((
                    COMMENT.to_owned(),
                    Val::Note(self.report(&chord_span, &reason, CONSTRUCTS)),
                ));
                continue;
            }
            bound.push((canonical, chord_span.line));
            let arguments: Vec<String> = argv.iter().map(|argument| names::string(argument)).collect();
            keys.push((trigger, Val::Raw(format!("spawn({})", arguments.join(", ")))));
        }
        let count = keys.iter().filter(|(key, _)| key != COMMENT).count();
        counts.keys = count;
        self.settle("keys", if keys.is_empty() { None } else { Some(Val::Map(keys)) });
        count > 0
    }

    fn finish_rules(&mut self, declared: &[String], counts: &mut ConvertedCounts) {
        let rules = self.rules.take().unwrap_or_default();
        let mut out: Vec<Val> = Vec::new();
        for rule in rules {
            match rule {
                RuleOut::Note(lines) => out.push(Val::Note(lines)),
                RuleOut::Rule(def) => {
                    if let Some((id, span)) = &def.workspace
                        && !declared.contains(id)
                    {
                        let reason = format!(
                            "the rule at line {} was left out: it moves windows to workspace \"{id}\", which the config does not declare",
                            def.span.line
                        );
                        out.push(Val::Note(self.report(span, &reason, CONSTRUCTS)));
                        continue;
                    }
                    counts.rules += 1;
                    let entries = def
                        .entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value))
                        .collect();
                    out.push(Val::Map(entries));
                }
            }
        }
        self.settle("rules", if out.is_empty() { None } else { Some(Val::List(out)) });
    }

    fn finish_sets(&mut self, declared: &[String], counts: &mut ConvertedCounts) {
        let sets = self.sets.take().unwrap_or_default();
        let mut out: Vec<Val> = Vec::new();
        let mut names_seen: Vec<String> = Vec::new();
        for set in sets {
            let def = match set {
                SetOut::Note(lines) => {
                    out.push(Val::Note(lines));
                    continue;
                }
                SetOut::Set(def) => def,
            };
            if let Some(why) = set_problem(&def, declared, &names_seen) {
                let reason = format!("the set \"{}\" was left out: {why}", def.name);
                out.push(Val::Note(self.report(&def.span, &reason, CONSTRUCTS)));
                continue;
            }
            names_seen.push(def.name.clone());
            counts.workspace_sets += 1;
            let entries = def
                .entries
                .iter()
                .map(|(workspace, output)| {
                    let mut entry = vec![("workspace".to_owned(), Val::Str(workspace.clone()))];
                    if let Some(output) = output {
                        entry.push(("output".to_owned(), Val::Str(output.clone())));
                    }
                    Val::Map(entry)
                })
                .collect();
            out.push(Val::Map(vec![
                ("name".to_owned(), Val::Str(def.name)),
                ("entries".to_owned(), Val::List(entries)),
            ]));
        }
        self.settle(
            "workspaceSets",
            if out.is_empty() { None } else { Some(Val::List(out)) },
        );
    }

    /// The TypeScript file.
    fn print(&self, entry: &str, uses_spawn: bool) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "// {entry} converted to TypeScript by `gnoblinctl config migrate`.\n"
        ));
        out.push_str("// Review it before you rely on it: the converter copies data and nothing else.\n");
        out.push_str(&format!("// Sources, in load order: {}.\n", self.sources.join(", ")));
        if !self.unmapped.is_empty() {
            out.push_str(&format!(
                "// {} construct(s) were not converted. Each is marked UNMAPPED where it stood, and the\n",
                self.unmapped.len()
            ));
            out.push_str("// command lists them with a pointer into docs/migrating-from-lua.md.\n");
        }
        if uses_spawn {
            out.push_str("import { defineConfig, spawn } from \"gnoblin\";\n\n");
        } else {
            out.push_str("import { defineConfig } from \"gnoblin\";\n\n");
        }
        out.push_str("export default defineConfig({\n");
        for (key, value) in &self.root {
            value::write_entry(&mut out, key, value, 1);
        }
        out.push_str("});\n");
        out
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
        "gnoblin.on" => "an event handler runs code: write `on: { clientAdded(client) {} }` or a `ClientListener`",
        "require" => "a required module is another file: convert it too and `import` it",
        n if n.starts_with("gnoblin.timers.") => "a timer runs code: use `gnoblin.after`",
        n if n.starts_with("gnoblin.process.") => "a process call runs code: use `gnoblin.spawn`",
        _ => "a call runs code when the config runs: port it by hand",
    }
}

/// The key of a rule as a static string, so an entry can hold it.
fn static_key(key: &str) -> &'static str {
    match key {
        "maximised" => "maximised",
        "fullscreen" => "fullscreen",
        _ => "secure",
    }
}

/// A trigger in the form two spellings of one trigger share: the modifiers in a fixed order, a one-letter key in lower
/// case.
fn canonical_trigger(trigger: &str) -> String {
    let mut parts: Vec<&str> = trigger.split('+').collect();
    let key = parts.pop().unwrap_or_default();
    parts.sort_by_key(|modifier| MODIFIER_ORDER.iter().position(|ordered| ordered == modifier));
    let key = if key.chars().count() == 1 {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    };
    parts.push(&key);
    parts.join("+")
}

/// Why a workspace set cannot stand, as `defineConfig` checks it.
fn set_problem(def: &SetDef, declared: &[String], names_seen: &[String]) -> Option<String> {
    if def.name.is_empty() {
        return Some("a set has a name".to_owned());
    }
    if names_seen.contains(&def.name) {
        return Some("a set name is used once".to_owned());
    }
    if def.entries.is_empty() {
        return Some("a set has at least one entry".to_owned());
    }
    let mut outputs: Vec<&str> = Vec::new();
    for (workspace, output) in &def.entries {
        if !declared.contains(workspace) {
            return Some(format!("workspace \"{workspace}\" is not declared by the config"));
        }
        if let Some(output) = output {
            if outputs.contains(&output.as_str()) {
                return Some(format!("output \"{output}\" is used twice"));
            }
            outputs.push(output);
        }
    }
    None
}

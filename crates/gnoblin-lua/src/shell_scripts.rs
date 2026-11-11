//! The scripts the shell VM loads after `node.lua`: the stock shell, then the user's shell files.
//!
//! The stock shell is the library at `<root>/gnoblin/shell` (`require("gnoblin.shell")`). It reads the host's
//! `sys` table when it loads, so [`install_sys`] runs first. The user's files come from the config directory
//! (the one `init.lua` lives in), in this order: `shell.lua`, then `shell/*.lua` by file name. Each runs once,
//! after the stock shell and before the first frame, so a file can change what the stock shell reads:
//! `seat.data.window_rules`, `seat.data.workspace_sets`, a class method, a key binding.
//!
//! # Failure modes first
//!
//! 1. **A user file has a syntax error or raises.** One error is logged, naming the file, and the next file
//!    still loads. What the file did before it raised stays. The stock shell is already running, so a broken
//!    user file never costs the session its shell (`tests::a_syntax_error_logs_one_error_and_the_stock_shell_and_other_files_still_run`).
//! 2. **A user file loops for ever.** The VM's own count hook stops the file when it has used its [`LoadBudget`]
//!    (instructions or time), and it is logged as an error like any other. The hook is the sandbox's, so it stays on
//!    for every handler the file defines.
//! 3. **A user file reaches for the host.** The VM's state is the configuration sandbox
//!    (`gnoblin-lua-sandbox`, `Profile::Config`): no `io`, `debug`, `package`, `dofile`, `loadfile` or binary
//!    `load`, and `os` cut to its clock half. Each file also runs in an environment of its own that names none of
//!    them. Its `require` reads `gnoblin.*` from the shipped library and any other name from the config directory,
//!    each confined to its root.
//! 4. **The stock shell does not load.** The error comes back to the caller, which leaves the VM off.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gnoblin_lua_sandbox::{Guard, Limits};
use mlua::{Function, Lua, Table, Value};
use rustix::process::{Pid, Signal, kill_process_group};

use crate::loader::{self, Resolution};
use crate::require::{ModuleRoots, is_module_name, read_source, resolve, runtime};

/// What loading may use: the stock shell as a whole, and each user file on its own. It is run under the VM's
/// guard, so the count hook the sandbox installed enforces it and stays installed. Every entry into Lua the load
/// makes runs through [`LoadBudget::run`], so none of it is left to the budget of whatever ran before.
pub(crate) struct LoadBudget<'a> {
    pub guard: &'a Guard,
    pub limits: Limits,
}

impl LoadBudget<'_> {
    /// Runs `run` as one entry with its own budget. A budget that runs out ends the entry with an error.
    fn run<T>(&self, run: impl FnOnce() -> mlua::Result<T>) -> mlua::Result<T> {
        self.guard.scoped(&self.limits, run).0
    }
}

/// The Lua that builds the environment a user file runs in. It takes the real globals and the user's
/// `require`, and returns the environment table.
const USER_ENV: &str = r#"
local G, user_require = ...
-- Names a user file cannot read: they reach the host. A write to one stays in the file's own environment.
local denied = {
    io = true, debug = true, package = true, dofile = true, loadfile = true, loadstring = true,
}
local clock = { time = G.os.time, date = G.os.date, clock = G.os.clock, difftime = G.os.difftime }
local env = {}
local function text_load(chunk, name, _, e)
    return G.load(chunk, name, "t", e or env)
end
setmetatable(env, {
    __metatable = false,
    __index = function(_, k)
        if k == "_G" then
            return env
        elseif k == "os" then
            return clock
        elseif k == "require" then
            return user_require
        elseif k == "load" then
            return text_load
        elseif denied[k] then
            return nil
        end
        return G[k]
    end,
    -- A global a file defines is the shell's global too, as it is in any Lua script: `function desk_name() end`.
    __newindex = function(_, k, v)
        if denied[k] or k == "os" or k == "require" or k == "load" then
            rawset(env, k, v)
        else
            G[k] = v
        end
    end,
})
return env
"#;

/// Installs the host's `sys` table when the VM has none: `spawn(command or argv)` and `date(format)`.
///
/// `spawn` starts a process in its own process group and returns `{ pid, kill = function(self, signal) end }`.
/// A string runs under `/bin/sh -c`; a table is the argument vector. The child's standard input is closed and
/// its output goes where the role's goes. A waiter thread reaps it.
pub(crate) fn install_sys(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    if globals.contains_key("sys")? {
        return Ok(());
    }
    let sys = lua.create_table()?;
    sys.set("spawn", lua.create_function(|lua, command: Value| spawn(lua, command))?)?;
    let date: Function = lua
        .load("local date = os.date; return function(format) return date(format) end")
        .eval()?;
    sys.set("date", date)?;
    globals.set("sys", sys)
}

fn spawn(lua: &Lua, command: Value) -> mlua::Result<Table> {
    let mut builder = match command {
        Value::String(text) => {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(text.to_str()?.to_string());
            command
        }
        Value::Table(argv) => {
            let argv: Vec<String> = argv.sequence_values::<String>().collect::<mlua::Result<_>>()?;
            let Some((program, args)) = argv.split_first() else {
                return Err(runtime("spawn: the argument vector is empty"));
            };
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        other => {
            return Err(runtime(format!(
                "spawn takes a command string or an argument table, not a {}",
                other.type_name()
            )));
        }
    };
    let mut child = {
        use std::os::unix::process::CommandExt;
        builder
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|error| runtime(format!("spawn: {error}")))?
    };
    let pid = child.id();
    let alive = Arc::new(AtomicBool::new(true));
    let waiter = Arc::clone(&alive);
    std::thread::spawn(move || {
        let _ = child.wait();
        waiter.store(false, Ordering::SeqCst);
    });
    let process = lua.create_table()?;
    process.set("pid", pid)?;
    process.set(
        "kill",
        lua.create_function(move |_, (_, signal): (Value, Option<String>)| {
            let signal = match signal.as_deref().unwrap_or("term") {
                "term" => Signal::TERM,
                "kill" => Signal::KILL,
                "int" => Signal::INT,
                "hup" => Signal::HUP,
                other => return Err(runtime(format!("kill: unknown signal {other:?}"))),
            };
            // The waiter clears the flag when the child is reaped, so a pid that may be reused is never signalled.
            if !alive.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let Some(group) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
                return Ok(false);
            };
            Ok(kill_process_group(group, signal).is_ok())
        })?,
    )?;
    Ok(process)
}

/// Loads the stock shell from `stock` (the directory holding `node.lua`), then the user's files from `user`.
///
/// Returns the errors the user's files raised, one text each, in load order; each was also logged. An error
/// loading the stock shell itself is returned as `Err`.
pub(crate) fn load(
    lua: &Lua,
    stock: &Path,
    user: Option<&Path>,
    budget: &LoadBudget<'_>,
) -> Result<Vec<String>, String> {
    let root = stock
        .parent()
        .ok_or_else(|| format!("{} has no parent directory to load gnoblin.shell from", stock.display()))?;
    budget
        .run(|| install_sys(lua))
        .map_err(|error| format!("installing sys: {error}"))?;
    budget
        .run(|| lua.load("require('gnoblin.shell')").set_name("gnoblin.shell").exec())
        .map_err(|error| format!("loading gnoblin.shell: {error}"))?;
    let Some(user) = user else {
        return Ok(Vec::new());
    };
    Ok(load_user_files(lua, root, user, budget))
}

/// The user's shell files under `dir`, in load order. A directory that is not there has none.
fn user_files(dir: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let root = match std::fs::canonicalize(dir) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((dir.to_path_buf(), Vec::new())),
        Err(error) => return Err(format!("cannot resolve {}: {error}", dir.display())),
    };
    let mut files = Vec::new();
    for pattern in ["shell.lua", "shell/*.lua"] {
        match loader::resolve(&root, &root, pattern)? {
            Resolution::Missing => {}
            Resolution::Files(found) => files.extend(found),
        }
    }
    Ok((root, files))
}

fn load_user_files(lua: &Lua, shipped: &Path, dir: &Path, budget: &LoadBudget<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let (root, files) = match user_files(dir) {
        Ok(found) => found,
        Err(error) => {
            crate::log::error!(area = "shell-vm", "the user's shell files were not loaded: {error}");
            return vec![error];
        }
    };
    if files.is_empty() {
        return errors;
    }
    let env = match budget.run(|| user_env(lua, shipped, &root)) {
        Ok(env) => env,
        Err(error) => {
            crate::log::error!(area = "shell-vm", "the user's shell files were not loaded: {error}");
            return vec![error.to_string()];
        }
    };
    for file in files {
        if let Err(error) = run_user_file(lua, &env, &file, budget) {
            crate::log::error!(area = "shell-vm", file = %file.display(), "user shell file failed: {error}");
            errors.push(format!("{}: {error}", file.display()));
        }
    }
    errors
}

/// Runs one file in `env`, stopped by the VM's count hook when it has used `budget`.
fn run_user_file(lua: &Lua, env: &Table, file: &Path, budget: &LoadBudget<'_>) -> mlua::Result<()> {
    let source = read_source(file, &file.display().to_string()).map_err(runtime)?;
    budget.run(|| {
        lua.load(source.as_str())
            .set_name(format!("@{}", file.display()))
            .set_environment(env.clone())
            .exec()
    })
}

/// The environment user files run in, with its own `require`.
fn user_env(lua: &Lua, shipped: &Path, user: &Path) -> mlua::Result<Table> {
    let roots = ModuleRoots {
        shipped: shipped.to_path_buf(),
        user: Some(user.to_path_buf()),
    };
    let real_require: Function = lua.globals().get("require")?;
    let loaded = lua.create_table()?;
    let loading = lua.create_table()?;
    let env_slot = lua.create_table()?;
    let user_require = {
        let loaded = loaded.clone();
        let loading = loading.clone();
        let env_slot = env_slot.clone();
        lua.create_function(move |lua, name: String| {
            if !is_module_name(&name) {
                return Err(runtime(format!("`{name}` is not a module name")));
            }
            // The shipped library is shared with the stock shell: one instance of each module.
            if name.starts_with("gnoblin.") {
                return real_require.call::<Value>(name);
            }
            if loading.get::<bool>(name.as_str()).unwrap_or(false) {
                return Err(runtime(format!("module `{name}` requires itself")));
            }
            let cached: Value = loaded.get(name.as_str())?;
            if !cached.is_nil() {
                return Ok(cached);
            }
            let resolved = resolve(&roots, &name).map_err(runtime)?;
            let source = read_source(&resolved.path, &name).map_err(runtime)?;
            let env: Table = env_slot.get("env")?;
            loading.set(name.as_str(), true)?; // a module that requires itself finds this marker
            let value = lua
                .load(source.as_str())
                .set_name(format!("@{}", resolved.path.display()))
                .set_environment(env)
                .call::<Value>(());
            let value = match value {
                Ok(Value::Nil) => Value::Boolean(true),
                Ok(value) => value,
                Err(error) => {
                    loading.set(name.as_str(), Value::Nil)?;
                    return Err(error);
                }
            };
            loading.set(name.as_str(), Value::Nil)?;
            loaded.set(name.as_str(), value.clone())?;
            Ok(value)
        })?
    };
    let build: Function = lua.load(USER_ENV).set_name("user environment").into_function()?;
    let env: Table = build.call((lua.globals(), user_require))?;
    env_slot.set("env", env.clone())?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::ops::Deref;
    use std::rc::Rc;
    use std::time::Duration;

    use gnoblin_lua_sandbox::Profile;

    use super::*;
    use crate::require::Modules;

    /// What loading may use in these tests: plenty, so only the test that loops runs into it.
    fn roomy() -> Limits {
        Limits {
            instructions: 400_000_000,
            memory_bytes: 64 * 1024 * 1024,
            handler_ticks: 5_000,
        }
    }

    /// A Lua state as the shell VM builds it: the config sandbox, its count hook, and a `require` that reads the
    /// stock library beside `stock`.
    struct State {
        lua: Lua,
        guard: Rc<Guard>,
        limits: Limits,
    }

    impl State {
        fn new(stock: &Path, limits: Limits) -> Self {
            let guard = Rc::new(Guard::new());
            guard.begin(&limits);
            let lua = gnoblin_lua_sandbox::create(Profile::Config, &limits, Rc::clone(&guard), Rc::new(|_, _| {}))
                .expect("the sandbox builds");
            let roots = ModuleRoots {
                shipped: stock.parent().expect("a parent").to_path_buf(),
                user: None,
            };
            crate::shell_vm::install_require(&lua, Modules::new(roots)).expect("require installs");
            Self { lua, guard, limits }
        }

        fn budget(&self) -> LoadBudget<'_> {
            LoadBudget {
                guard: &self.guard,
                limits: self.limits,
            }
        }
    }

    impl Deref for State {
        type Target = Lua;
        fn deref(&self) -> &Lua {
            &self.lua
        }
    }

    /// A stock library with a one-file shell, and a config directory beside it.
    struct Fixture {
        _dir: tempfile::TempDir,
        stock: PathBuf,
        config: PathBuf,
    }

    fn fixture(shell_init: &str) -> Fixture {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let stock = dir.path().join("lua/gnoblin");
        let config = dir.path().join("config");
        fs::create_dir_all(stock.join("shell")).expect("the stock directory");
        fs::create_dir_all(&config).expect("the config directory");
        fs::write(stock.join("shell/init.lua"), shell_init).expect("the stock shell");
        fs::write(stock.join("shell/rules.lua"), "return { list = {} }").expect("a stock module");
        Fixture {
            _dir: dir,
            stock,
            config,
        }
    }

    impl Fixture {
        fn write(&self, name: &str, text: &str) {
            let path = self.config.join(name);
            fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
            fs::write(path, text).expect("a user file");
        }

        fn state(&self) -> State {
            State::new(&self.stock, roomy())
        }

        fn load(&self, state: &State) -> Vec<String> {
            load(&state.lua, &self.stock, Some(&self.config), &state.budget()).expect("the stock shell loads")
        }
    }

    const STOCK: &str = "stock_loaded = true\nStockRules = require('gnoblin.shell.rules')\n";

    fn global<T: mlua::FromLua>(lua: &Lua, name: &str) -> T {
        lua.globals().get(name).expect("a global")
    }

    #[test]
    fn the_stock_shell_loads_before_the_user_files() {
        let f = fixture("order = { 'stock' }\n");
        f.write("shell.lua", "table.insert(order, 'shell.lua')");
        f.write("shell/20-b.lua", "table.insert(order, '20-b')");
        f.write("shell/10-a.lua", "table.insert(order, '10-a')");
        let lua = f.state();
        assert!(f.load(&lua).is_empty());
        let order: Table = global(&lua, "order");
        let order: Vec<String> = order.sequence_values().collect::<mlua::Result<_>>().expect("names");
        assert_eq!(order, ["stock", "shell.lua", "10-a", "20-b"]);
    }

    #[test]
    fn a_user_file_shares_globals_and_the_stock_modules_with_the_shell() {
        let f = fixture(STOCK);
        f.write(
            "shell.lua",
            "local Rules = require('gnoblin.shell.rules')\nRules.list = { 'mine' }\nmy_setting = 7\nseat_value = _G.stock_loaded",
        );
        let lua = f.state();
        assert!(f.load(&lua).is_empty());
        assert_eq!(
            global::<i64>(&lua, "my_setting"),
            7,
            "a file's global is the shell's global"
        );
        assert!(
            global::<bool>(&lua, "seat_value"),
            "a file reads the stock shell's globals"
        );
        let stock_rules: Table = global(&lua, "StockRules");
        let list: Table = stock_rules.get("list").expect("the list");
        assert_eq!(
            list.get::<String>(1).expect("the user's entry"),
            "mine",
            "require gives the file the stock shell's own module instance"
        );
    }

    #[test]
    fn a_syntax_error_logs_one_error_and_the_stock_shell_and_other_files_still_run() {
        let f = fixture(STOCK);
        f.write("shell/10-broken.lua", "this is not lua (");
        f.write("shell/20-fine.lua", "after_broken = true");
        // Tests in this crate log from many threads; tracing caches each call site's interest per process, so the
        // first capture can miss an event another thread's call registered first. Capture again until it is seen.
        let mut attempts = 0;
        let (lua, errors, logged) = loop {
            let lua = f.state();
            let mut errors = Vec::new();
            let logged = gnoblin_log::event::capture::logged(|| errors = f.load(&lua));
            attempts += 1;
            if !logged.is_empty() || attempts >= 50 {
                break (lua, errors, logged);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(logged.len(), 1, "exactly one event is logged: {logged:?}");
        assert!(
            format!("{} {}", logged[0].message, logged[0].fields_line()).contains("10-broken.lua"),
            "the error names the file: {:?}",
            logged[0]
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(global::<bool>(&lua, "stock_loaded"), "the stock shell is loaded");
        assert!(global::<bool>(&lua, "after_broken"), "the next file still loads");
    }

    #[test]
    fn a_file_that_raises_keeps_what_it_set_first() {
        let f = fixture(STOCK);
        f.write("shell.lua", "before = 1\nlocal x = nil + 1\nafter = 2");
        let lua = f.state();
        assert_eq!(f.load(&lua).len(), 1);
        assert_eq!(global::<i64>(&lua, "before"), 1);
        assert!(global::<Value>(&lua, "after").is_nil());
    }

    #[test]
    fn a_file_cannot_reach_the_host() {
        let f = fixture(STOCK);
        f.write(
            "shell.lua",
            r#"
            seen = {
                io = io, debug = debug, package = package, dofile = dofile, loadfile = loadfile,
                execute = os.execute, getenv = os.getenv, remove = os.remove, exit = os.exit,
                g_io = _G.io, g_os_execute = _G.os.execute,
                clock = type(os.time) .. type(os.date) .. type(os.clock) .. type(os.difftime),
            }
            seen.binary = load(string.dump(function() end))
            seen.text = load("return 1")()
            "#,
        );
        let lua = f.state();
        let errors = f.load(&lua);
        assert!(errors.is_empty(), "{errors:?}");
        let seen: Table = global(&lua, "seen");
        for name in [
            "io",
            "debug",
            "package",
            "dofile",
            "loadfile",
            "execute",
            "getenv",
            "remove",
            "exit",
            "g_io",
            "g_os_execute",
            "binary",
        ] {
            assert!(
                seen.get::<Value>(name).expect("a field").is_nil(),
                "{name} must be out of reach"
            );
        }
        assert_eq!(
            seen.get::<String>("clock").expect("clock"),
            "functionfunctionfunctionfunction"
        );
        assert_eq!(seen.get::<i64>("text").expect("text"), 1, "text chunks still load");
        // The VM's own state has none of them either: stock code and handlers a file defines run in it.
        for name in ["io", "debug", "package", "dofile", "loadfile"] {
            assert!(global::<Value>(&lua, name).is_nil(), "{name} is not in the shell VM");
        }
    }

    #[test]
    fn a_files_own_modules_load_from_the_config_directory_and_stay_inside_it() {
        let f = fixture(STOCK);
        f.write("mine/rules.lua", "return { rule = 'mine', no_io = io == nil }");
        f.write(
            "shell.lua",
            "mod = require('mine.rules')\nagain = require('mine.rules')",
        );
        f.write(
            "shell/50-escape.lua",
            "escaped = pcall(require, '../x')\nescaped_dot = pcall(require, 'mine..rules')",
        );
        f.write("shell/60-missing.lua", "require('nothing.here')");
        f.write("shell/70-cycle.lua", "require('cycle.a')");
        f.write("cycle/a.lua", "return require('cycle.b')");
        f.write("cycle/b.lua", "return require('cycle.a')");
        let lua = f.state();
        let errors = f.load(&lua);
        assert_eq!(errors.len(), 2, "the missing module and the cycle: {errors:?}");
        assert!(errors[0].contains("nothing.here"), "{errors:?}");
        assert!(errors[1].contains("requires itself"), "{errors:?}");
        let module: Table = global(&lua, "mod");
        assert_eq!(module.get::<String>("rule").expect("a field"), "mine");
        assert!(
            module.get::<bool>("no_io").expect("a field"),
            "a module runs in the file environment"
        );
        let again: Table = global(&lua, "again");
        assert_eq!(module, again, "a module loads once");
        assert!(!global::<bool>(&lua, "escaped"), "a name with .. is refused");
        assert!(!global::<bool>(&lua, "escaped_dot"), "an empty name part is refused");
    }

    #[test]
    fn a_file_that_never_ends_is_stopped_and_the_next_file_runs() {
        let f = fixture(STOCK);
        f.write("shell/10-loop.lua", "while true do end");
        f.write("shell/20-fine.lua", "after_loop = true");
        let limits = Limits {
            handler_ticks: 50,
            ..roomy()
        };
        let lua = State::new(&f.stock, limits);
        let errors = load(&lua.lua, &f.stock, Some(&f.config), &lua.budget()).expect("the stock shell loads");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("deadline"), "{errors:?}");
        assert!(global::<bool>(&lua, "after_loop"));
    }

    #[test]
    fn no_user_files_and_no_config_directory_load_nothing() {
        let f = fixture(STOCK);
        let lua = f.state();
        assert!(f.load(&lua).is_empty(), "an empty config directory");
        let lua = f.state();
        let absent = f.config.join("absent");
        assert!(
            load(&lua, &f.stock, Some(&absent), &lua.budget())
                .expect("loads")
                .is_empty()
        );
        let lua = f.state();
        assert!(load(&lua, &f.stock, None, &lua.budget()).expect("loads").is_empty());
    }

    #[test]
    fn a_stock_shell_that_does_not_load_is_an_error() {
        let f = fixture("error('the stock shell is broken')");
        f.write("shell.lua", "user_ran = true");
        let lua = f.state();
        let error = load(&lua, &f.stock, Some(&f.config), &lua.budget()).expect_err("the stock shell fails");
        assert!(error.contains("the stock shell is broken"), "{error}");
        assert!(
            global::<Value>(&lua, "user_ran").is_nil(),
            "no user file runs without the stock shell"
        );
    }

    #[test]
    fn sys_spawns_a_process_the_file_can_kill_and_formats_the_date() {
        let lua = Lua::new();
        install_sys(&lua).expect("sys installs");
        let (pid, killed): (u32, bool) = lua
            .load("local p = sys.spawn({ 'sleep', '30' }) return p.pid, p:kill()")
            .eval()
            .expect("spawn and kill");
        assert!(pid > 0);
        assert!(killed, "a live process takes the signal");
        let year: String = lua.load("return sys.date('%Y')").eval().expect("date");
        assert_eq!(year.len(), 4);
        let shell_pid: u32 = lua
            .load("return sys.spawn('exit 0').pid")
            .eval()
            .expect("a command string");
        assert!(shell_pid > 0);
        let refused = lua.load("return sys.spawn({})").eval::<Value>();
        assert!(refused.is_err(), "an empty argument vector is refused");
    }
}

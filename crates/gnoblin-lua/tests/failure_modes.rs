//! One test per documented failure mode, driven through the in-process loopback.
//!
//! Everything here goes through the real path: the host is an IPC client, the
//! server is a live [`gnoblin_model::Graph`], and every observation crosses the
//! wire codec. Only the byte transport is in process, so these tests keep their
//! meaning when the socket lands.
//!
//! A test observes Lua through three channels and no others:
//!
//! - `assert` inside the script. A failed assertion is a reported script error, so
//!   `errors.is_empty()` is an assertion the script made about what it saw.
//! - The handler count in a [`PumpReport`], which says what ran.
//! - The server's graph, which shows what a handler asked the compositor to do.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use gnoblin_client::{Loopback, Server};
use gnoblin_config::{SearchPaths, Value};
use gnoblin_model::{Fields, Graph, ObjectId, OperationId, OperationOutcome, Tick, TypeKey, Value as ModelValue};
use gnoblin_proto::Version;
use gnoblin_schema::Catalog;

use gnoblin_lua::{Host, Limits};

const VERSION: Version = Version::new(0, 2, 0);

/// A configuration directory for one test, removed when the test ends.
struct Config {
    root: PathBuf,
}

impl Config {
    fn new(name: &str) -> Config {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("gnoblin-lua-test-{}-{name}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("a test can create its configuration directory");
        Config { root }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("a test can create a directory");
        }
        fs::write(&path, text).expect("a test can write a file");
        path
    }

    fn paths(&self) -> SearchPaths {
        SearchPaths {
            home: None,
            config_home: None,
            override_path: Some(self.root.join("init.lua")),
        }
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn graph() -> Graph {
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    Graph::new(catalog)
}

/// A live server and a host connected to it.
fn pair() -> (Server, Host<Loopback>) {
    pair_with(Limits::default())
}

/// The same, with budgets the test chose.
fn pair_with(limits: Limits) -> (Server, Host<Loopback>) {
    let mut server = Server::new(graph(), VERSION);
    let client = server.connect(VERSION).expect("the versions match");
    let host = Host::new(client, limits).expect("the host starts");
    (server, host)
}

fn window_properties(title: &str) -> Fields {
    gnoblin_testkit::window_fields(&[
        ("id", ModelValue::Uint(1)),
        ("title", ModelValue::Text(title.to_owned())),
        ("app_id", ModelValue::Text("org.example.App".to_owned())),
        ("width", ModelValue::Uint(800)),
        ("height", ModelValue::Uint(600)),
    ])
}

fn open_window(server: &mut Server, title: &str) -> ObjectId {
    server
        .create(TypeKey::gnoblin("window"), window_properties(title))
        .expect("the window type and its properties are in the schema")
}

/// The session object, which is the first object a graph creates.
fn session(server: &mut Server) -> ObjectId {
    let mut properties = Fields::new();
    properties.insert("version".to_owned(), ModelValue::Text(VERSION.to_string()));
    // Every slot starts with no provider, which is the schema's fail-closed default.
    for slot in [
        "slot_auth_polkit",
        "slot_prompt_secret",
        "slot_location_agent",
        "slot_idle_policy",
    ] {
        properties.insert(slot.to_owned(), ModelValue::Text("none".to_owned()));
    }
    server
        .create(TypeKey::gnoblin("session"), properties)
        .expect("the session type is in the schema")
}

#[test]
fn a_script_cannot_reach_the_filesystem_or_a_library_the_sandbox_withholds() {
    let config = Config::new("sandbox");
    config.write(
        "init.lua",
        r#"
assert(io == nil, "io is available")
assert(os.execute == nil and os.remove == nil and os.rename == nil and os.getenv == nil and os.tmpname == nil and os.exit == nil, "a host-reaching os function is available")
assert(debug == nil, "debug is available")
assert(package == nil, "package is available")
assert(type(coroutine) == "table", "coroutine is missing")
assert(dofile == nil, "dofile is available")
assert(loadfile == nil, "loadfile is available")
assert(type(require) == "function", "require is missing")
assert(not pcall(require, "not.a.namespace"), "require loaded an undeclared namespace")
assert(type(gnoblin.load) == "function", "the loader is the one way to a file")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.loaded, vec![config.root.join("init.lua")]);
}

#[test]
fn require_answers_only_declared_extension_namespaces() {
    let config = Config::new("require");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "example.ext")
assert(not ok, "require loaded a namespace no host configuration declared")
assert(string.match(tostring(message), "unknown module `example.ext`") ~= nil, "message was " .. tostring(message))
assert(not pcall(function() return gnoblin.extensions end), "gnoblin.extensions must not exist")
assert(not pcall(function() return gnoblin.objects end), "gnoblin.objects must not exist")
local ok2, message2 = pcall(require, "other.nothing")
assert(not ok2, "require loaded an undeclared namespace")
assert(string.match(tostring(message2), "unknown module `other.nothing`") ~= nil, "message was " .. tostring(message2))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.loaded, vec![config.root.join("init.lua")]);
}

#[test]
fn a_binary_chunk_is_refused_rather_than_run() {
    let config = Config::new("binary");
    // The Lua signature followed by nonsense: text mode refuses it before any of
    // it is loaded, because crafted bytecode can crash the interpreter.
    fs::write(config.root.join("init.lua"), b"\x1bLua\x54\x00\x19\x93\r\n\x1a\n").expect("a test can write a file");
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    let message = report.errors[0].message().to_lowercase();
    assert!(message.contains("binary") || message.contains("text"), "{message}");
}

#[test]
fn an_infinite_loop_is_stopped_by_the_instruction_budget() {
    // A deadline far beyond the test's patience, so this test can only pass by the
    // instruction count.
    let limits = Limits {
        handler_ticks: 600_000,
        ..Limits::default()
    };
    let config = Config::new("loop");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function() while true do end end)
gnoblin.timers.after(1, function() gnoblin.session.by_id(1) end)
"#,
    );
    let (mut server, mut host) = pair_with(limits);
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let pumped = host.pump(Tick::new(1)).expect("the pump survives a loop");
    assert_eq!(pumped.handlers, 2, "the second timer must still run");
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    assert!(
        pumped.errors[0].message().contains("instruction budget"),
        "{}",
        pumped.errors[0].message()
    );

    // The host is still usable: the state survived the aborted entry.
    let again = host.pump(Tick::new(2)).expect("the host still pumps");
    assert_eq!(again.errors.len(), 0);
}

#[test]
fn an_erroring_handler_does_not_stop_the_other_handlers() {
    let (mut server, mut host) = pair();
    let session = session(&mut server);
    let window = open_window(&mut server, "First");
    let config = Config::new("handler-error");
    config.write(
        "init.lua",
        &format!(
            r#"
local session = gnoblin.session.by_id({session})
assert(session ~= nil, "the session is in the snapshot")
session:on("ready", function() error("boom") end)
session:on("ready", function() gnoblin.window.by_id({window}):focus() end)
"#,
            session = session.get(),
            window = window.get()
        ),
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    server
        .signal(session, "ready", Fields::new())
        .expect("ready is a signal");
    let pumped = host.pump(Tick::new(1)).expect("the pump survives an error");
    assert_eq!(pumped.handlers, 2, "the second handler still ran");
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    assert!(pumped.errors[0].message().contains("boom"), "{:?}", pumped.errors[0]);

    let pending: Vec<OperationId> = server.graph().pending().map(|operation| operation.id()).collect();
    assert_eq!(pending.len(), 1, "the second handler called focus");
    assert_eq!(server.graph().pending().next().expect("one call").object(), window);
}

#[test]
fn a_load_pattern_cannot_leave_the_configuration_directory() {
    let config = Config::new("escape");
    config.write("outside.lua", "error('this file must never run')\n");
    config.write("init.lua", "gnoblin.load('../outside.lua')\n");
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    let message = report.errors[0].message();
    assert!(message.contains("gnoblin.load"), "{message}");
    assert!(message.contains(".."), "{message}");

    // An absolute path is refused for the same reason.
    config.write("init.lua", "gnoblin.load('/etc/passwd')\n");
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    assert!(
        report.errors[0].message().contains("absolute"),
        "{:?}",
        report.errors[0]
    );
}

#[test]
fn a_file_symlink_that_points_outside_the_configuration_directory_is_refused() {
    let config = Config::new("escape-link");
    let outside = Config::new("escape-link-outside");
    outside.write("secret.lua", "error('this file must never run')\n");
    std::os::unix::fs::symlink(outside.root.join("secret.lua"), config.root.join("link.lua"))
        .expect("a test can create a link");
    config.write("init.lua", "gnoblin.load('link.lua')\n");
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    assert!(
        report.errors[0]
            .message()
            .contains("outside the configuration directory"),
        "{:?}",
        report.errors[0]
    );
}

#[test]
fn a_glob_matches_in_filename_order_and_loads_each_file() {
    let config = Config::new("glob");
    config.write("conf.d/20-second.lua", "gnoblin.configure { cursor = { size = 32 } }\n");
    config.write(
        "conf.d/10-first.lua",
        "gnoblin.configure { cursor = { theme = 'Adwaita' } }\n",
    );
    config.write(
        "init.lua",
        r#"
gnoblin.load("conf.d/**/*.lua")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.loaded.len(), 3, "init.lua and two parts: {:#?}", report.loaded);
    let document = report
        .document
        .as_table()
        .expect("a document")
        .get("cursor")
        .expect("cursor");
    let document = document.as_table().expect("a table");
    assert_eq!(document.get("size").and_then(Value::as_integer), Some(32));
    assert_eq!(document.get("theme").and_then(Value::as_str), Some("Adwaita"));
}

#[test]
fn an_operation_completes_its_callback_once() {
    let config = Config::new("completion");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function()
    local window = gnoblin.window.by_id(1)
    local operation = window:focus()
    assert(operation.status == "pending", "a fresh operation is pending, was " .. operation.status)
    assert(operation.method == "focus", "the handle names its method, was " .. operation.method)
    operation:on_complete(function()
        assert(operation.status == "succeeded", "the completion sets the status, was " .. operation.status)
        assert(operation.code == nil, "a success has no code")
        -- A second call is what makes a second delivery visible: one callback,
        -- one more pending operation on the server.
        window:focus()
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    let window = open_window(&mut server, "First");
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let pumped = host.pump(Tick::new(1)).expect("the timer calls focus");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);

    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("the first completion is accepted");
    // The graph refuses a second one, which is the half of the rule the
    // compositor owns.
    assert!(server.complete(operation, OperationOutcome::Done).is_err());

    let pumped = host.pump(Tick::new(2)).expect("the completion arrives");
    assert_eq!(pumped.handlers, 1, "exactly one completion callback");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    let pending: Vec<OperationId> = server.graph().pending().map(|operation| operation.id()).collect();
    assert_eq!(pending.len(), 1, "the callback ran once and called focus once");
    assert_eq!(
        server.graph().pending().next().expect("one call").object(),
        window,
        "the callback's call is on the same window"
    );
}

#[test]
fn a_repeated_error_is_reported_once_per_site() {
    let config = Config::new("repeat");
    config.write(
        "init.lua",
        r#"
local function same_site() error("boom") end
gnoblin.timers.after(1, same_site)
gnoblin.timers.after(1, same_site)
gnoblin.timers.after(1, function() error("other") end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let pumped = host.pump(Tick::new(1)).expect("the pump runs the timers");
    assert_eq!(pumped.handlers, 3);
    assert_eq!(pumped.errors.len(), 2, "one site reported twice: {:#?}", pumped.errors);
    let messages: Vec<&str> = pumped.errors.iter().map(gnoblin_lua::ScriptError::message).collect();
    assert!(messages.iter().any(|message| message.contains("boom")), "{messages:?}");
    assert!(messages.iter().any(|message| message.contains("other")), "{messages:?}");
}

#[test]
fn a_timer_fires_on_its_tick_and_never_after_cancel() {
    let config = Config::new("timer");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(10, function()
    assert(gnoblin.timers.now() == 10, "the timer sees the caller's tick, saw " .. gnoblin.timers.now())
end)
local cancelled = gnoblin.timers.after(5, function() error("a cancelled timer fired") end)
cancelled:cancel()
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    assert_eq!(host.pump(Tick::new(4)).expect("a pump").handlers, 0);
    assert_eq!(host.pump(Tick::new(5)).expect("a pump").handlers, 0, "cancelled at 5");
    let due = host.pump(Tick::new(10)).expect("a pump");
    assert_eq!(due.handlers, 1);
    assert!(due.errors.is_empty(), "{:#?}", due.errors);
    assert_eq!(
        host.pump(Tick::new(11)).expect("a pump").handlers,
        0,
        "a one-shot timer fires once"
    );
}

#[test]
fn configure_problems_arrive_with_their_path_and_change_nothing() {
    let config = Config::new("configure");
    config.write(
        "init.lua",
        r#"
gnoblin.configure { cursor = { size = 24 } }
local ok, message = pcall(function()
    gnoblin.configure { cursor = { size = "big", theme = 7 } }
end)
assert(not ok, "an invalid setting was accepted")
message = tostring(message)
assert(string.find(message, "cursor.size", 1, true), "the path is missing from: " .. message)
assert(string.find(message, "cursor.theme", 1, true), "every problem is reported: " .. message)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    // The rejected call changed nothing: the earlier value stands.
    let cursor = report
        .document
        .as_table()
        .expect("a document")
        .get("cursor")
        .expect("cursor");
    assert_eq!(
        cursor
            .as_table()
            .expect("a table")
            .get("size")
            .and_then(Value::as_integer),
        Some(24)
    );
}

#[test]
fn an_unknown_setting_is_refused_with_its_path() {
    let config = Config::new("unknown-setting");
    config.write("init.lua", "gnoblin.configure { cursor = { sise = 24 } }\n");
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    let message = report.errors[0].message();
    assert!(message.contains("cursor.sise"), "{message}");
    assert!(message.contains("unknown setting"), "{message}");
    // The rejected call changed nothing, so what is left still validates.
    assert!(report.document.as_table().expect("a document").is_empty());
    assert!(report.config.is_ok(), "{:?}", report.config.err());
}

#[test]
fn reads_come_from_the_mirror_and_calls_become_operations() {
    let config = Config::new("reads");
    config.write(
        "init.lua",
        r#"
local window = gnoblin.window.by_id(1)
assert(window ~= nil, "the window is in the snapshot")
assert(window.title == "Second", "the snapshot title, was " .. tostring(window.title))
assert(window.app_id == "org.example.App", "the app id, was " .. tostring(window.app_id))
assert(window.focused == false, "not focused")
assert(gnoblin.window.all()[1] == window, "two reads are the same record")
"#,
    );
    let (mut server, mut host) = pair();
    let window = open_window(&mut server, "First");
    server
        .set(window, "title", ModelValue::Text("Second".to_owned()))
        .expect("title is a declared property");
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    // A change after the snapshot arrives as a diff the next pump applies.
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function()
    local window = gnoblin.window.by_id(1)
    assert(window.title == "Third", "the diff was applied, was " .. tostring(window.title))
end)
"#,
    );
    server
        .set(window, "title", ModelValue::Text("Third".to_owned()))
        .expect("title is a declared property");
    let mut host = host;
    let report = host.run_config(&config.paths()).expect("the run restarts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let pumped = host.pump(Tick::new(1)).expect("a pump applies the diff");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_closed_object_delivers_closed_once_and_then_reads_nothing() {
    let (mut server, mut host) = pair();
    let window = open_window(&mut server, "First");
    let config = Config::new("closed");
    config.write(
        "init.lua",
        &format!(
            r#"
local window = gnoblin.window.by_id({id})
assert(window ~= nil, "the window is in the snapshot")
window:on("closed", function()
    -- The mirror still holds the object while the handler runs, so the last
    -- state is readable from inside it.
    assert(window.title == "First", "the last title, was " .. tostring(window.title))
end)
gnoblin.timers.after(2, function()
    assert(gnoblin.window.by_id({id}) == nil, "a closed object is not reachable")
end)
"#,
            id = window.get()
        ),
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    server.close(window).expect("the window can be closed");
    let pumped = host.pump(Tick::new(1)).expect("closed arrives");
    assert_eq!(pumped.handlers, 1, "one closed handler");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert!(server.graph().object(window).is_none());

    let pumped = host.pump(Tick::new(2)).expect("the timer checks the mirror");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);

    // The handler is gone with its object: a second close signal is impossible,
    // and a fresh pump has nothing left to run.
    let pumped = host.pump(Tick::new(3)).expect("nothing left");
    assert_eq!(pumped.handlers, 0);
}

#[test]
fn configure_is_refused_outside_a_configuration_run() {
    let config = Config::new("runtime-configure");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function()
    local ok, message = pcall(function() gnoblin.configure { cursor = { size = 32 } } end)
    assert(not ok, "a runtime configure was accepted")
    assert(string.find(tostring(message), "configuration loads", 1, true) ~= nil, tostring(message))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let pumped = host.pump(Tick::new(1)).expect("the timer runs");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_handler_that_outlives_its_deadline_is_stopped() {
    // One millisecond: the hook sees the budget gone long before the instruction
    // count would run out.
    let limits = Limits {
        handler_ticks: 1,
        ..Limits::default()
    };
    let config = Config::new("deadline");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function() while true do end end)
gnoblin.timers.after(1, function() gnoblin.session.by_id(1) end)
"#,
    );
    let (mut server, mut host) = pair_with(limits);
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let pumped = host.pump(Tick::new(1)).expect("the pump survives the deadline");
    assert_eq!(pumped.handlers, 2, "the second timer still ran");
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    assert!(
        pumped.errors[0].message().contains("deadline"),
        "{}",
        pumped.errors[0].message()
    );
}

#[test]
fn a_script_that_exceeds_the_memory_limit_is_stopped() {
    // The deadline is wide so the allocation, not the wall clock, is what stops the script. At 16 ms
    // the default deadline races a debug build's 1 MiB fill when the suite runs in parallel.
    let limits = Limits {
        memory_bytes: 1 << 20,
        instructions: 1_000_000_000,
        handler_ticks: 10_000,
        ..Limits::default()
    };
    let config = Config::new("memory");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function()
    local held = {}
    for index = 1, 100000000 do held[index] = index end
end)
"#,
    );
    let (_server, mut host) = pair_with(limits);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let pumped = host.pump(Tick::new(1)).expect("the pump survives the allocation");
    assert_eq!(pumped.handlers, 1);
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    let message = pumped.errors[0].message().to_lowercase();
    assert!(message.contains("memory"), "{message}");

    // The host is still usable after a refused allocation.
    let again = host.pump(Tick::new(2)).expect("the host still pumps");
    assert_eq!(again.errors.len(), 0);
}

// Plain modules: `require("a.b.c")` loads `a/b/c.lua` under the configuration's
// module roots. These tests pin the resolver's observable contract; they fail
// until the config VM's `require` is pointed at the shared resolver.

#[test]
fn require_maps_a_dotted_name_to_a_file_under_the_user_root() {
    let config = Config::new("require-dotted");
    config.write(
        "lib/util/strings.lua",
        "return { shout = function(s) return s .. '!' end }\n",
    );
    config.write(
        "init.lua",
        r#"
local strings = require("lib.util.strings")
assert(type(strings) == "table", "a plain module returns its value")
assert(strings.shout("hi") == "hi!", "the module's function is reachable")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_plain_module_is_evaluated_once_per_run() {
    let config = Config::new("require-cache");
    config.write("lib/counter.lua", "COUNT = (COUNT or 0) + 1\nreturn { runs = COUNT }\n");
    config.write(
        "init.lua",
        r#"
local first = require("lib.counter")
local second = require("lib.counter")
assert(rawequal(first, second), "a second require must return the cached value")
assert(first.runs == 1, "the module ran more than once")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn an_unknown_plain_module_names_both_searched_roots() {
    let config = Config::new("require-unknown");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "lib.missing")
assert(not ok, "require loaded a module that does not exist")
message = tostring(message)
assert(string.match(message, "unknown module `lib.missing`: no extension namespace or module has that name") ~= nil, "message was " .. message)
assert(string.match(message, "searched `") ~= nil, "message does not name the searched roots: " .. message)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_name_that_is_not_a_module_name_is_refused() {
    let config = Config::new("require-bad-name");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "lib..util")
assert(not ok, "require accepted an empty component")
assert(string.match(tostring(message), "`lib..util` is not a module name") ~= nil, "message was " .. tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_plain_module_symlinked_outside_its_root_is_refused() {
    let config = Config::new("require-escape");
    let outside = Config::new("require-escape-outside");
    outside.write("secret.lua", "return { leaked = true }\n");
    config.write("lib/.keep", "");
    std::os::unix::fs::symlink(outside.root.join("secret.lua"), config.root.join("lib/evil.lua"))
        .expect("a test can create a symlink");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "lib.evil")
assert(not ok, "require followed a symlink out of the configuration root")
assert(string.match(tostring(message), "module `lib.evil` resolves outside its root") ~= nil, "message was " .. tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn an_unknown_gnoblin_name_says_the_namespace_is_reserved() {
    let config = Config::new("require-reserved");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "gnoblin.nothing")
assert(not ok, "require invented a shipped module")
assert(string.match(tostring(message), "reserved") ~= nil, "message was " .. tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_module_that_requires_itself_through_a_cycle_is_refused() {
    let config = Config::new("require-cycle");
    config.write("cyc/a.lua", "return require('cyc.b')\n");
    config.write("cyc/b.lua", "return require('cyc.a')\n");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "cyc.a")
assert(not ok, "a cycle loaded")
assert(string.match(tostring(message), "requires itself") ~= nil, "message was " .. tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_missing_layout_reference_keeps_the_unknown_module_message() {
    let config = Config::new("require-layout-missing");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(require, "layouts.nothing")
assert(not ok, "require returned a layout that does not exist")
assert(string.match(tostring(message), "unknown module `layouts.nothing`") ~= nil, "message was " .. tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

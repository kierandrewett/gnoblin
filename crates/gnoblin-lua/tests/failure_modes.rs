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

use gnoblin_lua::{Host, Limits, ReloadOutcome};

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

    let pumped = host.pump(Tick::new(1000)).expect("the pump survives a loop");
    assert_eq!(pumped.handlers, 2, "the second timer must still run");
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    assert!(
        pumped.errors[0].message().contains("instruction budget"),
        "{}",
        pumped.errors[0].message()
    );

    // The host is still usable: the state survived the aborted entry.
    let again = host.pump(Tick::new(2000)).expect("the host still pumps");
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
fn global_signal_handlers_are_ordered_isolated_and_one_shot() {
    let config = Config::new("global-events");
    let (mut server, mut host) = pair();
    let session_id = session(&mut server);
    config.write(
        "init.lua",
        &format!(
            r#"
local calls = {{}}
gnoblin.on("gnoblin.session.ready", function()
    error("global handler failed")
end)
gnoblin.on("gnoblin.session.ready", function(event)
    assert(event.name == "gnoblin.session.ready")
    assert(event.object.version == "{VERSION}")
    table.insert(calls, "first")
end)
gnoblin.once("gnoblin.session.ready", function()
    table.insert(calls, "once")
end)
local removed = gnoblin.on("gnoblin.session.ready", function()
    table.insert(calls, "removed")
end)
removed:unsubscribe()
gnoblin.on("gnoblin.session.ready", function()
    table.insert(calls, "last")
    if #calls == 5 then
        assert(table.concat(calls, ",") == "first,once,last,first,last")
    end
end)
"#
        ),
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    for _ in 0..2 {
        server
            .signal(session_id, "ready", Fields::new())
            .expect("ready is a signal");
    }
    let pumped = host.pump(Tick::new(1)).expect("the pump survives handler failures");
    assert_eq!(pumped.handlers, 7, "one-shot and repeating callbacks are dispatched");
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    assert!(
        pumped.errors[0].message().contains("global handler failed"),
        "{:?}",
        pumped.errors[0]
    );
    assert_eq!(
        pumped.errors[0].source(),
        Some(config.root.join("init.lua").to_str().expect("test path is UTF-8"))
    );
}

#[test]
fn reloading_clears_global_signal_handlers() {
    let config = Config::new("global-event-reload");
    let path = config.write(
        "init.lua",
        r#"
gnoblin.on("gnoblin.session.ready", function() error("stale global handler") end)
"#,
    );
    let (mut server, mut host) = pair();
    let session_id = session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    fs::write(&path, "-- new configuration\n").expect("the config changes");
    assert!(matches!(
        host.reload_config(&config.paths()).expect("reload succeeds"),
        ReloadOutcome::Accepted(_)
    ));
    server
        .signal(session_id, "ready", Fields::new())
        .expect("ready is a signal");
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    assert_eq!(pumped.handlers, 0);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn global_event_registration_rejects_unknown_names_and_excess_handlers() {
    let config = Config::new("global-event-validation");
    config.write(
        "init.lua",
        r#"
assert(not pcall(gnoblin.on, "gnoblin.no_such_event", function() end))
for _ = 1, 1025 do
    gnoblin.on("gnoblin.session.ready", function() end)
end
"#,
    );
    // The loop of 1025 registrations is the test; the wall-clock budget is not, so it
    // is set well above what a loaded debug build needs.
    let (_server, mut host) = pair_with(Limits {
        handler_ticks: 600_000,
        ..Limits::default()
    });
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    assert!(
        report.errors[0].message().contains("handler limit"),
        "{:?}",
        report.errors[0]
    );
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

    let pumped = host.pump(Tick::new(1000)).expect("the timer calls focus");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);

    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("the first completion is accepted");
    // The graph refuses a second one, which is the half of the rule the
    // compositor owns.
    assert!(server.complete(operation, OperationOutcome::Done).is_err());

    let pumped = host.pump(Tick::new(2000)).expect("the completion arrives");
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
fn an_operation_map_transforms_its_completion_value() {
    let config = Config::new("operation-map");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus()
    operation:map(function(value, err)
        assert(err == nil, "a successful source has no error")
        return value .. " mapped"
    end):on_complete(function(value, err)
        assert(err == nil, "a successful map has no error")
        assert(value == "source mapped", "mapped value was " .. tostring(value))
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    assert!(
        host.run_config(&config.paths())
            .expect("the run starts")
            .errors
            .is_empty()
    );
    assert!(
        host.pump(Tick::new(0))
            .expect("the timer calls focus")
            .errors
            .is_empty()
    );
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(
            operation,
            OperationOutcome::Value(ModelValue::Text("source".to_owned())),
        )
        .expect("the completion is accepted");
    let pumped = host.pump(Tick::new(1)).expect("the map runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert_eq!(host.pump(Tick::new(2)).expect("the derived callback runs").handlers, 1);
}

#[test]
fn an_operation_map_error_fails_the_derived_operation() {
    let config = Config::new("operation-map-error");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus()
    operation:map(function() error("map exploded") end):on_complete(function(value, err)
        assert(value == nil)
        assert(err.code == "internal", "map error code was " .. tostring(err.code))
        assert(string.find(err.message, "map exploded", 1, true), "missing map error: " .. tostring(err.message))
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    assert!(
        host.run_config(&config.paths())
            .expect("the run starts")
            .errors
            .is_empty()
    );
    host.pump(Tick::new(0)).expect("the timer calls focus");
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("completion accepted");
    let pumped = host.pump(Tick::new(1)).expect("the map error is isolated");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    let callback = host.pump(Tick::new(2)).expect("the failure callback runs");
    assert_eq!(callback.handlers, 1);
    assert!(callback.errors.is_empty(), "{:#?}", callback.errors);
}

#[test]
fn an_operation_map_propagates_a_source_failure_without_calling_the_mapper() {
    let config = Config::new("operation-map-source-failure");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    gnoblin.window.by_id(1):focus()
        :map(function() error("mapper must not run") end)
        :on_complete(function(value, err)
            assert(value == nil)
            assert(err.code == "denied", "source failure became " .. tostring(err.code))
        end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    host.pump(Tick::new(0)).expect("the timer calls focus");
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(
            operation,
            OperationOutcome::Failed("denied".parse().expect("valid error code")),
        )
        .expect("failure completion accepted");
    host.pump(Tick::new(1)).expect("the failure propagates");
    let callback = host.pump(Tick::new(2)).expect("the chained callback runs");
    assert_eq!(callback.handlers, 1);
    assert!(callback.errors.is_empty(), "{:#?}", callback.errors);
}

#[test]
fn an_operation_timeout_fails_after_its_deadline() {
    let config = Config::new("operation-timeout");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus():timeout(0.5)
    operation:on_complete(function(value, err)
        assert(value == nil)
        assert(err.code == "timed_out", "timeout code was " .. tostring(err.code))
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    assert!(
        host.run_config(&config.paths())
            .expect("the run starts")
            .errors
            .is_empty()
    );
    host.pump(Tick::new(0)).expect("the timer calls focus");
    assert_eq!(host.pump(Tick::new(499)).expect("before timeout").handlers, 0);
    let pumped = host.pump(Tick::new(500)).expect("timeout fires");
    assert_eq!(pumped.handlers, 2);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert_eq!(
        server.graph().pending().count(),
        1,
        "timeout does not cancel the source"
    );
}

#[test]
fn an_operation_timeout_is_cleared_when_its_source_finishes() {
    let config = Config::new("operation-timeout-cleared");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus():timeout(1)
    operation:on_complete(function(_, err)
        assert(err == nil, "the source completed before its timeout")
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    host.pump(Tick::new(0)).expect("the timer calls focus");
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("completion accepted");
    assert_eq!(host.pump(Tick::new(1)).expect("source completion forwards").handlers, 1);
    assert_eq!(host.pump(Tick::new(2)).expect("the target callback runs").handlers, 1);
    let later = host.pump(Tick::new(1000)).expect("no timeout remains");
    assert_eq!(later.handlers, 0);
    assert!(later.errors.is_empty(), "{:#?}", later.errors);
}

#[test]
fn cancelling_an_operation_ignores_its_late_completion() {
    let config = Config::new("operation-cancel");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus()
    assert(operation:cancel(), "pending operation cancels")
    assert(operation.status == "failed")
    assert(operation.code == "cancelled")
    operation:on_complete(function(value, err)
        assert(value == nil)
        assert(err.code == "cancelled")
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    let cancelled = host.pump(Tick::new(0)).expect("the operation cancels");
    assert!(cancelled.errors.is_empty(), "{:#?}", cancelled.errors);
    let operation = server
        .graph()
        .pending()
        .next()
        .expect("the remote operation remains queued")
        .id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("late completion is accepted remotely");
    let pumped = host.pump(Tick::new(1)).expect("late completion is ignored");
    assert_eq!(pumped.handlers, 0);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn cancelling_a_completed_operation_is_a_noop() {
    let config = Config::new("operation-cancel-completed");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    local operation = gnoblin.window.by_id(1):focus()
    operation:on_complete(function()
        assert(not operation:cancel(), "completed operation cannot be cancelled")
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    host.pump(Tick::new(0)).expect("the timer calls focus");
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Done)
        .expect("completion accepted");
    let pumped = host.pump(Tick::new(1)).expect("completion callback runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn an_operation_timeout_does_not_fire_after_reload() {
    let config = Config::new("operation-timeout-reload");
    let path = config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    gnoblin.window.by_id(1):focus():timeout(1):on_complete(function()
        error("timeout survived reload")
    end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    host.pump(Tick::new(0)).expect("the timer calls focus");
    fs::write(&path, "-- new runtime\n").expect("the config changes");
    assert!(matches!(
        host.reload_config(&config.paths()).expect("reload succeeds"),
        ReloadOutcome::Accepted(_)
    ));
    let pumped = host.pump(Tick::new(1000)).expect("old timeout was cleared");
    assert_eq!(pumped.handlers, 0);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn operation_combinators_chain_maps_and_timeout() {
    let config = Config::new("operation-chain");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(0, function()
    gnoblin.window.by_id(1):focus()
        :map(function(value) return value * 2 end)
        :timeout(1)
        :map(function(value) return value + 1 end)
        :on_complete(function(value, err)
            assert(err == nil, "chained operation failed")
            assert(value == 7, "chained value was " .. tostring(value))
        end)
end)
"#,
    );
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    host.run_config(&config.paths()).expect("the run starts");
    host.pump(Tick::new(0)).expect("the timer calls focus");
    let operation = server.graph().pending().next().expect("one call").id();
    server
        .complete(operation, OperationOutcome::Value(ModelValue::Int(3)))
        .expect("completion accepted");
    for tick in 1..=3 {
        let report = host.pump(Tick::new(tick)).expect("the chain progresses");
        assert!(report.errors.is_empty(), "{:#?}", report.errors);
    }
    let callback = host.pump(Tick::new(4)).expect("the chained callback runs");
    assert_eq!(callback.handlers, 1);
    assert!(callback.errors.is_empty(), "{:#?}", callback.errors);
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

    let pumped = host.pump(Tick::new(1000)).expect("the pump runs the timers");
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
gnoblin.timers.after(1, function()
    assert(gnoblin.timers.now() == 1, "the timer sees the caller's tick, saw " .. gnoblin.timers.now())
end)
local cancelled = gnoblin.timers.after(0.5, function() error("a cancelled timer fired") end)
cancelled:cancel()
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    assert_eq!(host.pump(Tick::new(400)).expect("a pump").handlers, 0);
    assert_eq!(
        host.pump(Tick::new(500)).expect("a pump").handlers,
        0,
        "cancelled at 0.5 s"
    );
    let due = host.pump(Tick::new(1000)).expect("a pump");
    assert_eq!(due.handlers, 1);
    assert!(due.errors.is_empty(), "{:#?}", due.errors);
    assert_eq!(
        host.pump(Tick::new(1001)).expect("a pump").handlers,
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
    let pumped = host.pump(Tick::new(1000)).expect("a pump applies the diff");
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

    let pumped = host.pump(Tick::new(2000)).expect("the timer checks the mirror");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);

    // The handler is gone with its object: a second close signal is impossible,
    // and a fresh pump has nothing left to run.
    let pumped = host.pump(Tick::new(3000)).expect("nothing left");
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
    let pumped = host.pump(Tick::new(1000)).expect("the timer runs");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_global_event_handler_that_outlives_its_deadline_is_stopped() {
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
gnoblin.on("gnoblin.session.ready", function() while true do end end)
gnoblin.on("gnoblin.session.ready", function() gnoblin.session.by_id(1) end)
"#,
    );
    let (mut server, mut host) = pair_with(limits);
    let session = session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    server
        .signal(session, "ready", Fields::new())
        .expect("ready is a signal");
    let pumped = host.pump(Tick::new(1)).expect("the pump survives the deadline");
    assert_eq!(pumped.handlers, 2, "the second event handler still ran");
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

    let pumped = host.pump(Tick::new(1000)).expect("the pump survives the allocation");
    assert_eq!(pumped.handlers, 1);
    assert_eq!(pumped.errors.len(), 1, "{:#?}", pumped.errors);
    let message = pumped.errors[0].message().to_lowercase();
    assert!(message.contains("memory"), "{message}");

    // The host is still usable after a refused allocation.
    let again = host.pump(Tick::new(2000)).expect("the host still pumps");
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

#[test]
fn a_timer_delay_outside_one_day_or_not_a_number_is_refused() {
    let config = Config::new("timer-range");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(gnoblin.timers.after, 86401, function() end)
assert(not ok, "a delay past one day was accepted")
assert(string.find(tostring(message), "86400", 1, true), "the range is missing from: " .. tostring(message))
assert(not pcall(gnoblin.timers.after, -1, function() end), "a negative delay was accepted")
assert(not pcall(gnoblin.timers.after, 0 / 0, function() end), "a NaN delay was accepted")
gnoblin.timers.after(86400, function() end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(host.pump(Tick::new(86_399_999)).expect("a pump").handlers, 0);
    assert_eq!(host.pump(Tick::new(86_400_000)).expect("a pump").handlers, 1);
}

#[test]
fn the_pending_timer_count_is_capped_at_1024() {
    let config = Config::new("timer-cap");
    config.write(
        "init.lua",
        r#"
local refused = 0
for _ = 1, 1025 do
    if not pcall(gnoblin.timers.after, 1, function() end) then
        refused = refused + 1
    end
end
assert(refused == 1, "expected one refusal past 1024, saw " .. refused)
"#,
    );
    // The 1025 `pcall`s are cheap, but a debug build under a parallel suite can exceed the default
    // 16 ms handler deadline. The deadline is not what this test measures, so it is widened.
    let (_server, mut host) = pair_with(Limits {
        handler_ticks: 600_000,
        ..Limits::default()
    });
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(host.pump(Tick::new(1000)).expect("a pump").handlers, 1024);
}

#[test]
fn a_reload_cancels_the_timers_of_the_previous_run() {
    let config = Config::new("timer-reload");
    config.write(
        "init.lua",
        "gnoblin.timers.after(1, function() error(\"a timer from before the reload fired\") end)\n",
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    config.write("init.lua", "");
    let report = host.run_config(&config.paths()).expect("the run restarts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let pumped = host.pump(Tick::new(1000)).expect("a pump");
    assert_eq!(pumped.handlers, 0, "the reload cleared the old timer");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_reload_with_a_syntax_error_keeps_the_last_good_document_and_names_the_line() {
    let (mut server, mut host) = pair();
    let session = session(&mut server);
    let window = open_window(&mut server, "Kept");
    let config = Config::new("reload-syntax");
    let init = config.write(
        "init.lua",
        &format!(
            r#"
gnoblin.configure {{ cursor = {{ size = 24 }} }}
local session = gnoblin.session.by_id({session})
session:on("ready", function() gnoblin.window.by_id({window}):focus() end)
"#,
            session = session.get(),
            window = window.get()
        ),
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    fs::write(
        &init,
        "gnoblin.configure { cursor = { size = 12 } }\nlocal broken = = 1\n",
    )
    .expect("a test can rewrite its file");
    let ReloadOutcome::Rejected(reason) = host.reload_config(&config.paths()).expect("the reload runs") else {
        panic!("a reload with a syntax error was accepted");
    };
    assert!(reason.contains("init.lua:2:"), "the line is named: {reason}");

    // The refused file was never put in force, so the handler of the last good one still answers.
    server
        .signal(session, "ready", Fields::new())
        .expect("ready is a signal");
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    assert_eq!(pumped.handlers, 1, "the last good handler still runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert_eq!(
        server.graph().pending().next().expect("the focus call").object(),
        window
    );
}

#[test]
fn a_rejected_reload_leaves_an_operation_that_was_pending_with_its_callback() {
    let (mut server, mut host) = pair();
    let session = session(&mut server);
    let window = open_window(&mut server, "Pending");
    let config = Config::new("reload-pending");
    let init = config.write(
        "init.lua",
        &format!(
            r#"
local session = gnoblin.session.by_id({session})
session:on("ready", function()
    local operation = gnoblin.window.by_id({window}):focus()
    operation:on_complete(function() gnoblin.window.by_id({window}):focus() end)
end)
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
    host.pump(Tick::new(1)).expect("the handler calls focus");
    let first = server.graph().pending().next().expect("one call").id();

    fs::write(&init, "local broken = = 1\n").expect("a test can rewrite its file");
    assert!(matches!(
        host.reload_config(&config.paths()).expect("the reload runs"),
        ReloadOutcome::Rejected(_)
    ));

    // The rejected reload restored the callbacks, so the completion still reaches its handler,
    // which calls focus once more.
    server
        .complete(first, OperationOutcome::Done)
        .expect("the first completion is accepted");
    let pumped = host.pump(Tick::new(1)).expect("the completion is delivered");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    let pending: Vec<OperationId> = server.graph().pending().map(|operation| operation.id()).collect();
    assert_eq!(pending.len(), 1, "the completion callback ran once and called focus");
    assert_ne!(pending[0], first);
}

#[test]
fn an_accepted_reload_replaces_the_handlers_and_drops_the_callbacks_of_the_old_run() {
    let (mut server, mut host) = pair();
    let session = session(&mut server);
    let window = open_window(&mut server, "Replaced");
    let config = Config::new("reload-accepted");
    let init = config.write(
        "init.lua",
        &format!(
            r#"
local session = gnoblin.session.by_id({session})
session:on("ready", function()
    local operation = gnoblin.window.by_id({window}):focus()
    operation:on_complete(function() gnoblin.window.by_id({window}):focus() end)
end)
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
    host.pump(Tick::new(1)).expect("the handler calls focus");
    let old = server.graph().pending().next().expect("one call").id();

    fs::write(&init, "gnoblin.configure { cursor = { size = 12 } }\n").expect("a test can rewrite its file");
    let ReloadOutcome::Accepted(run) = host.reload_config(&config.paths()).expect("the reload runs") else {
        panic!("a valid file was refused");
    };
    assert!(run.errors.is_empty(), "{:#?}", run.errors);

    // The new run registers no handler, so the old callback has nothing left to run.
    server
        .complete(old, OperationOutcome::Done)
        .expect("the completion is accepted");
    let pumped = host.pump(Tick::new(1)).expect("the completion is delivered");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert_eq!(
        server.graph().pending().count(),
        0,
        "the old callback did not call focus again"
    );
}

#[test]
fn two_reload_requests_before_one_pump_coalesce_into_one_reload() {
    let (mut server, mut host) = pair();
    let session = session(&mut server);
    host.pump(Tick::new(1)).expect("the session reaches the mirror");

    // The compositor numbers each forwarded request, so the second signal carries the next one.
    let mut first_request = Fields::new();
    first_request.insert("generation".to_owned(), ModelValue::Uint(1));
    server
        .signal(session, "reload_requested", first_request)
        .expect("reload_requested is a signal");
    let mut second_request = Fields::new();
    second_request.insert("generation".to_owned(), ModelValue::Uint(2));
    server
        .signal(session, "reload_requested", second_request)
        .expect("reload_requested is a signal");
    host.pump(Tick::new(2)).expect("the pump delivers both requests");

    assert!(host.take_reload_request(), "the request reached the runtime");
    assert!(
        !host.take_reload_request(),
        "the second request was folded into the first"
    );
}

/// The `window-rules` list of a run's document, as the `app-id` each rule matches.
fn window_rule_app_ids(document: &Value) -> Vec<String> {
    let Some(Value::List(rules)) = document.as_table().and_then(|root| root.get("window-rules")) else {
        return Vec::new();
    };
    rules
        .iter()
        .map(|rule| {
            rule.as_table()
                .and_then(|rule| rule.get("match"))
                .and_then(Value::as_table)
                .and_then(|matcher| matcher.get("app-id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        })
        .collect()
}

#[test]
fn window_rule_appends_after_the_configured_rules_in_call_order() {
    let config = Config::new("window-rule-append");
    config.write(
        "init.lua",
        r#"
gnoblin.configure { ["window-rules"] = { { match = { ["app-id"] = "^first$" }, maximise = true } } }
gnoblin.window_rule { match = { ["app-id"] = "^second$" }, maximise = false }
gnoblin.window_rule { match = { ["app-id"] = "^third$" } }
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(
        window_rule_app_ids(&report.document),
        ["^first$", "^second$", "^third$"]
    );
}

#[test]
fn window_rule_without_a_configured_list_starts_one() {
    let config = Config::new("window-rule-first");
    config.write(
        "init.lua",
        "gnoblin.window_rule { match = { [\"app-id\"] = \"^only$\" }, maximise = true }\n",
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(window_rule_app_ids(&report.document), ["^only$"]);
}

#[test]
fn window_rule_refuses_an_unknown_key_by_name_and_changes_nothing() {
    let config = Config::new("window-rule-unknown-key");
    config.write(
        "init.lua",
        r#"
gnoblin.window_rule { match = { ["app-id"] = "^kept$" } }
local ok, message = pcall(function()
    gnoblin.window_rule { sise = 4 }
end)
assert(not ok, "an unknown key was accepted")
assert(string.find(tostring(message), "window-rules[2].sise", 1, true), tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(window_rule_app_ids(&report.document), ["^kept$"]);
}

#[test]
fn window_rule_refuses_an_effect_this_build_cannot_draw_by_name() {
    let config = Config::new("window-rule-refused-field");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(function()
    gnoblin.window_rule { match = { ["app-id"] = "^x$" }, blur = 4 }
end)
assert(not ok, "a refused field was accepted")
assert(string.find(tostring(message), "window-rules[1].blur", 1, true), tostring(message))
assert(string.find(tostring(message), "not yet built", 1, true) or string.find(tostring(message), "blur pass", 1, true), tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert!(window_rule_app_ids(&report.document).is_empty());
}

#[test]
fn window_rule_refuses_a_pattern_that_is_not_lua_pattern_syntax() {
    let config = Config::new("window-rule-bad-pattern");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(function()
    gnoblin.window_rule { match = { title = "(unclosed" } }
end)
assert(not ok, "a broken pattern was accepted")
assert(string.find(tostring(message), "title", 1, true), tostring(message))
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn window_rule_is_refused_after_the_configuration_loads() {
    let config = Config::new("window-rule-runtime");
    config.write(
        "init.lua",
        r#"
gnoblin.timers.after(1, function()
    local ok, message = pcall(function() gnoblin.window_rule { match = { ["app-id"] = "^late$" } } end)
    assert(not ok, "a runtime window_rule was accepted")
    assert(string.find(tostring(message), "configuration loads", 1, true) ~= nil, tostring(message))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let pumped = host.pump(Tick::new(1000)).expect("the timer runs");
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn window_rule_refuses_the_257th_rule_and_keeps_the_256_it_had() {
    let config = Config::new("window-rule-cap");
    config.write(
        "init.lua",
        r#"
for index = 1, 256 do
    gnoblin.window_rule { match = { ["app-id"] = "^rule" .. index .. "$" } }
end
local ok, message = pcall(function()
    gnoblin.window_rule { match = { ["app-id"] = "^overflow$" } }
end)
assert(not ok, "a 257th rule was accepted")
assert(string.find(tostring(message), "at most 256 window rules", 1, true), tostring(message))
"#,
    );
    let (_server, mut host) = pair_with(Limits {
        handler_ticks: 600_000,
        ..Limits::default()
    });
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let ids = window_rule_app_ids(&report.document);
    assert_eq!(ids.len(), 256);
    assert_eq!(ids.last().map(String::as_str), Some("^rule256$"));
}

#[test]
fn find_returns_only_live_windows_and_alive_turns_false_on_close() {
    let (mut server, mut host) = pair();
    let kept = open_window(&mut server, "Kept");
    let gone = open_window(&mut server, "Gone");
    let config = Config::new("find-live");
    config.write(
        "init.lua",
        &format!(
            r#"
local held = gnoblin.windows.by_id({gone})
assert(held ~= nil and held.alive == true, "the window is alive before close")
local titles = {{}}
for _, window in ipairs(gnoblin.find("Window")) do titles[#titles + 1] = window.title end
table.sort(titles)
assert(#titles == 2 and titles[1] == "Gone" and titles[2] == "Kept", "live windows: " .. table.concat(titles, ","))
gnoblin.timers.after(2, function()
    assert(held.alive == false, "a closed window reports alive == false")
    local left = gnoblin.find("Window")
    assert(#left == 1, "only the kept window is live, found " .. #left)
    assert(left[1] == gnoblin.windows.by_id({kept}), "the live window is the kept one")
end)
"#,
            gone = gone.get(),
            kept = kept.get()
        ),
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    server.close(gone).expect("the window can be closed");
    let pumped = host.pump(Tick::new(5000)).expect("the close is applied");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn find_refuses_a_table_field_the_kind_does_not_declare_by_name() {
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    let config = Config::new("find-unknown-field");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(gnoblin.find, "Window", {no_such_field = true})
assert(not ok, "an undeclared field was accepted")
assert(string.find(tostring(message), "no_such_field", 1, true) ~= nil, "the refusal names the field: " .. tostring(message))
assert(string.find(tostring(message), "app_id", 1, true) ~= nil, "the refusal lists the fields that work: " .. tostring(message))

local ok_table, message_table = pcall(gnoblin.find, "Window", {title = {}})
assert(not ok_table, "a table value was accepted as a filter value")
assert(string.find(tostring(message_table), "title", 1, true) ~= nil, "the refusal names the field: " .. tostring(message_table))
"#,
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn find_refuses_an_unknown_kind_and_names_the_kinds_that_work() {
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    let config = Config::new("find-unknown-kind");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(gnoblin.find, "Frame")
assert(not ok, "an unknown kind was accepted")
assert(string.find(tostring(message), "Window", 1, true) ~= nil, "the refusal lists the kinds: " .. tostring(message))
"#,
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_predicate_error_is_an_error_of_the_find_call() {
    let (mut server, mut host) = pair();
    open_window(&mut server, "First");
    let config = Config::new("find-predicate-error");
    config.write(
        "init.lua",
        r#"
local ok, message = pcall(gnoblin.find, "Window", function() error("predicate boom") end)
assert(not ok, "the predicate error was swallowed")
assert(string.find(tostring(message), "predicate boom", 1, true) ~= nil, "the message: " .. tostring(message))
"#,
    );
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_claimed_request_is_answered_once_and_finishes_answered() {
    let config = Config::new("request-answer");
    config.write(
        "init.lua",
        r#"
local finished = 0
gnoblin.on("gnoblin.request.offered", function(e)
  gnoblin.request.claim(e.request)
  gnoblin.request.answer(e.request, "passphrase")
end)
gnoblin.on("gnoblin.request.finished", function(e)
  finished = finished + 1
  assert(e.completion == "answered", e.completion)
  assert(finished == 1, "finished ran " .. finished .. " times")
end)
"#,
    );
    let (mut server, mut host) = pair();
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let request = server.offer(
        "slot_prompt_secret",
        "Unlock",
        "Enter the passphrase",
        b"keyring",
        Tick::new(100),
    );
    let first = host.pump(Tick::new(1)).expect("the pump runs");
    let second = host.pump(Tick::new(2)).expect("the pump runs");
    assert!(
        first.errors.is_empty() && second.errors.is_empty(),
        "{:#?}",
        (first.errors, second.errors)
    );
    // The offered handler and the finished handler each ran exactly once.
    assert_eq!(first.handlers + second.handlers, 2, "offered and finished, once each");
    assert!(matches!(
        server.finished(request),
        Some(gnoblin_model::RequestCompletion::Answered)
    ));
}

#[test]
fn a_repeat_claim_by_the_holder_is_accepted_and_keeps_the_request_claimed() {
    let config = Config::new("request-taken");
    config.write(
        "init.lua",
        r#"
gnoblin.on("gnoblin.request.offered", function(e)
  gnoblin.request.claim(e.request)
  local ok, err = pcall(gnoblin.request.claim, e.request)
  -- The holder repeating its own claim is idempotent: the broker keeps the request claimed.
  assert(ok, tostring(err))
end)
"#,
    );
    let (mut server, mut host) = pair();
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let request = server.offer(
        "slot_prompt_secret",
        "Unlock",
        "Enter the passphrase",
        b"keyring",
        Tick::new(100),
    );
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert_eq!(pumped.handlers, 1);
    assert!(
        server.finished(request).is_none(),
        "the first claim still holds the request"
    );
}

#[test]
fn an_answer_before_the_claim_is_refused_as_taken() {
    let config = Config::new("request-unclaimed");
    config.write(
        "init.lua",
        r#"
gnoblin.on("gnoblin.request.offered", function(e)
  local ok, err = pcall(gnoblin.request.answer, e.request, "early")
  assert(not ok, "an unclaimed request accepted an answer")
  assert(string.find(tostring(err), "request-taken", 1, true), tostring(err))
end)
"#,
    );
    let (mut server, mut host) = pair();
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let request = server.offer(
        "slot_prompt_secret",
        "Unlock",
        "Enter the passphrase",
        b"keyring",
        Tick::new(100),
    );
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert!(server.finished(request).is_none(), "a refused answer finishes nothing");
}

#[test]
fn an_answer_after_the_request_finished_is_refused_as_finished() {
    let config = Config::new("request-finished");
    config.write(
        "init.lua",
        r#"
gnoblin.on("gnoblin.request.offered", function(e)
  gnoblin.request.claim(e.request)
  gnoblin.request.answer(e.request, "first")
  local ok, err = pcall(gnoblin.request.answer, e.request, "second")
  assert(not ok, "a finished request accepted a second answer")
  assert(string.find(tostring(err), "request-finished", 1, true), tostring(err))
end)
"#,
    );
    let (mut server, mut host) = pair();
    session(&mut server);
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    let request = server.offer(
        "slot_prompt_secret",
        "Unlock",
        "Enter the passphrase",
        b"keyring",
        Tick::new(100),
    );
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    assert!(matches!(
        server.finished(request),
        Some(gnoblin_model::RequestCompletion::Answered)
    ));
}

/// Pump until a handler runs or reports, because a child process ends on its own clock.
fn pump_until_handled(host: &mut Host<Loopback>, tick: &mut u64) -> gnoblin_lua::PumpReport {
    for _ in 0..1000 {
        *tick += 10;
        let report = host.pump(Tick::new(*tick)).expect("a pump");
        if report.handlers > 0 || !report.errors.is_empty() {
            return report;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("no process completion reached the pump");
}

#[test]
fn a_missing_program_completes_its_wait_with_not_found() {
    let config = Config::new("process-missing");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"/nonexistent/gnoblin-test-program"})
job:wait():on_complete(function(value, err)
    assert(value == nil, "a program that did not start has no value")
    assert(err.code == "not_found", "expected not_found, got " .. tostring(err.code))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_started_process_names_its_pid_and_a_missing_one_has_none() {
    let config = Config::new("process-pid");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"true"})
assert(type(job.pid) == "number" and job.pid > 0, "a started process has a pid")
local missing = gnoblin.process.spawn({"/nonexistent/gnoblin-test-program"})
assert(missing.pid == nil, "a program that did not start has no pid")
job:wait():on_complete(function(_, err)
    assert(err == nil, "true ran")
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn captured_output_is_cut_at_output_limit_and_says_so() {
    let config = Config::new("process-capture");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"sh", "-c", "printf abcdefgh"}, {stdout = "capture", output_limit = 4})
job:wait():on_complete(function(value, err)
    assert(err == nil, "the program ran")
    assert(value.exit_code == 0, "exit status " .. tostring(value.exit_code))
    assert(value.stdout == "abcd", "kept " .. tostring(value.stdout))
    assert(value.stdout_truncated == true, "truncation was not reported")
    assert(value.stderr == "", "stderr was not captured")
    assert(value.stderr_truncated == false, "stderr was not truncated")
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn stdin_is_written_then_closed_so_the_program_ends() {
    let config = Config::new("process-stdin");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"cat"}, {stdin = "hello", stdout = "capture"})
job:wait():on_complete(function(value, err)
    assert(err == nil, "cat ran")
    assert(value.stdout == "hello", "cat echoed " .. tostring(value.stdout))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_timeout_ends_the_process_and_reports_it_as_timed_out() {
    let config = Config::new("process-timeout");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"sleep", "30"}, {timeout = 0.05})
job:wait():on_complete(function(value, err)
    assert(err == nil, "the program ran until the timeout")
    assert(value.timed_out == true, "the timeout was not reported")
    assert(value.signal == "term", "signal was " .. tostring(value.signal))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn killing_an_ended_process_fails_with_not_found() {
    let config = Config::new("process-kill-ended");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"true"})
job:wait():on_complete(function(value, err)
    assert(err == nil, "true ran")
    local kill = job:kill()
    kill:on_complete(function(_, kill_err)
        assert(kill_err ~= nil and kill_err.code == "not_found", "kill after exit must fail")
    end)
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert_eq!(pumped.handlers, 1, "the kill completion runs once");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn spawn_arguments_are_refused() {
    let config = Config::new("process-refused");
    config.write(
        "init.lua",
        r#"
assert(not pcall(gnoblin.process.spawn, {}), "an empty argv was accepted")
assert(not pcall(gnoblin.process.spawn, {"true", 1}), "a non-string argument was accepted")
assert(not pcall(gnoblin.process.spawn, {"true"}, {timeout = 0}), "a zero timeout was accepted")
assert(not pcall(gnoblin.process.spawn, {"true"}, {output_limit = -1}), "a negative limit was accepted")
assert(not pcall(gnoblin.process.spawn, {"true"}, {stdout = "pipe"}), "an unknown stream mode was accepted")
assert(not pcall(gnoblin.process.spawn, {"true"}, {cwd = "relative"}), "a relative cwd was accepted")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn a_process_reports_its_fields_and_how_it_ended() {
    let config = Config::new("process-fields");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"sh", "-c", "exit 3"})
assert(job.alive == true, "a started process is alive")
assert(job.status == "running", "status was " .. tostring(job.status))
assert(job.exit_code == nil and job.signal == nil, "nothing has ended yet")
assert(job.argv[1] == "sh" and job.argv[3] == "exit 3", "argv is the command line")
assert(job.revision == 1, "revision starts at 1")
job:watch("status", function(p, value, old)
    assert(value == "exited" and old == "running", "status moved " .. tostring(old) .. " -> " .. tostring(value))
end)
job:on("closed", function(p)
    assert(p.alive == false, "an ended process is not alive")
    assert(p.status == "exited", "status was " .. tostring(p.status))
    assert(p.exit_code == 3, "exit_code was " .. tostring(p.exit_code))
    assert(p.signal == nil, "a normal exit has no signal")
    assert(p.revision == 2, "one batch of changes counts once, was " .. tostring(p.revision))
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn changed_names_the_fields_that_moved_and_a_killed_process_names_its_signal() {
    let config = Config::new("process-changed");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"sleep", "30"})
job:on("changed", function(p, changed)
    local names = {}
    for _, name in ipairs(changed) do names[name] = true end
    assert(names.status and names.alive, "status and alive moved")
    assert(not names.argv and not names.id, "argv and id never move")
    assert(p.signal == "kill", "signal was " .. tostring(p.signal))
end)
job:kill("kill")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn a_failed_start_is_not_alive_and_takes_no_listeners() {
    let config = Config::new("process-failed-start");
    config.write(
        "init.lua",
        r#"
local missing = gnoblin.process.spawn({"/nonexistent/gnoblin-test-program"})
assert(missing.alive == false, "a program that did not start is not alive")
assert(missing.status == "failed", "status was " .. tostring(missing.status))
assert(not pcall(missing.on, missing, "closed", function() end), "a failed start took a listener")
assert(not pcall(missing.watch, missing, "pid", function() end), "a failed start took a watcher")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

#[test]
fn watch_refuses_fixed_fields_and_listening_refuses_an_ended_process() {
    let config = Config::new("process-refuse-listen");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"true"})
assert(not pcall(job.watch, job, "id", function() end), "id is fixed and was watchable")
assert(not pcall(job.watch, job, "argv", function() end), "argv is fixed and was watchable")
assert(not pcall(job.watch, job, "alive", function() end), "alive is fixed and was watchable")
assert(not pcall(job.watch, job, "unknown", function() end), "an unknown field was watchable")
assert(not pcall(job.on, job, "exit", function() end), "an unknown signal was accepted")
job:on("closed", function(p)
    assert(not pcall(p.on, p, "changed", function() end), "an ended process took a listener")
    assert(not pcall(p.watch, p, "status", function() end), "an ended process took a watcher")
end)
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn the_list_holds_running_processes_and_by_id_finds_them_again() {
    let config = Config::new("process-list");
    config.write(
        "init.lua",
        r#"
local job = gnoblin.process.spawn({"sleep", "30"})
local running = gnoblin.process.list()
assert(#running == 1 and running[1] == job, "the running process is listed")
assert(gnoblin.process.by_id(job.id) == job, "by_id returns the same object")
assert(gnoblin.process.by_id(job.id + 1000) == nil, "an unknown id has no process")
job:on("closed", function(p)
    assert(#gnoblin.process.list() == 0, "an ended process leaves the list")
end)
job:kill("kill")
"#,
    );
    let (_server, mut host) = pair();
    let report = host.run_config(&config.paths()).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut tick = 0;
    let pumped = pump_until_handled(&mut host, &mut tick);
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

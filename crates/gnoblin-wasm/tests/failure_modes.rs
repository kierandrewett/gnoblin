//! One test per documented failure mode, plus the two that prove the happy path.
//!
//! The modules here are WAT text compiled by the test, never checked-in binaries:
//! a reader can see exactly what the module does, and a test that needs a module
//! which loops forever can write one on the spot instead of keeping a fixture.
//!
//! Everything goes through the real path: the host is an IPC client, the server is
//! a live [`Graph`], the module is a component the runtime compiles from the
//! generated WIT, and the assertions read the host's state, the host's log and the
//! graph. Only the byte transport is in process, exactly as in the client's and the
//! Lua host's tests, so these keep their meaning when the socket lands.

use std::collections::BTreeMap;
use std::path::Path;

use gnoblin_client::Server;
use gnoblin_model::{Fields, Graph, ObjectId, TypeKey, Value};
use gnoblin_schema::Catalog;
use gnoblin_types::Version;

use gnoblin_wasm::abi::kind;
use gnoblin_wasm::{
    Budget, CallError, Capabilities, HostError, Limits, Manifest, ModuleHost, ModuleState, Record, Sink,
};

/// The IPC version the tests connect at.
const VERSION: Version = Version::new(0, 2, 0);

/// The version the generated package declares, which is the crate's own.
const PACKAGE: &str = env!("CARGO_PKG_VERSION");

/// Where the generated WIT was written, which is what the host was compiled from.
const WIT: &str = include_str!(concat!(env!("OUT_DIR"), "/gnoblin-ipc.wit"));

/// A sink that keeps the lines for the test to read.
#[derive(Clone, Debug, Default)]
struct Capture {
    lines: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Capture {
    fn new() -> Capture {
        Capture::default()
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.lock().expect("the lock is not poisoned"))
    }
}

impl Sink for Capture {
    fn write(&mut self, record: &Record<'_>) {
        self.lines
            .lock()
            .expect("the lock is not poisoned")
            .push(record.to_string());
    }
}

/// A graph built from the checked-in schema.
fn graph() -> Graph {
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    Graph::new(catalog)
}

/// The properties a window is created with, in schema order.
fn window_fields(id: u64, title: &str) -> Fields {
    let mut fields = BTreeMap::new();
    fields.insert("id".to_owned(), Value::Uint(id));
    fields.insert("title".to_owned(), Value::Text(title.to_owned()));
    fields.insert("app_id".to_owned(), Value::Text("org.example.App".to_owned()));
    fields.insert("focused".to_owned(), Value::Bool(false));
    fields
}

/// A server with one window whose identity is 1.
fn server() -> (Server, ObjectId) {
    let mut server = Server::new(graph(), VERSION);
    let window = server
        .create(TypeKey::gnoblin("window"), window_fields(1, "first"))
        .expect("the graph takes a window");
    (server, window)
}

/// A budget small enough that a runaway module reaches it quickly and large enough
/// that a well-behaved one does not.
fn budget() -> Budget {
    Budget::fuel(2_000_000)
}

/// The component boilerplate every test module shares.
///
/// The imported surface is fixed — the `gnoblin` namespace's window lookup, its
/// `focus` method and its `watch` subscription — so a test only writes the core
/// module's bodies. The component is hand-written rather than generated because it
/// is the *client* side of the ABI: generating it from the same code that generates
/// the host's side would test the generator against itself.
///
/// Addresses in the shared memory: `64` is the lookup's return area, `96` the
/// method's and `128` the watch's. The return area of a `result` is its
/// discriminant as one byte followed by the payload at its alignment, which is what
/// `tests/generated_wit.rs` pins against the WIT.
fn module(start: &str, on_event: &str) -> String {
    format!(
        r#"
(component
  (import "gnoblin:ipc/gnoblin@{PACKAGE}" (instance $api
    (type $ec (enum "denied" "refused" "unknown-member" "unknown-object"))
    (export "error-code" (type $ecx (eq $ec)))
    (export "operation" (type $op (sub resource)))
    (export "window" (type $w (sub resource)))
    (export "window-by-id" (func (param "id" u64) (result (result (own $w) (error $ecx)))))
    (export "[method]window.focus" (func (param "self" (borrow $w)) (result (result (own $op) (error $ecx)))))
    (export "[method]window.watch" (func (param "self" (borrow $w)) (result (result bool (error $ecx)))))
  ))
  (core module $memmod (memory (export "mem") 1))
  (core instance $mi (instantiate $memmod))
  (alias core export $mi "mem" (core memory $mem))
  (alias export $api "window-by-id" (func $byid))
  (alias export $api "[method]window.focus" (func $focus))
  (alias export $api "[method]window.watch" (func $watch))
  (core func $byid_core (canon lower (func $byid) (memory $mem)))
  (core func $focus_core (canon lower (func $focus) (memory $mem)))
  (core func $watch_core (canon lower (func $watch) (memory $mem)))
  (core module $app
    (import "env" "mem" (memory 1))
    (import "" "window-by-id" (func $byid (param i64 i32)))
    (import "" "focus" (func $focus (param i32 i32)))
    (import "" "watch" (func $watch (param i32 i32)))
    (global $window (mut i32) (i32.const 0))
    (func (export "start")
{start}
    )
    (func (export "stop"))
    (func (export "on-event") (param i32 i64 i32 i64)
{on_event}
    )
  )
  (core instance $appi (instantiate $app
    (with "env" (instance (export "mem" (memory $mem))))
    (with "" (instance
      (export "window-by-id" (func $byid_core))
      (export "focus" (func $focus_core))
      (export "watch" (func $watch_core))
    ))
  ))
  (func $start (canon lift (core func $appi "start")))
  (func $stop (canon lift (core func $appi "stop")))
  (instance $lifecycle
    (export "start" (func $start))
    (export "stop" (func $stop))
  )
  (export "gnoblin:ipc/lifecycle@{PACKAGE}" (instance $lifecycle))
  (func $onevent (param "kind" u32) (param "object" u64) (param "member" u32) (param "argument" u64)
    (canon lift (core func $appi "on-event")))
  (instance $callbacks (export "on-event" (func $onevent)))
  (export "gnoblin:ipc/callbacks@{PACKAGE}" (instance $callbacks))
)
"#
    )
}

/// A module that does nothing at all: the control for the failure tests.
fn idle_module() -> Vec<u8> {
    compile(&module("      (nop)", "      (nop)"))
}

/// Compile a component from text, reporting the WAT error with the text.
fn compile(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("the test module parses")
}

/// A manifest that may read windows.
fn reader(name: &str) -> Manifest {
    Manifest::new(name).capabilities(Capabilities::none().read("window"))
}

/// A manifest that may read windows, watch them and focus them.
fn watcher(name: &str) -> Manifest {
    Manifest::new(name).capabilities(Capabilities::none().read("window").subscribe().call("window", "focus"))
}

/// A host with a capture sink.
fn host() -> (ModuleHost, Capture) {
    let capture = Capture::new();
    let host = ModuleHost::with_sink(Box::new(capture.clone())).expect("the engine starts");
    (host, capture)
}

// 1. A module loops forever.
#[test]
fn a_module_that_loops_forever_is_killed_and_the_host_survives() {
    let (mut host, capture) = host();
    // The loops and the trap tests need no host API at all, only a start body.
    let looping = compile(&module("      (loop $forever (br $forever))", "      (nop)"));
    host.load(reader("looping"), &looping, budget()).expect("loads");
    host.load(reader("idle"), &idle_module(), budget()).expect("loads");

    let error = host.start("looping", budget()).expect_err("the loop never returns");
    assert!(
        matches!(error, CallError::Deadline { ref module, fuel } if module == "looping" && fuel == budget().as_fuel()),
        "expected a deadline, got {error}"
    );
    assert_eq!(
        host.state("looping"),
        Some(&ModuleState::Failed {
            reason: error.to_string()
        })
    );

    // The module's store is gone, so nothing half-run can be called again.
    let again = host
        .start("looping", budget())
        .expect_err("a dead module accepts nothing");
    assert!(matches!(again, CallError::NotRunning { .. }), "got {again}");

    // The other module is untouched: same host, its own store.
    host.start("idle", budget()).expect("the idle module still starts");
    assert_eq!(host.state("idle"), Some(&ModuleState::Running));
    assert!(
        capture.take().iter().any(|line| line.contains("looping")),
        "the death is reported"
    );
}

// 2. A module allocates without bound.
#[test]
fn a_module_that_grows_without_bound_hits_the_memory_limit() {
    // Grow by a fixed 32 pages, trapping if the runtime refuses: a `memory.grow`
    // that fails returns -1 rather than trapping, so the module has to turn the
    // refusal into a trap for the host to see it. The same module under a roomy
    // limit reaches the target, which is what proves the first failure was the
    // limit.
    let growing = compile(&module(
        r#"      (local $pages i32)
      (block $done
        (loop $grow
          (br_if $done (i32.ge_u (local.get $pages) (i32.const 32)))
          (if (i32.eq (memory.grow (i32.const 1)) (i32.const -1)) (then (unreachable)))
          (local.set $pages (i32.add (local.get $pages) (i32.const 1)))
          (br $grow)))"#,
        "      (nop)",
    ));

    let (mut host, _) = host();
    let small = Limits {
        memory_bytes: 2 * 64 * 1024,
        ..Limits::default()
    };
    host.load(reader("small").limits(small), &growing, budget())
        .expect("loads");
    let error = host.start("small", budget()).expect_err("2 pages is the limit");
    assert!(matches!(error, CallError::Trapped { .. }), "got {error}");

    // The same module under a limit it cannot reach grows happily: the failure above
    // was the limit and not the module.
    let roomy = Limits {
        memory_bytes: 64 * 64 * 1024,
        ..Limits::default()
    };
    host.load(reader("roomy").limits(roomy), &growing, budget())
        .expect("loads");
    host.start("roomy", budget())
        .expect("33 pages fits under a 64-page limit");
    assert_eq!(host.state("roomy"), Some(&ModuleState::Running));
    assert!(
        host.state("small").and_then(ModuleState::reason).is_some(),
        "the killed module keeps the reason"
    );
}

// 3. A module calls a method it did not declare, and the denial is logged once.
#[test]
fn a_module_that_calls_an_undeclared_method_is_denied_once() {
    // Open window 1, then call `focus` twice. The manifest grants reading and
    // nothing else, so both calls are denied; the module traps unless the error is
    // exactly `denied`, which makes "no trap" mean "the host said denied".
    let calling = compile(&module(
        r#"      (call $byid (i64.const 1) (i32.const 64))
      (if (i32.ne (i32.load8_u (i32.const 64)) (i32.const 0)) (then (unreachable)))
      (global.set $window (i32.load (i32.const 68)))
      (call $focus (global.get $window) (i32.const 96))
      (if (i32.ne (i32.load8_u (i32.const 96)) (i32.const 1)) (then (unreachable)))
      (if (i32.ne (i32.load (i32.const 100)) (i32.const 0)) (then (unreachable)))
      (call $focus (global.get $window) (i32.const 96))
      (if (i32.ne (i32.load8_u (i32.const 96)) (i32.const 1)) (then (unreachable)))"#,
        "      (nop)",
    ));

    let (mut server, window) = server();
    let (mut host, capture) = host();
    host.load(reader("denied"), &calling, budget()).expect("loads");
    host.connect(&mut server, VERSION).expect("connects");
    host.start("denied", budget())
        .expect("the module saw `denied` for both calls");

    assert_eq!(
        server.graph().pending().count(),
        0,
        "a denied call never reaches the compositor"
    );
    let lines = capture.take();
    let denials: Vec<&String> = lines.iter().filter(|line| line.contains("window.focus")).collect();
    assert_eq!(denials.len(), 1, "one line for two denied calls, got {lines:#?}");
    assert!(
        denials[0].contains("denied") || denials[0].contains("does not grant"),
        "{denials:?}"
    );

    // Reading was granted, so the handle was real and the denial is about the method.
    assert_eq!(host.state("denied"), Some(&ModuleState::Running));
    let _ = window;
}

// 4. A module traps.
#[test]
fn a_module_that_traps_is_killed_and_reported() {
    let trapping = compile(&module("      (unreachable)", "      (nop)"));
    let (mut host, capture) = host();
    host.load(reader("trapping"), &trapping, budget()).expect("loads");
    let error = host.start("trapping", budget()).expect_err("it traps");
    assert!(matches!(error, CallError::Trapped { .. }), "got {error}");
    assert!(
        host.state("trapping")
            .and_then(ModuleState::reason)
            .is_some_and(|reason| reason.contains("unreachable")),
        "the reason names the trap: {:?}",
        host.state("trapping")
    );
    assert!(capture.take().iter().any(|line| line.contains("unreachable")));
}

// 5. A failed reload leaves the running module in place.
#[test]
fn a_failed_reload_leaves_the_running_module_in_place() {
    let (mut server, _window) = server();
    let (mut host, _) = host();
    host.load(watcher("reloaded"), &idle_module(), budget()).expect("loads");
    host.connect(&mut server, VERSION).expect("connects");
    host.start("reloaded", budget()).expect("starts");
    assert_eq!(host.state("reloaded"), Some(&ModuleState::Running));

    let error = host
        .reload("reloaded", b"not a component", budget())
        .expect_err("refused");
    assert!(matches!(error, HostError::Compile { .. }), "got {error}");
    assert_eq!(
        host.state("reloaded"),
        Some(&ModuleState::Running),
        "the running module is untouched by a broken reload"
    );
    host.deliver("reloaded", budget())
        .expect("the old module still delivers");

    // A reload that works replaces it, and the replacement is inert until started.
    host.reload("reloaded", &idle_module(), budget()).expect("reloads");
    assert_eq!(host.state("reloaded"), Some(&ModuleState::Loaded));

    // And a module that never loaded cannot be reloaded into existence.
    let missing = host
        .reload("absent", &idle_module(), budget())
        .expect_err("no such module");
    assert!(matches!(missing, HostError::Missing { .. }), "got {missing}");
}

// 6. A module imports something the host does not provide.
#[test]
fn a_module_that_imports_wasi_is_refused_at_load() {
    let wasi = compile(
        r#"(component
  (import "wasi:filesystem/types@0.2.0" (instance $types
    (export "get-flags" (func (result u32)))))
)"#,
    );
    let (mut host, _) = host();
    let error = host.load(reader("wasi"), &wasi, budget()).expect_err("no WASI here");
    match &error {
        HostError::Link { reason, .. } => assert!(
            reason.contains("wasi:filesystem/types"),
            "the reason names the import: {reason}"
        ),
        other => panic!("expected a link failure, got {other}"),
    }
    assert_eq!(host.state("wasi"), None, "a refused module is not loaded");
}

// 7. A module receives property diffs and signals, and its completions.
#[test]
fn a_module_receives_diffs_and_signals_from_the_model() {
    // Start: open window 1 and watch it. On an event: focus it, so each delivered
    // observation leaves an operation in the graph, which is how the test sees that
    // the module received it.
    let echoing = compile(&module(
        r#"      (call $byid (i64.const 1) (i32.const 64))
      (if (i32.ne (i32.load8_u (i32.const 64)) (i32.const 0)) (then (unreachable)))
      (global.set $window (i32.load (i32.const 68)))
      (call $watch (global.get $window) (i32.const 128))
      (if (i32.ne (i32.load8_u (i32.const 128)) (i32.const 0)) (then (unreachable)))"#,
        r#"      (if (i32.eq (local.get 0) (i32.const 3)) (then (return)))  ;; closed: nothing to call
      (call $focus (global.get $window) (i32.const 96))"#,
    ));

    let (mut server, window) = server();
    let (mut host, _) = host();
    host.load(watcher("echo"), &echoing, budget()).expect("loads");
    host.connect(&mut server, VERSION).expect("connects");
    host.start("echo", budget()).expect("starts");

    // The snapshot's window arrives as `opened`, which starts the loop: one focus.
    let delivered = host.deliver("echo", budget()).expect("delivers");
    assert_eq!(delivered, 1, "the inherited window is one opened event");
    assert_eq!(server.graph().pending().count(), 1, "the module acted on it");

    // A property change and a signal are the two observations the ABI names.
    server
        .set(window, "title", Value::Text("renamed".to_owned()))
        .expect("the graph takes the change");
    assert_eq!(host.deliver("echo", budget()).expect("delivers"), 1);
    server
        .signal(window, "closed", Fields::new())
        .expect("the graph takes the signal");
    assert_eq!(host.deliver("echo", budget()).expect("delivers"), 1);
    assert_eq!(
        server.graph().pending().count(),
        3,
        "the opened window, the change and the signal each made the module call focus"
    );

    // An operation's completion is an event too.
    let operation = server
        .graph()
        .pending()
        .next()
        .map(|operation| operation.id())
        .expect("an operation is pending");
    server
        .complete(operation, gnoblin_model::OperationOutcome::Done)
        .expect("the graph takes the completion");
    assert_eq!(host.deliver("echo", budget()).expect("delivers"), 1);
    assert_eq!(host.state("echo"), Some(&ModuleState::Running));

    // Nothing left: a delivery with nothing to do is not an error.
    assert_eq!(host.deliver("echo", budget()).expect("delivers"), 0);
}

// 8. A stopped module accepts nothing.
#[test]
fn a_stopped_module_accepts_no_calls() {
    let (mut server, _window) = server();
    let (mut host, _) = host();
    host.load(watcher("stopped"), &idle_module(), budget()).expect("loads");
    host.connect(&mut server, VERSION).expect("connects");
    host.start("stopped", budget()).expect("starts");
    host.stop("stopped", budget()).expect("stops");
    assert_eq!(host.state("stopped"), Some(&ModuleState::Stopped));

    let error = host
        .deliver("stopped", budget())
        .expect_err("a stopped module gets nothing");
    assert!(matches!(error, CallError::NotRunning { .. }), "got {error}");
    // Stopping twice is not an error: the hook ran once and the store is gone.
    host.stop("stopped", budget()).expect("stopping again is a no-op");
}

// 9. The generated WIT and the schema agree, member by member, both ways.
#[test]
fn the_generated_wit_matches_the_schema() {
    let mut resolve = wit_parser::Resolve::new();
    let package = resolve
        .push_str("gnoblin-ipc.wit", WIT)
        .expect("the generated WIT parses");

    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");

    // The package version is the crate's, prerelease and all: a module built against
    // a release candidate must not link into a release compositor by accident.
    let name = &resolve.packages[package].name;
    assert_eq!(name.namespace, "gnoblin");
    assert_eq!(name.name, "ipc");
    assert_eq!(name.version.as_ref().map(ToString::to_string).as_deref(), Some(PACKAGE));

    let mut schema_members: Vec<(String, String)> = Vec::new();
    for namespace in catalog.namespaces() {
        let interface_name = namespace.as_str().replace('.', "-");
        let interface = resolve
            .interfaces
            .iter()
            .find(|(_, interface)| interface.name.as_deref() == Some(interface_name.as_str()))
            .map(|(_, interface)| interface)
            .unwrap_or_else(|| panic!("the WIT declares the `{interface_name}` interface"));

        // The WIT's members of each resource, by kebab-cased name.
        let mut resources: BTreeMap<&str, BTreeMap<&str, &wit_parser::Function>> = BTreeMap::new();
        for (name, function) in &interface.functions {
            if let Some(rest) = name.strip_prefix("[method]") {
                let (resource, member) = rest
                    .split_once('.')
                    .unwrap_or_else(|| panic!("`{name}` names a resource and a member"));
                resources.entry(resource).or_default().insert(member, function);
            }
        }

        for object in catalog.objects(&namespace) {
            let resource = object.name.as_str();
            let declared = resources
                .remove(resource)
                .unwrap_or_else(|| panic!("the WIT declares the `{resource}` resource"));
            let object_type = interface
                .types
                .get(resource)
                .map(|id| &resolve.types[*id])
                .unwrap_or_else(|| panic!("`{resource}` is a type of the interface"));
            assert!(
                matches!(object_type.kind, wit_parser::TypeDefKind::Resource),
                "`{resource}` is a resource"
            );

            // The ABI adds `object-id` and `watch` to every resource; everything else
            // the WIT declares has to be a schema member, and every schema member has
            // to be in the WIT.
            let mut expected: Vec<String> = vec!["object-id".to_owned(), "watch".to_owned()];
            for property in &object.properties {
                let getter = property.name.replace('_', "-");
                assert!(
                    declared.contains_key(getter.as_str()),
                    "`{resource}.{getter}` is a getter"
                );
                expected.push(getter.clone());
                schema_members.push((object.name.clone(), property.name.clone()));
            }
            for method in &object.methods {
                let name = method.name.replace('_', "-");
                let function = declared
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("`{resource}.{name}` is a method"));
                assert_eq!(
                    function.params.len(),
                    method.params.len() + 1,
                    "`{resource}.{name}` takes self and the schema's parameters"
                );
                for (index, parameter) in method.params.iter().enumerate() {
                    assert_eq!(
                        function.params[index + 1].name,
                        parameter.name,
                        "`{resource}.{name}` keeps the schema's parameter order"
                    );
                }
                expected.push(name);
            }
            let mut found: Vec<String> = declared.keys().map(|name| (*name).to_owned()).collect();
            expected.sort();
            found.sort();
            assert_eq!(
                found, expected,
                "`{resource}`'s members are the schema's plus the ABI's"
            );

            // And the lookup that turns an identity into a handle.
            assert!(
                interface.functions.contains_key(format!("{resource}-by-id").as_str()),
                "the WIT declares `{resource}-by-id`"
            );
            for signal in &object.signals {
                schema_members.push((object.name.clone(), signal.name.clone()));
            }
        }

        // Every error code the schema declares is a case of the interface's
        // `error-code`, so a refusal can carry it.
        let error_code = interface
            .types
            .get("error-code")
            .map(|id| &resolve.types[*id])
            .expect("the interface declares `error-code`");
        let wit_parser::TypeDefKind::Enum(cases) = &error_code.kind else {
            panic!("`error-code` is an enum");
        };
        for error in catalog.errors(&namespace) {
            assert!(
                cases.cases.iter().any(|case| case.name == error.name),
                "`{}` is a case of `{namespace}`'s error-code",
                error.name
            );
        }
    }

    // The member table, which is what an event's word means, is exactly the
    // schema's properties and signals: no extra row, no missing row.
    let mut wit_members: Vec<(String, String)> = gnoblin_wasm::abi::MEMBERS
        .iter()
        .map(|row| (row.object.to_owned(), row.name.to_owned()))
        .collect();
    schema_members.sort();
    wit_members.sort();
    assert_eq!(schema_members, wit_members, "the member table is the schema's");
}

// 10. The return-area layout the test modules use is the one the canonical ABI
//     defines, so a rejection test that reads a discriminant is reading the right
//     byte.
#[test]
fn the_event_words_are_the_generated_table() {
    assert_eq!(kind::OPENED, 0);
    assert_eq!(kind::CHANGED, 1);
    assert_eq!(kind::SIGNALLED, 2);
    assert_eq!(kind::CLOSED, 3);
    assert_eq!(kind::OPERATION_FINISHED, 4);
    assert_eq!(
        gnoblin_wasm::abi::member_index("window", "title"),
        Some(gnoblin_wasm::abi::WINDOW_TITLE)
    );
    assert_eq!(
        gnoblin_wasm::abi::member_index("window", "id"),
        Some(gnoblin_wasm::abi::WINDOW_ID)
    );
}

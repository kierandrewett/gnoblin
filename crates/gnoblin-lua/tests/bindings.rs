//! The generated bindings and the generated stub, checked against the schema.
//!
//! Both are produced by `build.rs` from `crates/gnoblin-schema/schema`, so the test
//! re-reads the schema and compares member by member. The runtime comparison is
//! exact: asking an instance for a name it does not have returns the list of names
//! it does have, so one `pcall` per object type compares the whole member set in
//! both directions rather than sampling a few names.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use gnoblin_client::{Loopback, Server};
use gnoblin_config::SearchPaths;
use gnoblin_model::{Fields, Graph, ObjectId, Tick, TypeKey};
use gnoblin_proto::Version;
use gnoblin_schema::{Catalog, ObjectType, Type};

use gnoblin_lua::{Host, Limits};

const VERSION: Version = Version::new(0, 2, 0);

fn catalog() -> Catalog {
    Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads")
}

/// A configuration directory holding one script, removed when the test ends.
struct Config {
    root: std::path::PathBuf,
}

impl Config {
    fn new(name: &str) -> Config {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("gnoblin-lua-bindings-{}-{name}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("a test can create its configuration directory");
        Config { root }
    }

    fn script(&self, text: &str) -> SearchPaths {
        fs::write(self.root.join("init.lua"), text).expect("a test can write a file");
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

/// The Lua expression that reaches one object type's class table: core types sit on
/// `gnoblin`, extension types on the table `require` returns for their namespace.
fn class_path(namespace: &str, object: &str) -> String {
    if namespace == "gnoblin" {
        format!("gnoblin.{object}")
    } else {
        format!("require(\"{namespace}\").{object}")
    }
}

/// One string literal for the generated script.
fn lua_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Every member an instance accepts, sorted the way a refusal spells them.
fn expected_members(object: &ObjectType) -> Vec<String> {
    let mut members: Vec<String> = object.properties.iter().map(|property| property.name.clone()).collect();
    members.extend(object.methods.iter().map(|method| method.name.clone()));
    members.push("on".to_owned());
    members.sort();
    members
}

#[test]
fn the_stub_matches_the_schema() {
    let catalog = catalog();
    let stub = gnoblin_lua::stub();
    assert!(stub.contains("---@meta"), "the stub is a LuaCATS file");
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            let class = format!("{}.{}", namespace.as_str(), object.name);
            assert!(
                stub.contains(&format!("---@class {class}")),
                "the stub has no class for {class}"
            );
            for property in &object.properties {
                assert!(
                    stub.contains(&format!("---@field {} ", property.name)),
                    "the stub has no field for {class}.{}",
                    property.name
                );
            }
            for method in &object.methods {
                assert!(
                    stub.contains(&format!("function {}:{}(", object.name, method.name)),
                    "the stub has no method {class}:{}",
                    method.name
                );
                for parameter in &method.params {
                    assert!(
                        stub.contains(&format!("---@param {} ", parameter.name)),
                        "the stub has no parameter {} of {class}:{}",
                        parameter.name,
                        method.name
                    );
                }
            }
            for signal in &object.signals {
                assert!(
                    stub.contains(&format!("---@param name \"{}\"", signal.name)),
                    "the stub has no signal {class}:{}",
                    signal.name
                );
            }
            // Each namespace is a module class: `require` answers with it, and it
            // carries one collection field per object type.
            assert!(
                stub.contains(&format!(
                    "---@field {} {}.{}Collection",
                    object.name,
                    namespace.as_str(),
                    object.name
                )),
                "the stub has no collection field for {class}"
            );
        }
    }
    // A name no schema declares must not appear as a class.
    assert!(!stub.contains("---@class gnoblin.nonexistent"));
}

#[test]
fn the_stub_can_be_written_where_a_caller_asks() {
    let dir = Config::new("stub");
    let path = gnoblin_lua::write_stub(&dir.root).expect("the stub is written");
    assert_eq!(path, dir.root.join("gnoblin.d.lua"));
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        gnoblin_lua::stub()
    );
}

/// One value of a schema type, for a test that has to fill every property.
fn value_for(ty: &Type, created: &[(String, String, ObjectId)]) -> gnoblin_model::Value {
    use gnoblin_model::Value;
    match ty {
        Type::Bool => Value::Bool(true),
        Type::Int => Value::Int(0),
        Type::Uint => Value::Uint(1),
        Type::Double => Value::Double(0.0),
        Type::Text => Value::Text("a value".to_owned()),
        Type::Bytes => Value::Bytes(vec![1, 2, 3]),
        Type::List(inner) => Value::List(vec![value_for(inner, created)]),
        Type::ObjectRef(name) => {
            let (_, _, id) = created
                .iter()
                .find(|(_, object, _)| object == name)
                .unwrap_or_else(|| panic!("the test creates `{name}` before a property that refers to it"));
            Value::Object(*id)
        }
    }
}

#[test]
fn the_installed_tables_match_the_schema() {
    let catalog = catalog();
    let mut server = Server::new(Graph::new(catalog.clone()), VERSION);
    let mut live: Vec<(String, String, ObjectId)> = Vec::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            // The graph requires every declared property, so the test builds one
            // value per declared property from its schema type.
            let mut properties = Fields::new();
            for property in &object.properties {
                properties.insert(property.name.clone(), value_for(&property.ty, &live));
            }
            let id = server
                .create(TypeKey::new(namespace.as_str(), object.name.clone()), properties)
                .expect("every schema type can be created");
            live.push((namespace.as_str().to_owned(), object.name.clone(), id));
        }
    }
    let client = server.connect(VERSION).expect("the versions match");
    let mut host = Host::new(client, Limits::default()).expect("the host starts");

    let mut script = String::from("--- Generated by the test from the schema.\n");
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            let class = class_path(namespace.as_str(), &object.name);
            let id = live
                .iter()
                .find(|(declared_namespace, declared_object, _)| {
                    declared_namespace == namespace.as_str() && *declared_object == object.name
                })
                .map(|(_, _, id)| id.get())
                .expect("the object was created");
            let label = lua_string(&format!("{class}.by_id({id})"));
            script.push_str(&format!(
                "local instance = {class}.by_id({id})\nassert(instance ~= nil, {label} .. \" found nothing\")\n"
            ));
            for property in &object.properties {
                script.push_str(&format!("local _ = instance.{property}\n", property = property.name));
            }
            for method in &object.methods {
                let label = lua_string(&format!("{class}.{}", method.name));
                script.push_str(&format!(
                    "assert(type(instance.{method}) == \"function\", {label} .. \" is not callable\")\n",
                    method = method.name
                ));
            }
            assert!(object.signals.iter().all(|signal| !signal.name.is_empty()));
            let label = lua_string(&class);
            let members = lua_string(&expected_members(object).join(", "));
            script.push_str(&format!(
                "assert(type(instance.on) == \"function\", {label} .. \":on is missing\")\nlocal ok, message = pcall(function() return instance.no_such_member end)\nassert(not ok, {label} .. \" accepted a name the schema does not declare\")\nlocal listed = string.match(tostring(message), \"members: ([^\\n]+)\")\nassert(listed == {members}, \"members were \" .. tostring(listed))\n"
            ));
            script.push_str(&format!(
                "local class_ok, class_message = pcall(function() return {class}.no_such_member end)\nassert(not class_ok, {label} .. \" accepted a class member\")\nassert(string.match(tostring(class_message), \"all, by_id\") ~= nil, {label} .. \" class members: \" .. tostring(class_message))\n"
            ));
        }
    }
    script.push_str(
    "local ok, message = pcall(function() return gnoblin.no_such_member end)\nassert(not ok)\nassert(string.match(tostring(message), \"members: capture, configure, effect, eis, idle, load, lock, output, portal_outcome, portal_ui, request, session, settings, store, timers, window, window_rule, workspace\") ~= nil, \"gnoblin members: \" .. tostring(message))\nassert(string.match(tostring(message), \"extensions\") == nil, \"gnoblin still lists extensions\")\n",
    );

    let config = Config::new("installed");
    let paths = config.script(&script);
    let report = host.run_config(&paths).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);

    // One instance per object type is reachable, and reads are stable.
    let pumped = host.pump(Tick::new(1)).expect("a pump");
    assert!(pumped.errors.is_empty(), "{:#?}", pumped.errors);
}

#[test]
fn an_extension_namespace_is_a_required_module() {
    let catalog = catalog();
    let stub = gnoblin_lua::stub();
    for namespace in catalog.namespaces() {
        if namespace.as_str() == "gnoblin" {
            continue;
        }
        assert!(
            stub.contains(&format!(
                "---@overload fun(name: \"{}\"): {}Module",
                namespace.as_str(),
                namespace.as_str()
            )),
            "the stub has no require overload for {}",
            namespace.as_str()
        );
        for object in catalog.objects(&namespace) {
            assert!(
                stub.contains(&format!(
                    "---@field {} {}.{}Collection",
                    object.name,
                    namespace.as_str(),
                    object.name
                )),
                "the module {} does not expose {}",
                namespace.as_str(),
                object.name
            );
        }
    }
}

/// A host connected to a server with one live window, for the read test below.
fn window_pair() -> (Server, Host<Loopback>, ObjectId) {
    let mut server = Server::new(Graph::new(catalog()), VERSION);
    let properties = gnoblin_testkit::window_fields(&[
        ("id", gnoblin_model::Value::Uint(1)),
        ("title", gnoblin_model::Value::Text("a window".to_owned())),
        ("app_id", gnoblin_model::Value::Text("org.example.App".to_owned())),
        ("width", gnoblin_model::Value::Uint(800)),
        ("height", gnoblin_model::Value::Uint(600)),
    ]);
    let window = server
        .create(TypeKey::gnoblin("window"), properties)
        .expect("the window type is in the schema");
    let client = server.connect(VERSION).expect("the versions match");
    let host = Host::new(client, Limits::default()).expect("the host starts");
    (server, host, window)
}

#[test]
fn a_property_read_is_a_read_of_the_mirror() {
    let (mut server, mut host, window) = window_pair();
    // The snapshot has the title; a later change arrives as a diff.
    server
        .set(window, "title", gnoblin_model::Value::Text("changed".to_owned()))
        .expect("title is a declared property");
    let config = Config::new("read");
    let paths = config.script(&format!(
        r#"
local window = gnoblin.window.by_id({id})
assert(window ~= nil, "the window is in the snapshot")
assert(window.title == "changed", "the mirror has the current title, was " .. tostring(window.title))
assert(window.app_id == "org.example.App", "the app id, was " .. tostring(window.app_id))
assert(window.focused == false, "not focused")
local refused = pcall(function() window.title = "written" end)
assert(not refused, "a property cannot be assigned")
"#,
        id = window.get()
    ));
    let report = host.run_config(&paths).expect("the run starts");
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
}

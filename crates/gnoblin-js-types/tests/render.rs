//! The renderer over small catalogs: the rules that the stock schema does not exercise.

use gnoblin_js_types::render_dts;
use gnoblin_schema::{Catalog, parse_schema};

const DEMO: &str = r#"
namespace = "demo"
version = "0.4.0"

[[object]]
name = "base_thing"
since = "0.3.0"

[[object.property]]
name = "item_count"
type = "uint"
since = "0.3.0"
readonly = true

[[object.property]]
name = "label"
type = "string"
since = "0.3.0"
default = "caf\u00e9"

[[object.property]]
name = "points"
type = "list<object:point>"
since = "0.3.0"

[[object.property]]
name = "friends"
type = "list<object:base_thing>"
since = "0.3.0"

[[object.property]]
name = "owner"
type = "object:base_thing"
since = "0.3.0"

[[object.property]]
name = "grid"
type = "list<list<double>>"
since = "0.3.0"
default = [[1.5, 2.0]]

[[object.method]]
name = "do_it"
since = "0.3.0"

[[object.method.param]]
name = "default"
type = "string"
since = "0.3.0"

[[object.method.param]]
name = "payload"
type = "bytes"
since = "0.3.0"

[[object.signal]]
name = "moved"
since = "0.4.0"

[[object.signal.field]]
name = "target"
type = "string"
since = "0.4.0"

[[object.signal.field]]
name = "new_count"
type = "int"
since = "0.4.0"

[[object]]
name = "derived_thing"
since = "0.3.0"
base = "base_thing"

[[object.property]]
name = "label"
type = "string"
since = "0.3.0"
default = "caf\u00e9"
readonly = true

[[object.property]]
name = "extra"
type = "bool"
since = "0.4.0"

[[object]]
name = "point"
since = "0.3.0"
record = true

[[object.property]]
name = "px"
type = "double"
since = "0.3.0"

[[object]]
name = "util"
since = "0.3.0"

[[object.method]]
name = "reset"
since = "0.3.0"
function = true

[[object.method.param]]
name = "hard"
type = "bool"
since = "0.3.0"

[[object.method]]
name = "peek"
since = "0.3.0"

[[error]]
name = "not-ready"
since = "0.3.0"

[[error]]
name = "too-busy"
since = "0.4.0"
"#;

fn demo() -> String {
    let schema = parse_schema("demo.toml", DEMO).expect("the demo schema parses");
    let catalog = Catalog::new(vec![schema]).expect("the demo catalog builds");
    render_dts(&catalog)
}

fn module(text: &str) -> &str {
    let module = text
        .split("declare module \"gnoblin/schema/demo\" {")
        .nth(1)
        .expect("the demo module is declared");
    module.split("\n}\n").next().expect("the module closes")
}

fn has(text: &str, line: &str) {
    assert!(
        text.lines().any(|candidate| candidate == line),
        "missing `{line}` in\n{text}"
    );
}

#[test]
fn a_list_is_an_array_and_a_nested_list_is_a_nested_array() {
    let text = demo();
    let demo = module(&text);
    has(demo, "        points: Point[];");
    has(demo, "        friends: BaseThing[];");
    has(demo, "        grid: number[][];");
    has(demo, "        /** Default `[[1.5, 2]]`. */");
}

#[test]
fn an_identity_may_be_nil_and_a_record_may_not() {
    let text = demo();
    let demo = module(&text);
    has(demo, "        owner: BaseThing | null;");
    assert!(!demo.contains("Point | null"));
}

#[test]
fn readonly_and_the_unsigned_kind_are_declared() {
    let text = demo();
    has(module(&text), "        readonly itemCount: number;");
    has(module(&text), "        /** Schema name `item_count`. */");
}

#[test]
fn a_default_text_is_an_ascii_literal() {
    let text = demo();
    has(module(&text), "        /** Default `\"caf\\u00e9\"`. */");
}

#[test]
fn a_redeclared_property_is_declared_again_and_an_identical_one_is_not() {
    let text = demo();
    let demo = module(&text);
    let derived = demo
        .split("    export interface DerivedThing extends BaseThing {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("the derived interface");
    assert!(derived.contains("readonly label: string;"), "{derived}");
    assert!(derived.contains("extra: boolean;"), "{derived}");
    assert!(derived.contains("/** Since 0.4.0. */"), "{derived}");
    assert!(!derived.contains("itemCount"), "{derived}");
    assert!(!derived.contains("onMoved"), "{derived}");
    assert!(!derived.contains("doIt"), "{derived}");
}

#[test]
fn a_reserved_parameter_name_and_a_field_named_target_are_renamed() {
    let text = demo();
    let demo = module(&text);
    has(demo, "        /** Schema name `do_it`. */");
    has(demo, "        doIt(default_: string, payload: Uint8Array): Reply;");
    has(demo, "        /** Signal `moved`. Since 0.4.0. */");
    has(
        demo,
        "        onMoved(handler: (target_: this, target: string, newCount: number) => void): Unsubscribe;",
    );
}

#[test]
fn a_namespace_function_is_a_member_of_an_exported_constant() {
    let text = demo();
    let demo = module(&text);
    has(demo, "    export const util: {");
    has(demo, "        reset(hard: boolean): Reply;");
    has(demo, "        peek(): Reply;");
    // The function is not an instance method; `peek` is.
    let util = demo
        .split("    export interface Util {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("the util interface");
    assert!(util.contains("peek(): Reply;"), "{util}");
    assert!(!util.contains("reset"), "{util}");
}

#[test]
fn error_codes_are_a_union_of_string_literals() {
    let text = demo();
    let demo = module(&text);
    has(demo, "    export type ErrorCode =");
    has(demo, "        | \"not-ready\"");
    has(demo, "        | \"too-busy\";");
}

#[test]
fn an_object_that_declares_nothing_is_an_empty_interface() {
    let schema = parse_schema(
        "empty.toml",
        "namespace = \"demo\"\nversion = \"0.1.0\"\n[[object]]\nname = \"nothing\"\nsince = \"0.1.0\"\n",
    )
    .expect("parses");
    let text = render_dts(&Catalog::new(vec![schema]).expect("builds"));
    has(module(&text), "    export interface Nothing {}");
}

#[test]
fn the_output_is_ascii_and_deterministic() {
    let text = demo();
    assert!(text.is_ascii());
    assert_eq!(text, demo());
}

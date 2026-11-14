//! The tree classes as declarations: one interface per class, with the schema's value
//! kinds as TypeScript types. Each assertion is one line of `gnoblin.d.ts`.

use gnoblin_js_types::{render_dts, stock_catalog};
use gnoblin_schema::Namespace;

fn rendered() -> String {
    render_dts(&stock_catalog().expect("the embedded schema loads"))
}

/// The text of the tree module, without the modules around it.
fn tree_module(text: &str) -> &str {
    let module = text
        .split("declare module \"gnoblin/schema/tree\" {")
        .nth(1)
        .expect("the tree module is declared");
    module.split("\n}\n").next().expect("the module closes")
}

/// The member lines of the interface `name` in the tree module, trimmed, and its opener.
fn interface(text: &str, name: &str) -> (String, Vec<String>) {
    let opener = format!("    export interface {name} ");
    let mut lines = tree_module(text).lines().skip_while(|line| !line.starts_with(&opener));
    let first = lines.next().expect("the interface is declared").to_owned();
    if first.ends_with("{}") {
        return (first, Vec::new());
    }
    let members = lines
        .take_while(|line| *line != "    }")
        .map(|line| line.trim().to_owned())
        .collect();
    (first, members)
}

fn has(lines: &[String], expected: &str) {
    assert!(
        lines.iter().any(|line| line == expected),
        "missing `{expected}` in {lines:#?}"
    );
}

fn lacks(lines: &[String], unexpected: &str) {
    assert!(
        !lines.iter().any(|line| line == unexpected),
        "unexpected `{unexpected}`"
    );
}

#[test]
fn every_tree_object_has_exactly_one_interface_that_extends_its_base() {
    let catalog = stock_catalog().expect("the embedded schema loads");
    let text = rendered();
    let tree = tree_module(&text);
    let namespace = Namespace::parse("tree").expect("a namespace");
    let mut classes = 0;
    for object in catalog.objects(&namespace) {
        classes += 1;
        let pascal: String = object
            .name
            .split('_')
            .map(|word| word[..1].to_uppercase() + &word[1..])
            .collect();
        let opener = format!("    export interface {pascal} ");
        assert_eq!(
            tree.lines().filter(|line| line.starts_with(&opener)).count(),
            1,
            "{pascal} is declared once"
        );
        let (first, _) = interface(&text, &pascal);
        match &object.base {
            Some(base) => assert!(
                first.contains(&format!("extends {}", base[..1].to_uppercase() + &base[1..])),
                "{first}"
            ),
            None => assert!(!first.contains("extends"), "{first}"),
        }
    }
    assert!(
        classes >= 12,
        "the tree declares its classes and records, got {classes}"
    );
}

#[test]
fn node_declares_the_base_members() {
    let (first, node) = interface(&rendered(), "Node");
    assert_eq!(first, "    export interface Node {");
    has(&node, "readonly id: number;");
    has(&node, "readonly parent: Node | null;");
    has(&node, "name: string;");
    has(&node, "x: number;");
    has(&node, "rotation: number[];");
    has(&node, "visible: boolean;");
    has(&node, "tags: string[];");
    has(&node, "effects: Effect[];");
    has(&node, "readonly skipped: string[];");
    has(&node, "/** Default `[1, 1, 1]`. Animatable. */");
    has(&node, "onDestroyed(handler: (target: this) => void): Unsubscribe;");
}

#[test]
fn window_declares_only_its_own_members() {
    let (first, window) = interface(&rendered(), "Window");
    assert_eq!(first, "    export interface Window extends Node {");
    has(&window, "readonly title: string;");
    has(&window, "readonly appId: string;");
    has(&window, "/** Schema name `app_id`. Default `\"\"`. */");
    has(&window, "readonly focused: boolean;");
    has(&window, "fullscreen: boolean;");
    has(&window, "workspace: Workspace | null;");
    has(&window, "readonly unresponsive: boolean;");
    has(&window, "focus(): Reply;");
    has(&window, "close(): Reply;");
    has(&window, "kill(): Reply;");
    lacks(&window, "readonly id: number;");
    lacks(&window, "x: number;");
    lacks(&window, "onDestroyed(handler: (target: this) => void): Unsubscribe;");
}

#[test]
fn surface_workspace_screen_and_view_extend_their_bases() {
    let text = rendered();
    let (first, surface) = interface(&text, "Surface");
    assert_eq!(first, "    export interface Surface extends Node {");
    has(&surface, "readonly role: string;");
    has(&surface, "readonly mapped: boolean;");

    let (first, workspace) = interface(&text, "Workspace");
    assert_eq!(first, "    export interface Workspace extends Node {");
    has(&workspace, "index: number;");
    has(&workspace, "output: Screen | null;");
    has(&workspace, "activate(): Reply;");

    let (first, view) = interface(&text, "View");
    assert_eq!(first, "    export interface View extends Node {");
    has(&view, "source: Node | null;");
    has(&view, "capture(): Reply;");

    let (first, screen) = interface(&text, "Screen");
    assert_eq!(first, "    export interface Screen extends View {");
    has(&screen, "readonly screenWidth: number;");
    has(&screen, "readonly mode: string;");
    has(&screen, "transform: string;");
    has(&screen, "apply(): Reply;");
    has(&screen, "onModeChanged(handler: (target: this) => void): Unsubscribe;");
}

#[test]
fn seat_methods_take_typed_parameters_and_signals_carry_their_fields() {
    let (first, seat) = interface(&rendered(), "Seat");
    assert_eq!(first, "    export interface Seat extends Node {");
    has(&seat, "focus: Node | null;");
    has(&seat, "readonly device: string[];");
    has(&seat, "readonly autoRaise: boolean;");
    has(&seat, "readonly autoRaiseDelay: number;");
    has(
        &seat,
        "claim(id: number, chord: string, grab: boolean, device: string): Reply;",
    );
    has(
        &seat,
        "claimGesture(id: number, kind: string, fingers: number, device: string): Reply;",
    );
    has(
        &seat,
        "onChord(handler: (target: this, id: number, chord: string, state: string) => void): Unsubscribe;",
    );
    has(
        &seat,
        "onGesture(handler: (target: this, id: number, kind: string, fingers: number, phase: string, dx: number, dy: number, scale: number, rotation: number) => void): Unsubscribe;",
    );
}

#[test]
fn bytes_and_a_secret_parameter_are_declared() {
    let text = rendered();
    let (_, canvas) = interface(&text, "Canvas");
    has(&canvas, "content: Uint8Array;");
    let (_, lock) = interface(&text, "Lock");
    has(&lock, "readonly state: string;");
    has(&lock, "/** Secret parameters: `text`. */");
    has(&lock, "authenticate(text: string): Reply;");
    has(&lock, "onRequested(handler: (target: this) => void): Unsubscribe;");
}

#[test]
fn a_record_is_a_value_and_never_null() {
    let text = rendered();
    let (first, effect) = interface(&text, "Effect");
    assert_eq!(first, "    export interface Effect {");
    has(&effect, "kind: string;");
    has(&effect, "offsetX: number;");
    let (_, op) = interface(&text, "Op");
    has(&op, "valueEffects: Effect[];");
    let (_, tree) = interface(&text, "Tree");
    has(&tree, "transact(baseSerial: number, serial: number, ops: Op[]): Reply;");
    assert!(!text.contains("Effect | null"), "a record is never nil");
    assert!(!text.contains("Op | null"), "a record is never nil");
}

#[test]
fn the_header_documents_the_one_signal_shape() {
    let text = rendered();
    assert!(text.starts_with("// gnoblin.d.ts"));
    assert!(text.contains("//   - SIGNALS: one shape everywhere. A signal `mode_changed` is the method"));
    assert!(text.contains("// Schema: gnoblin 0.2.0, tree 0.2.0."));
}

#[test]
fn a_namespace_declares_its_error_codes_as_a_string_literal_union() {
    let text = rendered();
    assert!(text.contains("    export type ErrorCode =\n        | \"too-many-inhibitors\"\n"));
    assert!(text.contains("        | \"refused\";\n"));
    // The tree declares no error codes, so it has no union.
    assert!(!tree_module(&text).contains("ErrorCode"));
}

#[test]
fn the_stock_library_follows_the_generated_modules_and_is_marked_proposed() {
    let text = rendered();
    let generated_end = text
        .find("declare module \"gnoblin\" {")
        .expect("the stock module is declared");
    assert!(text.find("declare module \"gnoblin/schema/tree\"").expect("tree") < generated_end);
    assert!(text.contains("PROPOSED API"));
    for line in [
        "    export function defineConfig(config: Config): Config;",
        "    export function spawn(command: string, ...args: string[]): SpawnAction;",
        "    export function show(workspace: Workspace | string): ShowAction;",
        "    export interface Collection<T> extends Iterable<T> {",
        "    export class Client extends Node {",
        "    export class Workspace extends Node {",
        "    export class Layout extends NativeObject {",
        "    export class Binding {",
        "    export interface InputContext {",
        "    export class ClientListener extends MemberListener<Client> {",
        "        requestTransition(staging: () => void, options?: TransitionOptions): Transition;",
    ] {
        assert!(text.lines().any(|candidate| candidate == line), "missing `{line}`");
    }
}

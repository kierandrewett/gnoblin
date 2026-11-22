//! The compositor's `gnoblin` namespace as Python and TypeScript: the typed verbs and signals.
//!
//! The tree's nodes have typed classes of their own (`python.rs`, `typescript.rs`). What is not a node is the `gnoblin` namespace
//! the owner socket serves: the session (`logout`, `suspend`, the bell), the idle watch (`inhibit`), the outputs. The core's table
//! reaches it through two generic functions, `call` and `subscribe`, and each runtime
//! hands a script the same two as one native function each (`_gnoblin_core.verb_call` and `.verb_subscribe`; `__gnoblinCore.call`
//! and `.subscribe`). Everything above them is generated here from the catalog, so a method or a signal added to the schema is in
//! both languages the next time `cargo xtask gen-sdks` runs, with its parameters named and typed and the payload of each signal
//! typed by its fields. Nothing about a verb is written by hand.
//!
//! - `gnoblin/_generated/verbs.py`: one class per object of the namespace, each method and signal a method of it, and
//!   one instance of each class (`verbs.session.logout()`, `verbs.session.on_bell(fn)`).
//! - `data/js/gnoblin-core/generated/verbs.ts`: the same, camel-cased (`verbs.session.logout()`, `verbs.session.onBell(fn)`).

use std::collections::BTreeSet;

use gnoblin_schema::{Field, Method, ObjectType, Signal, Type};

use super::model::SdkSpec;
use super::text::{CodeWriter, camel, pascal, python_ident, ts_ident};

/// What of an object the namespace layer carries: the methods and signals a script may use (nothing secret).
struct Verbs {
    object: ObjectType,
    methods: Vec<Method>,
    signals: Vec<Signal>,
}

/// The objects of the `gnoblin` namespace that have a verb or a signal.
fn objects(spec: &SdkSpec) -> Vec<Verbs> {
    let namespace = spec.namespace("gnoblin");
    spec.catalog
        .objects(&namespace)
        .map(|object| Verbs {
            object: object.clone(),
            methods: object
                .methods
                .iter()
                .filter(|method| !method.is_sensitive())
                .cloned()
                .collect(),
            signals: object
                .signals
                .iter()
                .filter(|signal| !signal.is_sensitive())
                .cloned()
                .collect(),
        })
        .filter(|verbs| !verbs.methods.is_empty() || !verbs.signals.is_empty())
        .collect()
}

fn class_name(object: &ObjectType) -> String {
    format!("gnoblin.{}", object.name)
}

// ---------------------------------------------------------------------------------------------------------------------
// Python

/// A Python annotation for a verb's parameter or a signal's field: the namespace has no node classes, so a reference is a record.
fn py_type(ty: &Type) -> String {
    super::python::annotation(ty, &BTreeSet::new())
}

fn py_parameters(params: &[Field]) -> String {
    let mut text = String::new();
    for param in params {
        let ty = py_type(&param.ty);
        if param.optional {
            text.push_str(&format!(", {}: {ty} | None = None", python_ident(&param.name)));
        } else {
            text.push_str(&format!(", {}: {ty}", python_ident(&param.name)));
        }
    }
    text
}

/// `verbs.py`.
pub fn python(spec: &SdkSpec) -> String {
    let objects = objects(spec);
    let mut code = CodeWriter::new("    ");
    code.line("\"\"\"The compositor's `gnoblin` namespace: its verbs and its signals, as the schema names them.");
    code.blank();
    code.line("Each object of the namespace is an instance here (`verbs.session`, `verbs.idle`). A method calls the verb and, given");
    code.line("`on_answer`, hears the compositor's answer once, on the shell thread, as `(result, error)`: the operation's value as JSON");
    code.line("bytes (empty for a bare done) and the compositor's error code, or `None`. A signal method (`on_bell`) calls its function");
    code.line("with the payload in the schema's field order and returns the `Subscription` that removes it. A reload removes what a");
    code.line("generation registered.");
    code.line("\"\"\"");
    code.blank();
    code.line("from __future__ import annotations");
    code.blank();
    code.line("from collections.abc import Callable");
    code.line("from typing import Any");
    code.blank();
    code.line("import _gnoblin_core as core");
    code.line("from ..events import Subscription");
    code.blank();
    let names: Vec<String> = objects
        .iter()
        .map(|verbs| format!("{:?}", python_ident(&verbs.object.name)))
        .collect();
    code.line(format!("__all__ = [\"Answer\", {}]", names.join(", ")));
    code.blank();
    code.line("Answer = Callable[[bytes, \"str | None\"], None]");
    code.blank();
    code.blank();
    code.line("def _positional(*values: Any) -> tuple[Any, ...]:");
    code.indent();
    code.line("\"\"\"The arguments of a call: the optional ones a caller left out are not sent.\"\"\"");
    code.line("end = len(values)");
    code.line("while end and values[end - 1] is None:");
    code.indent();
    code.line("end -= 1");
    code.dedent();
    code.line("return tuple(values[:end])");
    code.dedent();
    for verbs in &objects {
        let object = &verbs.object;
        let class = class_name(object);
        let name = format!("{}Verbs", pascal(&object.name));
        code.blank();
        code.blank();
        code.line(format!("class {name}:"));
        code.indent();
        code.line(format!(
            "\"\"\"The `{}` object of the `gnoblin` namespace.\"\"\"",
            object.name
        ));
        code.blank();
        code.line(format!("CLASS = {class:?}"));
        for method in &verbs.methods {
            let parameters = py_parameters(&method.params);
            let arguments: Vec<String> = method.params.iter().map(|param| python_ident(&param.name)).collect();
            code.blank();
            code.line(format!(
                "def {}(self{parameters}, *, on_answer: Answer | None = None, object_id: int = 0) -> None:",
                python_ident(&method.name)
            ));
            code.indent();
            code.line(format!(
                "\"\"\"Calls `{}` on the compositor's `{}` object (`object_id` names one of several, `0` the only one).\"\"\"",
                method.name, object.name
            ));
            code.line(format!(
                "core.verb_call(self.CLASS, {:?}, _positional({}), on_answer, object_id)",
                method.name,
                arguments.join(", ")
            ));
            code.dedent();
        }
        for signal in &verbs.signals {
            let fields: Vec<String> = signal.fields.iter().map(|field| py_type(&field.ty)).collect();
            let callable = format!("Callable[[{}], None]", fields.join(", "));
            let payload: Vec<String> = signal.fields.iter().map(|field| format!("`{}`", field.name)).collect();
            code.blank();
            code.line(format!(
                "def on_{}(self, callback: {callable}) -> Subscription:",
                python_ident(&signal.name)
            ));
            code.indent();
            if payload.is_empty() {
                code.line(format!(
                    "\"\"\"Calls `callback()` each time the `{}` signal is raised.\"\"\"",
                    signal.name
                ));
            } else {
                code.line(format!(
                    "\"\"\"Calls `callback` with {} each time the `{}` signal is raised.\"\"\"",
                    payload.join(", "),
                    signal.name
                ));
            }
            code.line(format!(
                "return Subscription(core.verb_subscribe(self.CLASS, {:?}, callback))",
                signal.name
            ));
            code.dedent();
        }
        code.dedent();
    }
    code.blank();
    for verbs in &objects {
        code.line(format!(
            "{} = {}Verbs()",
            python_ident(&verbs.object.name),
            pascal(&verbs.object.name)
        ));
    }
    code.finish()
}

// ---------------------------------------------------------------------------------------------------------------------
// TypeScript

fn ts_type(ty: &Type) -> String {
    super::typescript::ts_type(ty, &BTreeSet::new())
}

fn ts_parameters(params: &[Field]) -> String {
    params
        .iter()
        .map(|param| {
            let optional = if param.optional { "?" } else { "" };
            format!("{}{optional}: {}", ts_ident(&camel(&param.name)), ts_type(&param.ty))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `verbs.ts`.
pub fn typescript(spec: &SdkSpec) -> String {
    let objects = objects(spec);
    let mut code = CodeWriter::new("  ");
    code.line("// The compositor's `gnoblin` namespace: its verbs and its signals, as the schema names them, over the two native functions");
    code.line("// `gnoblin-runtime-js` installs as `__gnoblinCore` (the core table's `call` and `subscribe`). A shell host with no runtime");
    code.line("// library has none, and the first verb says so.");
    code.blank();
    code.line("interface NativeCore {");
    code.line("  call(className: string, method: string, args: unknown[], onAnswer: ((result: string, error: string | null) => void) | null, object: number): void;");
    code.line("  subscribe(className: string, signal: string, callback: (...values: unknown[]) => void): number;");
    code.line("  unsubscribe(registration: number): void;");
    code.line("}");
    code.blank();
    code.line("declare const __gnoblinCore: NativeCore | undefined;");
    code.blank();
    code.line("function native(): NativeCore {");
    code.line("  if (typeof __gnoblinCore === \"undefined\") {");
    code.line("    throw new Error(\"verbs need the JavaScript runtime library (libgnoblin-runtime-js): this shell host has no `__gnoblinCore`\");");
    code.line("  }");
    code.line("  return __gnoblinCore;");
    code.line("}");
    code.blank();
    code.line("/** What the compositor answered to a verb: the operation's value (parsed from its JSON, `undefined` for a bare done) or its error code. */");
    code.line("export interface VerbAnswer {");
    code.line("  readonly value: unknown;");
    code.line("  readonly error: string | null;");
    code.line("}");
    code.blank();
    code.line("/** A signal registration. A reload removes what a generation registered; `unsubscribe` removes one sooner. */");
    code.line("export class VerbSubscription {");
    code.line("  #registration: number;");
    code.line("  constructor(registration: number) {");
    code.line("    this.#registration = registration;");
    code.line("  }");
    code.line("  unsubscribe(): void {");
    code.line("    native().unsubscribe(this.#registration);");
    code.line("  }");
    code.line("}");
    code.blank();
    code.line("function answer(handler: ((answer: VerbAnswer) => void) | undefined): ((result: string, error: string | null) => void) | null {");
    code.line("  if (handler === undefined) {");
    code.line("    return null;");
    code.line("  }");
    code.line(
        "  return (result, error) => handler({ value: result === \"\" ? undefined : JSON.parse(result), error });",
    );
    code.line("}");
    code.blank();
    code.line("/** The arguments of a call: the optional ones a caller left out are not sent. */");
    code.line("function positional(values: unknown[]): unknown[] {");
    code.line("  let end = values.length;");
    code.line("  while (end > 0 && values[end - 1] === undefined) {");
    code.line("    end -= 1;");
    code.line("  }");
    code.line("  return values.slice(0, end);");
    code.line("}");
    for verbs in &objects {
        let object = &verbs.object;
        let class = class_name(object);
        let class_type = format!("{}Verbs", pascal(&object.name));
        code.blank();
        code.line(format!(
            "/** The `{}` object of the `gnoblin` namespace. */",
            object.name
        ));
        code.line(format!("export class {class_type} {{"));
        code.indent();
        code.line(format!("static readonly CLASS = {class:?};"));
        for method in &verbs.methods {
            let parameters = ts_parameters(&method.params);
            let separator = if parameters.is_empty() { "" } else { ", " };
            let arguments: Vec<String> = method
                .params
                .iter()
                .map(|param| ts_ident(&camel(&param.name)))
                .collect();
            code.blank();
            code.line(format!(
                "/** Calls `{}` on the compositor's `{}` object; `onAnswer` hears the answer once, on the shell thread. `object` names one of several (`0`, the only one). */",
                method.name, object.name
            ));
            code.line(format!(
                "{}({parameters}{separator}onAnswer?: (answer: VerbAnswer) => void, object = 0): void {{",
                ts_ident(&camel(&method.name))
            ));
            code.line(format!(
                "  native().call({class_type}.CLASS, {:?}, positional([{}]), answer(onAnswer), object);",
                method.name,
                arguments.join(", ")
            ));
            code.line("}");
        }
        for signal in &verbs.signals {
            let fields: Vec<String> = signal
                .fields
                .iter()
                .map(|field| format!("{}: {}", ts_ident(&camel(&field.name)), ts_type(&field.ty)))
                .collect();
            code.blank();
            code.line(format!(
                "/** Calls `callback` each time the `{}` signal is raised, with its payload in the schema's field order. */",
                signal.name
            ));
            code.line(format!(
                "on{}(callback: ({}) => void): VerbSubscription {{",
                pascal(&signal.name),
                fields.join(", ")
            ));
            code.line(format!(
                "  return new VerbSubscription(native().subscribe({class_type}.CLASS, {:?}, callback as (...values: unknown[]) => void));",
                signal.name
            ));
            code.line("}");
        }
        code.dedent();
        code.line("}");
    }
    code.blank();
    code.line("/** The compositor's `gnoblin` namespace. */");
    code.line("export const verbs = {");
    for verbs in &objects {
        code.line(format!(
            "  {}: new {}Verbs(),",
            ts_ident(&camel(&verbs.object.name)),
            pascal(&verbs.object.name)
        ));
    }
    code.line("};");
    code.finish()
}

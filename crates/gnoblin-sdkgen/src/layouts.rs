//! The built-in layouts and pointer bindings of `libgnoblin-std`, as typed classes: one per kind, with its options as properties and
//! its actions as methods.
//!
//! The table is `gnoblin_std_abi::layouts` (`LAYOUT_KINDS`, `BINDING_KINDS`): the library validates every option against it and takes
//! its defaults from it, so the class of a kind says what the library accepts and nothing else. A kind added there has a class in
//! every language the day `cargo xtask gen-sdks` runs.

use gnoblin_std_abi::layouts::{
    ActionDesc, BINDING_KINDS, EnumMember, Initial, LAYOUT_KINDS, LayoutKindDesc, OptionDesc, ValueDesc, ValueShape,
};
use gnoblin_std_abi::{STD_VALUE_BOOL, STD_VALUE_ENUM, STD_VALUE_NODE, STD_VALUE_NUMBER, STD_VALUE_TEXT};

use super::text::{CodeWriter, camel, pascal, ts_ident};

/// The `STD_VALUE_*` number of a shape, and the field of `GnoblinStdOption` that holds it.
fn wire(shape: &ValueShape) -> (u32, &'static str) {
    match shape {
        ValueShape::Bool => (STD_VALUE_BOOL, "flag"),
        ValueShape::Number | ValueShape::Int => (STD_VALUE_NUMBER, "number"),
        ValueShape::Enum(_) => (STD_VALUE_ENUM, "number"),
        ValueShape::Node => (STD_VALUE_NODE, "node"),
        ValueShape::Text => (STD_VALUE_TEXT, "text"),
    }
}

fn enum_class(kind: &str, option: &str) -> String {
    format!("{}{}", pascal(kind), pascal(option))
}

/// The enumerations of a kind's options and actions: `(class name, members)`.
fn enums_of(kind: &str, options: &[OptionDesc], actions: &[ActionDesc]) -> Vec<(String, &'static [EnumMember])> {
    let mut found: Vec<(String, &'static [EnumMember])> = Vec::new();
    let mut add = |name: &str, desc: &ValueDesc| {
        if let ValueShape::Enum(members) = desc.ty {
            let class = enum_class(kind, name);
            if !found.iter().any(|(seen, _)| *seen == class) {
                found.push((class, members));
            }
        }
    };
    for option in options {
        add(option.name, &option.value);
    }
    for action in actions {
        for arg in action.args {
            add(arg.name, &arg.value);
        }
        if let Some(returns) = &action.returns {
            add(action.name, returns);
        }
    }
    found
}

fn doc_comment(code: &mut CodeWriter, prefix: &str, text: &str) {
    let lines: Vec<String> = text.split_whitespace().fold(vec![String::new()], |mut lines, word| {
        let last = lines.last_mut().expect("one line at least");
        if last.len() + word.len() + 1 > 110 && !last.is_empty() {
            lines.push(word.to_owned());
        } else {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(word);
        }
        lines
    });
    code.doc(prefix, &lines);
}

// ---------------------------------------------------------------------------------------------------------------------
// TypeScript

fn ts_type(kind: &str, name: &str, desc: &ValueDesc) -> String {
    let base = match desc.ty {
        ValueShape::Bool => "boolean".to_owned(),
        ValueShape::Number | ValueShape::Int | ValueShape::Node => "number".to_owned(),
        ValueShape::Enum(_) => enum_class(kind, name),
        ValueShape::Text => "string".to_owned(),
    };
    if desc.optional { format!("{base} | null") } else { base }
}

fn ts_shapes(code: &mut CodeWriter, name: &str, descs: &[(&str, &ValueDesc)]) {
    code.line(format!(
        "static readonly {name}: Record<string, [number, string, boolean]> = {{"
    ));
    code.indent();
    for (key, desc) in descs {
        let (kind, field) = wire(&desc.ty);
        code.line(format!(
            "{key:?}: [{kind}, {field:?}, {}],",
            matches!(desc.ty, ValueShape::Int | ValueShape::Enum(_) | ValueShape::Node)
        ));
    }
    code.dedent();
    code.line("};");
}

fn ts_default(kind: &str, option: &OptionDesc) -> String {
    match (&option.default, &option.value.ty) {
        (Initial::None, _) => "null".to_owned(),
        (Initial::Bool(flag), _) => flag.to_string(),
        (Initial::Text(text), _) => format!("{text:?}"),
        (Initial::Number(value), ValueShape::Enum(members)) => members
            .iter()
            .find(|member| f64::from(member.value) == *value)
            .map_or_else(
                || format!("{value}"),
                |member| format!("{}.{}", enum_class(kind, option.name), member.name.to_ascii_uppercase()),
            ),
        (Initial::Number(value), _) => format!("{value}"),
    }
}

fn ts_enums(code: &mut CodeWriter, kind: &str, options: &[OptionDesc], actions: &[ActionDesc]) {
    for (class, members) in enums_of(kind, options, actions) {
        code.line(format!("export const {class} = {{"));
        code.indent();
        for member in members {
            doc_comment(code, "//", member.doc);
            code.line(format!("{}: {},", member.name.to_ascii_uppercase(), member.value));
        }
        code.dedent();
        code.line("} as const;");
        code.line(format!("export type {class} = (typeof {class})[keyof typeof {class}];"));
        code.blank();
    }
}

/// The TypeScript module of the built-in layouts and bindings (`layouts.ts`).
pub fn typescript() -> String {
    let mut code = CodeWriter::new("  ");
    code.line("import { call } from \"gnoblin-std/generated/native\";");
    code.line("import { Registration, Workspace } from \"gnoblin-std/generated/classes\";");
    code.line("import { dragOutcomeFromNative, dragStepToNative } from \"gnoblin-std/generated/records\";");
    code.line("import type { DragOutcome, DragStep } from \"gnoblin-std/generated/records\";");
    code.line("import type { Init } from \"gnoblin-std/generated/native\";");
    code.blank();
    code.line(BASE_TS.trim_matches('\n'));
    code.blank();
    for kind in LAYOUT_KINDS {
        ts_enums(&mut code, kind.name, kind.options, kind.actions);
        ts_layout(&mut code, kind);
    }
    for kind in BINDING_KINDS {
        ts_enums(&mut code, kind.name, kind.options, &[]);
        let class = format!("{}Binding", pascal(kind.name));
        let options: Vec<String> = kind
            .options
            .iter()
            .map(|option| {
                let ty = ts_type(kind.name, option.name, &option.value);
                format!("{}?: {ty};", ts_ident(&camel(option.name)))
            })
            .collect();
        code.line(format!("export interface {class}Options {{ {} }}", options.join(" ")));
        code.blank();
        doc_comment(&mut code, "//", kind.doc);
        code.line(format!("export class {class} extends BuiltinBinding {{"));
        code.indent();
        code.line(format!("static readonly KIND = {};", kind.id));
        let descs: Vec<(&str, &ValueDesc)> = kind.options.iter().map(|option| (option.name, &option.value)).collect();
        ts_shapes(&mut code, "SHAPES", &descs);
        code.blank();
        code.line(format!("constructor(options: {class}Options = {{}}) {{"));
        code.indent();
        code.line(format!(
            "super({}, {class}.SHAPES, {{ {} }});",
            kind.id,
            kind.options
                .iter()
                .map(|option| format!(
                    "{:?}: options.{} ?? {}",
                    option.name,
                    ts_ident(&camel(option.name)),
                    ts_default(kind.name, option)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        code.dedent();
        code.line("}");
        code.dedent();
        code.line("}");
        code.blank();
    }
    code.finish()
}

fn ts_layout(code: &mut CodeWriter, kind: &LayoutKindDesc) {
    let class = format!("{}Layout", pascal(kind.name));
    let options: Vec<String> = kind
        .options
        .iter()
        .map(|option| {
            let ty = ts_type(kind.name, option.name, &option.value);
            let ty = if matches!(option.default, Initial::None) && !option.value.optional {
                format!("{ty} | null")
            } else {
                ty
            };
            format!("{}?: {ty};", ts_ident(&camel(option.name)))
        })
        .collect();
    code.line(format!("export interface {class}Options {{ {} }}", options.join(" ")));
    code.blank();
    doc_comment(code, "//", kind.doc);
    code.line(format!("export class {class} extends BuiltinLayout {{"));
    code.indent();
    code.line(format!("static readonly KIND = {};", kind.id));
    let option_descs: Vec<(&str, &ValueDesc)> =
        kind.options.iter().map(|option| (option.name, &option.value)).collect();
    ts_shapes(code, "SHAPES", &option_descs);
    let action_args: Vec<(String, ValueDesc)> = kind
        .actions
        .iter()
        .flat_map(|action| {
            action
                .args
                .iter()
                .map(move |arg| (format!("{}.{}", action.name, arg.name), arg.value))
                .chain(
                    action
                        .returns
                        .iter()
                        .map(move |returns| (format!("{}->", action.name), *returns)),
                )
        })
        .collect();
    let refs: Vec<(&str, &ValueDesc)> = action_args.iter().map(|(key, desc)| (key.as_str(), desc)).collect();
    ts_shapes(code, "ACTION_SHAPES", &refs);
    code.blank();
    code.line(format!("constructor(options: {class}Options = {{}}) {{"));
    code.indent();
    let pairs: Vec<String> = kind
        .options
        .iter()
        .map(|option| {
            format!(
                "{:?}: options.{} ?? {}",
                option.name,
                ts_ident(&camel(option.name)),
                ts_default(kind.name, option)
            )
        })
        .collect();
    code.line(format!(
        "super({}, {class}.SHAPES, {class}.ACTION_SHAPES, {{ {} }});",
        kind.id,
        pairs.join(", ")
    ));
    code.dedent();
    code.line("}");
    code.blank();
    for option in kind.options {
        let ident = ts_ident(&camel(option.name));
        let ty = ts_type(kind.name, option.name, &option.value);
        let ty = if matches!(option.default, Initial::None) && !option.value.optional {
            format!("{ty} | null")
        } else {
            ty
        };
        doc_comment(code, "//", option.doc);
        code.line(format!("get {ident}(): {ty} {{"));
        code.line(format!("  return this.getOption({:?}) as {ty};", option.name));
        code.line("}");
        code.line(format!("set {ident}(value: {ty}) {{"));
        code.line(format!("  this.setOption({:?}, value);", option.name));
        code.line("}");
        code.blank();
    }
    for action in kind.actions {
        let params: Vec<String> = action
            .args
            .iter()
            .map(|arg| {
                let ty = ts_type(kind.name, arg.name, &arg.value);
                format!(
                    "{}{}: {ty}",
                    ts_ident(&camel(arg.name)),
                    if arg.value.optional { "?" } else { "" }
                )
            })
            .collect();
        let returns = match &action.returns {
            None => "void".to_owned(),
            Some(desc) => ts_type(kind.name, action.name, desc),
        };
        doc_comment(code, "//", action.doc);
        code.line(format!(
            "{}({}): {returns} {{",
            ts_ident(&camel(action.name)),
            params.join(", ")
        ));
        let args: Vec<String> = action
            .args
            .iter()
            .map(|arg| format!("[{:?}, {}]", arg.name, ts_ident(&camel(arg.name))))
            .collect();
        code.line(format!(
            "  {}this.act({:?}, [{}]){};",
            if action.returns.is_some() { "return " } else { "" },
            action.name,
            args.join(", "),
            if action.returns.is_some() {
                format!(" as {returns}")
            } else {
                String::new()
            }
        ));
        code.line("}");
        code.blank();
    }
    code.dedent();
    code.line("}");
    code.blank();
}

/// The base classes the generated layouts and bindings extend. They are the part that does not depend on a kind, so they are written
/// once here and ship with the module that needs them.
const BASE_TS: &str = r#"
const STD_VALUE_BOOL = 1;

type Shapes = Record<string, [number, string, boolean]>;

function optionToNative(name: string, value: unknown, shape: [number, string, boolean]): Record<string, unknown> {
  const [kind, field] = shape;
  const native: Record<string, unknown> = { kind, name };
  native[field] = kind === STD_VALUE_BOOL ? (value ? 1 : 0) : value;
  return native;
}

function optionFromNative(native: any, whole: boolean): unknown {
  switch (native.kind) {
    case 0:
      return null;
    case 1:
      return native.flag !== 0;
    case 2:
    case 3:
      return whole ? Math.trunc(native.number) : native.number;
    case 4:
      return native.node;
    default:
      return native.text;
  }
}

/** A built-in layout: options before it is attached are kept here, after it they are the workspace's. */
export class BuiltinLayout {
  workspace: Workspace | null = null;

  protected constructor(
    readonly kind: number,
    private readonly shapes: Shapes,
    private readonly actionShapes: Shapes,
    private options: Record<string, unknown>,
  ) {}

  /** Gives the workspace this layout; its options and actions act on that workspace from then on. */
  attach(workspace: Workspace): this {
    const list = Object.entries(this.options)
      .filter(([, value]) => value !== null && value !== undefined)
      .map(([name, value]) => optionToNative(name, value, this.shapes[name]));
    call("workspace_set_layout_kind", workspace.handle, this.kind, list);
    this.workspace = workspace;
    return this;
  }

  protected getOption(name: string): unknown {
    if (this.workspace === null) {
      return this.options[name] ?? null;
    }
    const native = call("workspace_layout_get", this.workspace.handle, name);
    return optionFromNative(native, this.shapes[name][2]);
  }

  protected setOption(name: string, value: unknown): void {
    this.options[name] = value;
    if (this.workspace !== null) {
      call("workspace_layout_set", this.workspace.handle, optionToNative(name, value, this.shapes[name]));
    }
  }

  protected act(name: string, args: [string, unknown][]): unknown {
    if (this.workspace === null) {
      throw new Error(`${name}: the layout is not attached to a workspace`);
    }
    const list = args
      .filter(([, value]) => value !== undefined && value !== null)
      .map(([arg, value]) => optionToNative(arg, value, this.actionShapes[`${name}.${arg}`]));
    const native = call("workspace_layout_act", this.workspace.handle, name, list);
    return optionFromNative(native, this.actionShapes[`${name}->`]?.[2] ?? false);
  }
}

/** A pointer binding the library runs itself. */
export class BuiltinBinding {
  protected constructor(
    readonly kind: number,
    private readonly shapes: Shapes,
    private readonly options: Record<string, unknown>,
  ) {}

  /** Claims the binding's trigger; `remove()` on the result ends it. */
  start(): Registration {
    return new Registration(call("binding_builtin", this.kind, this.list()));
  }

  /**
   * Gives the binding's rules one step of a drag the shell matched itself (`drag_step`): nothing is claimed, the chord, the press
   * and the threshold are the shell's, and so is what a drag keeps between its steps. The answer says what to write to the shell's
   * own window, tile and divider objects.
   */
  step(step: Init<DragStep>): DragOutcome {
    return dragOutcomeFromNative(call("drag_step", this.kind, this.list(), dragStepToNative(step)));
  }

  private list(): Record<string, unknown>[] {
    return Object.entries(this.options)
      .filter(([, value]) => value !== null && value !== undefined)
      .map(([name, value]) => optionToNative(name, value, this.shapes[name]));
  }
}
"#;

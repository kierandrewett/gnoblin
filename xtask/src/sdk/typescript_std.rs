//! The TypeScript std bindings: `data/js/gnoblin-std/generated/`.
//!
//! The same table as `python_std`, in TypeScript, over the native `__gnoblinStd` object that `gnoblin-runtime-js` installs
//! (generic by table entry, over `gnoblin-std-dyn`):
//!
//! - `native.ts`: the one entry, `call(name, ...args)`, and `Init<T>`, the type of a record a caller gives in part;
//! - `constants.ts`: every constant group as an `as const` object and its value type;
//! - `records.ts`: every record as an interface with `fooToNative` and `fooFromNative`, which carry the camelCase of
//!   TypeScript to the snake_case of the table and back, and the constructors of a tagged record (`Prop.number(3)`);
//! - `classes.ts`: a class per handle type and per object of the table (`Collection`, `Workspace`, `Client`, `Transition`...), the
//!   `Layout` interface and its adapter, the module-level functions;
//! - `index.ts`: all of them.
//!
//! These are the binding a TypeScript shell uses as `gnoblin-std/native`. Nothing here knows what a layout does.

use super::model::{
    Arg, Callback, CallbackRet, Elem, FieldDef, Function, Library, Ret, Scalar, Shape, StdObject, StructDef, Tagged,
    Variant,
};
use super::text::{CodeWriter, camel, pascal, ts_ident};
use super::{Comment, File, SdkSpec};

const DIR: &str = "data/js/gnoblin-std/generated";

/// The files of the std TypeScript bindings.
pub fn files(spec: &SdkSpec) -> Result<Vec<File>, String> {
    let library = spec.library("std").ok_or("the std ABI is not reflected")?;
    let renderer = Renderer { lib: library };
    let file = |name: &str, text: String| File {
        path: format!("{DIR}/{name}"),
        comment: Some(Comment::Slashes),
        text,
    };
    Ok(vec![
        file("native.ts", renderer.native()),
        file("constants.ts", renderer.constants()),
        file("records.ts", renderer.records()?),
        file("classes.ts", renderer.classes()?),
        file("layouts.ts", super::layouts::typescript()),
        file("index.ts", index()),
    ])
}

fn index() -> String {
    let mut code = CodeWriter::new("  ");
    for module in ["constants", "records", "classes", "layouts"] {
        code.line(format!("export * from \"gnoblin-std/generated/{module}\";"));
    }
    code.line("export { call, functions, type Init } from \"gnoblin-std/generated/native\";");
    code.finish()
}

struct Renderer<'a> {
    lib: &'a Library,
}

fn short(name: &str) -> String {
    name.strip_prefix("GnoblinStd").unwrap_or(name).to_owned()
}

fn record_name(name: &str) -> String {
    short(name)
}

fn handle_class(name: &str) -> String {
    match short(name).as_str() {
        "Reg" => "Registration".to_owned(),
        other => other.to_owned(),
    }
}

fn vtable_name(record: &str) -> String {
    short(record).trim_end_matches("Vtable").to_owned()
}

fn field_name(name: &str) -> String {
    ts_ident(&camel(name))
}

fn to_native_fn(record: &str) -> String {
    format!("{}ToNative", camel(&short(record)))
}

fn from_native_fn(record: &str) -> String {
    format!("{}FromNative", camel(&short(record)))
}

fn vtable_adapter(record: &str) -> String {
    format!("{}ToNative", camel(&vtable_name(record)))
}

impl Renderer<'_> {
    fn scalar_type(&self, _scalar: &Scalar) -> &'static str {
        "number"
    }

    fn native(&self) -> String {
        let mut c = CodeWriter::new("  ");
        c.line("// The native layer: `gnoblin-runtime-js` installs `__gnoblinStd`, one function that calls any entry of `libgnoblin-std`'s table by");
        c.line("// name. The shell's own TypeScript host (a test harness with no runtime library) has none; the first call then says so.");
        c.blank();
        c.line("interface NativeStd {");
        c.line("  load(): void;");
        c.line("  call(name: string, args: unknown[]): any;");
        c.line("  functions(): string[];");
        c.line("}");
        c.blank();
        c.line("declare const __gnoblinStd: NativeStd | undefined;");
        c.blank();
        c.line("function native(): NativeStd {");
        c.line("  if (typeof __gnoblinStd === \"undefined\") {");
        c.line("    throw new Error(\"gnoblin-std/native needs the JavaScript runtime library (libgnoblin-runtime-js): this shell host has no `__gnoblinStd`\");");
        c.line("  }");
        c.line("  return __gnoblinStd;");
        c.line("}");
        c.blank();
        c.line("// Opening the library at import means a shell that imports it without `libgnoblin-std.so.1` installed fails at the import, naming the file.");
        c.line("if (typeof __gnoblinStd !== \"undefined\") {");
        c.line("  __gnoblinStd.load();");
        c.line("}");
        c.blank();
        c.line("/** A record a caller gives in part: the fields it leaves out are zero. */");
        c.line("export type Init<T> = {");
        c.line("  [K in keyof T]?: T[K] extends (infer E)[] ? E[] : T[K] extends object ? Init<T[K]> : T[K];");
        c.line("};");
        c.blank();
        c.line("/** Calls the table function `name` with `args`, for the generation that is running. */");
        c.line("export function call(name: string, ...args: unknown[]): any {");
        c.line("  return native().call(name, args);");
        c.line("}");
        c.blank();
        c.line("/** The names of the table functions this build of the runtime knows, in table order. */");
        c.line("export function functions(): string[] {");
        c.line("  return native().functions();");
        c.line("}");
        c.finish()
    }

    fn constants(&self) -> String {
        let mut c = CodeWriter::new("  ");
        for group in &self.lib.groups {
            if group.members.iter().any(|member| member.value < 0) {
                continue;
            }
            let name = group.type_name();
            c.line(format!("/** The `{}*` constants. */", group.prefix));
            c.line(format!("export const {name} = {{"));
            c.indent();
            for member in &group.members {
                let key = if member.short.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                    format!("N{}", member.short)
                } else {
                    member.short.clone()
                };
                c.line(format!("{key}: {},", member.value));
            }
            c.dedent();
            c.line("} as const;");
            c.line(format!("export type {name} = (typeof {name})[keyof typeof {name}];"));
            c.blank();
        }
        for constant in &self.lib.loose {
            c.line(format!("export const {} = {};", constant.name, constant.value));
        }
        c.finish()
    }

    // -----------------------------------------------------------------------------------------------------------------
    // records

    fn field_type(&self, shape: &Shape, tagged: Option<&Tagged>, name: &str) -> String {
        match shape {
            Shape::Scalar(_) if name == "kind" && tagged.is_some() => {
                tagged.map(|tagged| tagged.type_name.clone()).unwrap_or_default()
            }
            Shape::Scalar(scalar) => self.scalar_type(scalar).to_owned(),
            Shape::Text => "string".to_owned(),
            Shape::Record(inner) => record_name(inner),
            Shape::RecordPtr(inner) => format!("{} | null", record_name(inner)),
            Shape::Array(Elem::Scalar(_)) => "number[]".to_owned(),
            Shape::Array(Elem::Record(inner)) => format!("{}[]", record_name(inner)),
            Shape::Vtable(inner) => format!("{} | null", vtable_name(inner)),
            Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => "unknown".to_owned(),
        }
    }

    fn visible<'r>(&self, record: &'r StructDef) -> Vec<&'r FieldDef> {
        record
            .visible_fields()
            .filter(|field| !matches!(field.shape, Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_)))
            .collect()
    }

    fn records(&self) -> Result<String, String> {
        let mut c = CodeWriter::new("  ");
        c.line("import type { Init } from \"gnoblin-std/generated/native\";");
        let kinds: Vec<String> = self
            .lib
            .groups
            .iter()
            .filter(|group| group.members.iter().all(|m| m.value >= 0))
            .map(|group| group.type_name())
            .collect();
        c.line(format!(
            "import {{ {} }} from \"gnoblin-std/generated/constants\";",
            kinds.join(", ")
        ));
        c.blank();
        for record in &self.lib.records {
            if record.list_of.is_some() || record.vtable || !record.plain {
                continue;
            }
            self.record(&mut c, record)?;
        }
        for record in self.lib.records.iter().filter(|record| record.vtable) {
            self.vtable(&mut c, record)?;
        }
        Ok(c.finish())
    }

    fn record(&self, c: &mut CodeWriter, record: &StructDef) -> Result<(), String> {
        let name = record_name(&record.name);
        let tagged = self.lib.tagged_record(&record.name);
        let fields = self.visible(record);
        c.doc("//", &record.doc);
        c.line(format!("export interface {name} {{"));
        c.indent();
        for field in &fields {
            c.line(format!(
                "{}: {};",
                field_name(&field.name),
                self.field_type(&field.shape, tagged, &field.name)
            ));
        }
        c.dedent();
        c.line("}");
        c.blank();
        // To the native layer: the snake_case keys of the table.
        c.line(format!(
            "export function {}(value: Init<{name}>): Record<string, unknown> {{",
            to_native_fn(&record.name)
        ));
        c.indent();
        c.line("return {");
        c.indent();
        for field in &fields {
            let ident = field_name(&field.name);
            let expr = match &field.shape {
                Shape::Scalar(_) | Shape::Text => format!("value.{ident}"),
                Shape::Record(inner) => format!(
                    "value.{ident} === undefined ? undefined : {}(value.{ident})",
                    to_native_fn(inner)
                ),
                Shape::RecordPtr(inner) => format!(
                    "value.{ident} == null ? undefined : {}(value.{ident})",
                    to_native_fn(inner)
                ),
                Shape::Vtable(inner) => format!(
                    "value.{ident} == null ? undefined : {}(value.{ident} as {})",
                    vtable_adapter(inner),
                    vtable_name(inner)
                ),
                Shape::Array(_) => continue,
                Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => continue,
            };
            c.line(format!("{}: {expr},", field.name));
        }
        c.dedent();
        c.line("};");
        c.dedent();
        c.line("}");
        c.blank();
        // From the native layer.
        c.line(format!(
            "export function {}(data: any): {name} {{",
            from_native_fn(&record.name)
        ));
        c.indent();
        c.line("return {");
        c.indent();
        for field in &fields {
            let ident = field_name(&field.name);
            let key = format!("data.{}", field.name);
            let expr = match &field.shape {
                Shape::Scalar(_) if field.name == "kind" && tagged.is_some() => {
                    format!(
                        "{key} as {}",
                        tagged.map(|tagged| tagged.type_name.clone()).unwrap_or_default()
                    )
                }
                Shape::Scalar(_) | Shape::Text => key,
                Shape::Record(inner) => format!("{}({key})", from_native_fn(inner)),
                Shape::RecordPtr(inner) => format!("{key} == null ? null : {}({key})", from_native_fn(inner)),
                Shape::Array(Elem::Scalar(_)) => format!("Array.from({key} as number[])"),
                Shape::Array(Elem::Record(inner)) => {
                    format!("(Array.from({key}) as any[]).map({})", from_native_fn(inner))
                }
                Shape::Vtable(_) => "null".to_owned(),
                Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => continue,
            };
            c.line(format!("{ident}: {expr},"));
        }
        c.dedent();
        c.line("};");
        c.dedent();
        c.line("}");
        c.blank();
        // The constructors of a tagged record.
        if let Some(tagged) = tagged {
            self.variants(c, &name, record, tagged)?;
        }
        Ok(())
    }

    fn variants(&self, c: &mut CodeWriter, name: &str, record: &StructDef, tagged: &Tagged) -> Result<(), String> {
        c.line(format!("/** The kinds of `{name}`, each with the fields it reads. */"));
        c.line(format!("export const {name} = {{"));
        c.indent();
        for variant in &tagged.variants {
            self.variant(c, name, record, tagged, variant)?;
        }
        c.dedent();
        c.line("};");
        c.blank();
        Ok(())
    }

    fn variant(
        &self,
        c: &mut CodeWriter,
        name: &str,
        record: &StructDef,
        tagged: &Tagged,
        variant: &Variant,
    ) -> Result<(), String> {
        let method = ts_ident(&camel(&variant.short.to_ascii_lowercase()));
        let mut params = Vec::new();
        let mut sets = Vec::new();
        for field_name_ in &variant.fields {
            let field = record
                .fields
                .iter()
                .find(|field| &field.name == field_name_)
                .ok_or_else(|| format!("{} has no field {field_name_}", record.name))?;
            let ident = field_name(&field.name);
            params.push(format!(
                "{ident}: {} = {}",
                self.field_type(&field.shape, None, &field.name),
                self.zero(&field.shape)
            ));
            sets.push(ident);
        }
        // Every field of the record is present; those the kind does not read are zero.
        let mut fields = Vec::new();
        for field in self.visible(record) {
            let ident = field_name(&field.name);
            if field.name == "kind" {
                fields.push(format!("kind: {}.{}", tagged.type_name, variant.short));
            } else if sets.contains(&ident) {
                fields.push(ident);
            } else {
                fields.push(format!("{ident}: {}", self.zero(&field.shape)));
            }
        }
        c.line(format!("{method}({}): {name} {{", params.join(", ")));
        c.indent();
        c.line(format!("return {{ {} }};", fields.join(", ")));
        c.dedent();
        c.line("},");
        Ok(())
    }

    fn zero(&self, shape: &Shape) -> String {
        match shape {
            Shape::Scalar(_) => "0".to_owned(),
            Shape::Text => "\"\"".to_owned(),
            Shape::RecordPtr(_) | Shape::Vtable(_) => "null".to_owned(),
            Shape::Array(_) => "[]".to_owned(),
            Shape::Record(inner) => {
                let fields: Vec<String> = self
                    .lib
                    .record(inner)
                    .map(|record| {
                        self.visible(record)
                            .iter()
                            .map(|field| format!("{}: {}", field_name(&field.name), self.zero(&field.shape)))
                            .collect()
                    })
                    .unwrap_or_default();
                format!("{{ {} }}", fields.join(", "))
            }
            Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => "undefined".to_owned(),
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // classes

    fn class_infos(&self) -> Vec<ClassInfo> {
        let std_handle = |name: &str| self.lib.handle(name).is_some();
        let mut list: Vec<ClassInfo> = Vec::new();
        for object in self.lib.objects() {
            let (handle, attribute) = match &object.receiver {
                Some(Scalar::Handle(handle)) if std_handle(handle) => (Some(handle.clone()), Some("handle")),
                Some(Scalar::Handle(_)) => (None, Some("node")),
                _ => (None, None),
            };
            list.push(ClassInfo {
                name: handle.as_deref().map_or_else(|| pascal(&object.name), handle_class),
                object: Some(object.name.clone()),
                handle,
                attribute,
            });
        }
        for handle in &self.lib.handles {
            if !list
                .iter()
                .any(|class| class.handle.as_deref() == Some(handle.name.as_str()))
            {
                list.push(ClassInfo {
                    name: handle_class(&handle.name),
                    object: None,
                    handle: Some(handle.name.clone()),
                    attribute: Some("handle"),
                });
            }
        }
        list
    }

    fn class_of_handle(&self, classes: &[ClassInfo], handle: &str) -> Option<String> {
        classes
            .iter()
            .find(|class| class.handle.as_deref() == Some(handle))
            .map(|class| class.name.clone())
    }

    fn arg_type(&self, classes: &[ClassInfo], arg: &Arg) -> String {
        match arg {
            Arg::Value {
                ty: Scalar::Handle(handle),
                ..
            } => match self.class_of_handle(classes, handle) {
                Some(class) => format!("{class} | number"),
                None => "number".to_owned(),
            },
            Arg::Value { .. } => "number".to_owned(),
            Arg::Text { .. } => "string".to_owned(),
            Arg::Record { record, .. } => format!("Init<{}>", record_name(record)),
            Arg::Records { record, .. } => format!("Init<{}>[]", record_name(record)),
            Arg::Callback { callback, .. } => {
                let params: Vec<String> = self
                    .lib
                    .callback(callback)
                    .map(|callback| {
                        callback
                            .params
                            .iter()
                            .filter(|param| !param.out)
                            .map(|param| match &param.shape {
                                Shape::Record(record) => {
                                    format!("{}: {}", ts_ident(&camel(&param.name)), record_name(record))
                                }
                                _ => format!("{}: number", ts_ident(&camel(&param.name))),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                format!("({}) => unknown", params.join(", "))
            }
            _ => "unknown".to_owned(),
        }
    }

    fn return_type(&self, classes: &[ClassInfo], function: &Function) -> String {
        let outs: Vec<&Arg> = function.outs().collect();
        let one = |arg: &Arg| -> String {
            match arg {
                Arg::Out {
                    ty: Scalar::Handle(handle),
                    ..
                } => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("{class} | null"),
                    None => "number | null".to_owned(),
                },
                Arg::Out { .. } => "number".to_owned(),
                Arg::OutRecord { record, .. } => match self.lib.record(record).and_then(|def| def.list_of.as_ref()) {
                    Some(Elem::Scalar(_)) => "number[]".to_owned(),
                    Some(Elem::Record(inner)) => format!("{}[]", record_name(inner)),
                    None => record_name(record),
                },
                Arg::OutRecords { record, .. } => format!("{}[]", record_name(record)),
                Arg::OutText { .. } => "string".to_owned(),
                _ => "unknown".to_owned(),
            }
        };
        let base = match outs.len() {
            0 => match &function.ret {
                Ret::Status if function.flag => "boolean".to_owned(),
                Ret::Status | Ret::Value(_) => "number".to_owned(),
                Ret::Handle(handle) => self
                    .class_of_handle(classes, handle)
                    .unwrap_or_else(|| "number".to_owned()),
                Ret::Void => "void".to_owned(),
            },
            1 => one(outs[0]),
            _ => format!("[{}]", outs.iter().map(|arg| one(arg)).collect::<Vec<_>>().join(", ")),
        };
        if function.flag && !outs.is_empty() {
            format!("{base} | null")
        } else {
            base
        }
    }

    fn wrap(&self, classes: &[ClassInfo], function: &Function) -> String {
        let outs: Vec<&Arg> = function.outs().collect();
        let wrap_one = |arg: &Arg, value: &str| -> String {
            match arg {
                Arg::Out {
                    ty: Scalar::Handle(handle),
                    ..
                } => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("{value} == null ? null : new {class}({value})"),
                    None => value.to_owned(),
                },
                Arg::OutRecord { record, .. } => match self.lib.record(record).and_then(|def| def.list_of.as_ref()) {
                    Some(Elem::Scalar(_)) => format!("Array.from({value} as number[])"),
                    Some(Elem::Record(inner)) => {
                        format!("(Array.from({value}) as any[]).map({})", from_native_fn(inner))
                    }
                    None => format!("{}({value})", from_native_fn(record)),
                },
                Arg::OutRecords { record, .. } => {
                    format!("(Array.from({value}) as any[]).map({})", from_native_fn(record))
                }
                _ => value.to_owned(),
            }
        };
        match outs.len() {
            0 => match &function.ret {
                Ret::Status if function.flag => "raw !== 0".to_owned(),
                Ret::Handle(handle) => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("new {class}(raw)"),
                    None => "raw".to_owned(),
                },
                _ => "raw".to_owned(),
            },
            1 => {
                let wrapped = wrap_one(outs[0], "raw");
                if function.flag {
                    format!("raw == null ? null : ({wrapped})")
                } else {
                    format!("({wrapped})")
                }
            }
            _ => {
                let parts: Vec<String> = outs
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| wrap_one(arg, &format!("raw[{index}]")))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }

    fn convert_callback(&self, callback: &Callback, name: &str) -> String {
        let params: Vec<String> = callback
            .params
            .iter()
            .filter(|param| !param.out)
            .map(|param| ts_ident(&camel(&param.name)))
            .collect();
        let args: Vec<String> = callback
            .params
            .iter()
            .filter(|param| !param.out)
            .map(|param| {
                let ident = ts_ident(&camel(&param.name));
                match &param.shape {
                    Shape::Record(record) => format!("{}({ident})", from_native_fn(record)),
                    _ => ident,
                }
            })
            .collect();
        let typed: Vec<String> = params.iter().map(|param| format!("{param}: any")).collect();
        format!("({}) => {name}({})", typed.join(", "), args.join(", "))
    }

    fn pass(&self, arg: &Arg) -> String {
        let name = ts_ident(&camel(arg.name().unwrap_or("value")));
        match arg {
            Arg::Value {
                ty: Scalar::Handle(_), ..
            } => format!("handleOf({name})"),
            Arg::Record { record, .. } => format!("{}({name})", to_native_fn(record)),
            Arg::Records { record, .. } => format!("{name}.map({})", to_native_fn(record)),
            Arg::Callback { callback, .. } => match self.lib.callback(callback) {
                Some(callback) => self.convert_callback(callback, &name),
                None => name,
            },
            _ => name,
        }
    }

    fn function(
        &self,
        c: &mut CodeWriter,
        classes: &[ClassInfo],
        class: Option<&ClassInfo>,
        function: &Function,
        module_level: bool,
    ) {
        let receiver = function.receiver;
        let instance = !module_level && receiver.is_some() && class.is_some_and(|class| class.attribute.is_some());
        // A member may be called `new` or `delete`; a module-level function may not.
        let method = if module_level {
            ts_ident(&camel(&function.name))
        } else {
            camel(&function.verb)
        };
        let mut params: Vec<String> = Vec::new();
        let mut passes: Vec<String> = Vec::new();
        for (at, arg) in function.args.iter().enumerate() {
            if !arg.is_input() {
                continue;
            }
            if instance && receiver == Some(at) {
                let attribute = class.and_then(|class| class.attribute).unwrap_or("handle");
                passes.push(format!("this.{attribute}"));
            } else {
                let name = ts_ident(&camel(arg.name().unwrap_or("value")));
                params.push(format!("{name}: {}", self.arg_type(classes, arg)));
                passes.push(self.pass(arg));
            }
        }
        c.doc("//", &function.doc);
        let prefix = if module_level {
            "export function "
        } else if instance {
            ""
        } else {
            "static "
        };
        c.line(format!(
            "{prefix}{method}({}): {} {{",
            params.join(", "),
            self.return_type(classes, function)
        ));
        c.indent();
        let mut call = vec![format!("{:?}", function.name)];
        call.extend(passes);
        c.line(format!("const raw = call({});", call.join(", ")));
        c.line(format!("return {};", self.wrap(classes, function)));
        c.dedent();
        c.line("}");
    }

    fn classes(&self) -> Result<String, String> {
        let lib = self.lib;
        let infos = self.class_infos();
        let mut c = CodeWriter::new("  ");
        c.line("import { call } from \"gnoblin-std/generated/native\";");
        c.line("import type { Init } from \"gnoblin-std/generated/native\";");
        let names: Vec<String> = lib
            .records
            .iter()
            .filter(|record| record.list_of.is_none() && !record.vtable && record.plain)
            .flat_map(|record| {
                // The constructors of a tagged record are a value of the record's name: one import covers both.
                [
                    record_name(&record.name),
                    to_native_fn(&record.name),
                    from_native_fn(&record.name),
                ]
            })
            .collect();
        c.line(format!(
            "import {{ {} }} from \"gnoblin-std/generated/records\";",
            names.join(", ")
        ));
        c.blank();
        c.line("function handleOf(value: { handle: number } | { node: number } | number): number {");
        c.line("  if (typeof value === \"number\") {");
        c.line("    return value;");
        c.line("  }");
        c.line("  return \"handle\" in value ? value.handle : value.node;");
        c.line("}");
        c.blank();
        let objects = lib.objects();
        for class in &infos {
            let object: Option<&StdObject> = class
                .object
                .as_ref()
                .and_then(|name| objects.iter().find(|object| &object.name == name));
            c.line(format!("export class {} {{", class.name));
            c.indent();
            if let Some(attribute) = class.attribute {
                c.line(format!("constructor(readonly {attribute}: number) {{}}"));
                c.blank();
            }
            if let Some(object) = object {
                for &index in &object.functions {
                    let function = &lib.functions[index];
                    if function.scriptable {
                        self.function(&mut c, &infos, Some(class), function, false);
                    }
                }
            }
            if let Some(handle) = &class.handle {
                for function in lib
                    .functions
                    .iter()
                    .filter(|function| function.object.is_none() && function.scriptable)
                {
                    let first = function.args.iter().position(Arg::is_input);
                    if let Some(at) = first
                        && matches!(&function.args[at], Arg::Value { ty: Scalar::Handle(found), .. } if found == handle)
                    {
                        let mut method = function.clone();
                        method.receiver = Some(at);
                        method.verb = function.name.clone();
                        self.function(&mut c, &infos, Some(class), &method, false);
                    }
                }
            }
            for bracket in lib.brackets() {
                if class.object.as_deref() == Some(bracket.object.as_str()) {
                    self.bracket(&mut c, &infos, class, &bracket);
                }
            }
            c.dedent();
            c.line("}");
            c.blank();
        }
        for function in lib
            .functions
            .iter()
            .filter(|function| function.object.is_none() && function.scriptable)
        {
            self.function(&mut c, &infos, None, function, true);
            c.blank();
        }
        Ok(c.finish())
    }

    fn bracket(&self, c: &mut CodeWriter, classes: &[ClassInfo], class: &ClassInfo, bracket: &super::model::Bracket) {
        let lib = self.lib;
        let begin = &lib.functions[bracket.begin];
        let commit = &lib.functions[bracket.commit];
        let abort = &lib.functions[bracket.abort];
        let inputs: Vec<&Arg> = begin.inputs().collect();
        let mut params: Vec<String> = inputs
            .iter()
            .map(|arg| {
                format!(
                    "{}: {}",
                    ts_ident(&camel(arg.name().unwrap_or("value"))),
                    self.arg_type(classes, arg)
                )
            })
            .collect();
        params.push("body: () => void".to_owned());
        let names: Vec<String> = inputs
            .iter()
            .map(|arg| ts_ident(&camel(arg.name().unwrap_or("value"))))
            .collect();
        c.line(format!(
            "/** `{}`, then `body()`, then `{}`; `{}` when `body` throws. */",
            begin.name, commit.name, abort.name
        ));
        c.line(format!(
            "static run({}): {} {{",
            params.join(", "),
            self.return_type(classes, commit)
        ));
        c.indent();
        c.line(format!("{}.{}({});", class.name, camel(&begin.verb), names.join(", ")));
        c.line("try {");
        c.line("  body();");
        c.line("} catch (error) {");
        c.line(format!("  {}.{}();", class.name, camel(&abort.verb)));
        c.line("  throw error;");
        c.line("}");
        c.line(format!("return {}.{}();", class.name, camel(&commit.verb)));
        c.dedent();
        c.line("}");
    }

    fn vtable(&self, c: &mut CodeWriter, record: &StructDef) -> Result<(), String> {
        let name = vtable_name(&record.name);
        c.doc("//", &record.doc);
        c.line(format!("export interface {name} {{"));
        c.indent();
        for field in record.visible_fields() {
            let Shape::Callback(callback) = &field.shape else {
                continue;
            };
            let callback = self
                .lib
                .callback(callback)
                .ok_or_else(|| format!("no callback {callback}"))?;
            let params: Vec<String> = callback
                .params
                .iter()
                .filter(|param| !param.out)
                .map(|param| {
                    let ty = match &param.shape {
                        Shape::Record(inner) => record_name(inner),
                        _ => "number".to_owned(),
                    };
                    format!("{}: {ty}", ts_ident(&camel(&param.name)))
                })
                .collect();
            let emits = self.emits(callback);
            let out = callback.params.iter().find(|param| param.out);
            let returns = match (&callback.ret, out) {
                (_, Some(param)) => match &param.shape {
                    Shape::Record(inner) => format!("Init<{}> | null", record_name(inner)),
                    _ => "unknown".to_owned(),
                },
                (CallbackRet::Void, _) => "void".to_owned(),
                (CallbackRet::Status, _) if emits => "Init<Placement>[]".to_owned(),
                (CallbackRet::Status, _) => "boolean | void".to_owned(),
                (CallbackRet::Value(_), _) => "number".to_owned(),
            };
            c.doc("//", &field.doc);
            c.line(format!(
                "{}{}({}): {returns};",
                field_name(&field.name),
                if field.optional { "?" } else { "" },
                params.join(", ")
            ));
        }
        c.dedent();
        c.line("}");
        c.blank();
        // The adapter: the table's snake_case function names, the records as native values.
        c.line(format!(
            "export function {}(target: {name}): Record<string, unknown> {{",
            vtable_adapter(&record.name)
        ));
        c.indent();
        c.line("const table: Record<string, unknown> = {};");
        for field in record.visible_fields() {
            let Shape::Callback(callback) = &field.shape else {
                continue;
            };
            let callback = self
                .lib
                .callback(callback)
                .ok_or_else(|| format!("no callback {callback}"))?;
            let ident = field_name(&field.name);
            let params: Vec<String> = callback
                .params
                .iter()
                .filter(|param| !param.out)
                .map(|param| ts_ident(&camel(&param.name)))
                .collect();
            let args: Vec<String> = callback
                .params
                .iter()
                .filter(|param| !param.out)
                .map(|param| {
                    let ident = ts_ident(&camel(&param.name));
                    match &param.shape {
                        Shape::Record(record) => format!("{}({ident})", from_native_fn(record)),
                        _ => ident,
                    }
                })
                .collect();
            let call = format!("target.{ident}!({})", args.join(", "));
            let emits = self.emits(callback);
            let out = callback.params.iter().find(|param| param.out);
            let body = if emits {
                let emitted = callback
                    .params
                    .iter()
                    .find_map(|param| match &param.shape {
                        Shape::Record(record) => self.sink_record(record),
                        _ => None,
                    })
                    .unwrap_or_default();
                format!("({call} ?? []).map({})", to_native_fn(&emitted))
            } else if let Some(param) = out {
                match &param.shape {
                    Shape::Record(inner) => {
                        format!(
                            "((found) => (found == null ? null : {}(found)))({call})",
                            to_native_fn(inner)
                        )
                    }
                    _ => call,
                }
            } else {
                call
            };
            let typed: Vec<String> = params.iter().map(|param| format!("{param}: any")).collect();
            if field.optional {
                c.line(format!("if (typeof target.{ident} === \"function\") {{"));
            } else {
                c.line("{");
            }
            c.indent();
            c.line(format!("table.{} = ({}) => {body};", field.name, typed.join(", ")));
            c.dedent();
            c.line("}");
        }
        c.line("return table;");
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    /// Whether the callback's answer is the list of records its context's sink emits.
    fn emits(&self, callback: &Callback) -> bool {
        callback.ret == CallbackRet::Status
            && callback.params.iter().any(|param| match &param.shape {
                Shape::Record(record) => self.sink_record(record).is_some(),
                _ => false,
            })
    }

    /// The record a context's `emit` is given, when the context has a sink.
    fn sink_record(&self, record: &str) -> Option<String> {
        let def = self.lib.record(record)?;
        let emit = def.fields.iter().find_map(|field| match &field.shape {
            Shape::Sink(emit) => Some(emit.clone()),
            _ => None,
        })?;
        self.lib
            .callback(&emit)?
            .params
            .iter()
            .find_map(|param| match &param.shape {
                Shape::Record(record) => Some(record.clone()),
                _ => None,
            })
    }
}

struct ClassInfo {
    name: String,
    object: Option<String>,
    handle: Option<String>,
    attribute: Option<&'static str>,
}

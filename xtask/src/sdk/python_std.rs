//! The Python std bindings: `data/python/gnoblin/std.py` and `std.pyi`.
//!
//! The module is the std table as typed Python. Every function of `GnoblinStdApi` is a method of the class of its object, every
//! record a dataclass, every constant group an `IntEnum` or `IntFlag`, the vtable of a layout a base class and an adapter. All of
//! it calls the one native `gnoblin._std.call(name, *args)` (`gnoblin-runtime-python`, over `gnoblin-std-dyn`), which is generic
//! by table entry. The hand-written layer is `_std_base.py` (the native, `Handle`, argument and result conversion) and
//! `_std_sugar.py` (the spellings `len(collection)` and `client.window`).

use super::model::{
    Arg, Callback, CallbackRet, Elem, Function, Library, Prim, Ret, Scalar, SdkSpec, Shape, StdObject, StructDef,
};
use super::text::{CodeWriter, pascal, python_ident};
use super::{Comment, File};

/// Written next to the Python target's own files.
pub fn files(spec: &SdkSpec) -> Result<Vec<File>, String> {
    let library = spec.library("std").ok_or("the std ABI is not reflected")?;
    let file = |path: &str, stub: bool| -> Result<File, String> {
        Ok(File {
            path: path.to_owned(),
            comment: Some(Comment::Hash),
            text: Renderer::new(library, stub).render()?,
        })
    };
    Ok(vec![
        file("data/python/gnoblin/std.py", false)?,
        file("data/python/gnoblin/std.pyi", true)?,
    ])
}

struct Renderer<'a> {
    lib: &'a Library,
    stub: bool,
    code: CodeWriter,
}

/// The classes of the std module, by what they wrap.
struct Classes {
    /// `(class name, receiver)` for every handle typedef and every object with a receiver or none.
    list: Vec<ClassInfo>,
}

struct ClassInfo {
    name: String,
    /// The object whose functions it holds, if there is one.
    object: Option<String>,
    /// The handle typedef it wraps.
    handle: Option<String>,
    /// The attribute that holds the id: `handle` for a std handle, `node` for a node.
    attribute: Option<&'static str>,
}

fn short(name: &str) -> String {
    name.strip_prefix("GnoblinStd").unwrap_or(name).to_owned()
}

/// The Python name of a record: `GnoblinStdRect` is `Rect`.
fn record_class(name: &str) -> String {
    short(name)
}

/// The name of a handle typedef's class.
fn handle_class(name: &str) -> String {
    match short(name).as_str() {
        "Reg" => "Registration".to_owned(),
        other => other.to_owned(),
    }
}

fn prim_annotation(prim: Prim) -> &'static str {
    match prim {
        Prim::F64 => "float",
        _ => "int",
    }
}

impl<'a> Renderer<'a> {
    /// A docstring of `doc`'s lines, unless there are none.
    fn docstring(&mut self, doc: &[String]) {
        let clean = |line: &str| line.replace('\\', "\\\\").replace("\"\"\"", "'''");
        match doc {
            [] => {}
            [one] => self.code.line(format!("\"\"\"{}\"\"\"", clean(one))),
            [first, rest @ ..] => {
                self.code.line(format!("\"\"\"{}", clean(first)));
                for line in rest {
                    self.code.line(clean(line));
                }
                self.code.line("\"\"\"");
            }
        }
    }

    fn new(lib: &'a Library, stub: bool) -> Renderer<'a> {
        Renderer {
            lib,
            stub,
            code: CodeWriter::new("    "),
        }
    }

    fn classes(&self) -> Classes {
        let mut list: Vec<ClassInfo> = Vec::new();
        let std_handle = |name: &str| self.lib.handle(name).is_some();
        for object in self.lib.objects() {
            let (handle, attribute) = match &object.receiver {
                Some(Scalar::Handle(handle)) if std_handle(handle) => (Some(handle.clone()), Some("handle")),
                Some(Scalar::Handle(_)) => (None, Some("node")),
                _ => (None, None),
            };
            list.push(ClassInfo {
                name: pascal(&object.name),
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
        // The class of a handle typedef is named after it, not after its object.
        for class in &mut list {
            if let Some(handle) = &class.handle {
                class.name = handle_class(handle);
            }
        }
        Classes { list }
    }

    /// The class a handle typedef's value is wrapped in.
    fn class_of_handle(&self, classes: &Classes, handle: &str) -> Option<String> {
        classes
            .list
            .iter()
            .find(|class| class.handle.as_deref() == Some(handle))
            .map(|class| class.name.clone())
    }

    /// A Python annotation for a field or an argument that carries `scalar`.
    fn scalar_annotation(&self, scalar: &Scalar) -> String {
        match scalar {
            Scalar::Prim(prim) => prim_annotation(*prim).to_owned(),
            Scalar::Handle(_) => "int".to_owned(),
        }
    }

    fn render(mut self) -> Result<String, String> {
        let classes = self.classes();
        let lib = self.lib;
        self.header();
        self.constants();
        // Records first (a class refers to them), the vtable bases and adapters, then the classes.
        for record in &lib.records {
            if record.list_of.is_some() || record.vtable || !record.plain {
                continue;
            }
            self.record(record)?;
        }
        for record in lib.records.iter().filter(|record| record.vtable) {
            self.vtable(record)?;
        }
        for class in &classes.list {
            self.class(&classes, class)?;
        }
        self.module_functions(&classes)?;
        for line in super::layouts::python(self.stub).lines() {
            self.code.line(line);
        }
        self.footer(&classes);
        Ok(self.code.finish())
    }

    fn header(&mut self) {
        let c = &mut self.code;
        c.line("\"\"\"`gnoblin.std`: the desktop object model above the core, generated from `libgnoblin-std`'s function table.");
        c.blank();
        c.line("Workspaces, collections of clients, layouts, bindings, transitions, styles and scene nodes. Every class is a thin typed face of");
        c.line("one native call, `gnoblin._std.call(name, *args)`; nothing here knows what a layout does. `docs/python-shell.md` has the tour.");
        c.line("\"\"\"");
        c.blank();
        if self.stub {
            c.line("from __future__ import annotations");
            c.blank();
            c.line("from dataclasses import dataclass, field");
            c.line("from enum import IntEnum, IntFlag");
            c.line("from typing import Any, Callable, ClassVar, Iterator, Optional, Sequence");
            c.blank();
            c.line("from ._base import Node");
        } else {
            c.line("from __future__ import annotations");
            c.blank();
            c.line("from dataclasses import dataclass, field");
            c.line("from enum import IntEnum, IntFlag");
            c.line("from typing import Any, Callable, ClassVar, Optional, Sequence");
            c.blank();
            c.line("from . import _std_base as _b");
            c.blank();
            c.line("_call = _b.call");
        }
        c.blank();
    }

    fn constants(&mut self) {
        let lib = self.lib;
        for group in &lib.groups {
            if group.members.iter().any(|member| member.value < 0) {
                continue;
            }
            let base = if group.flags { "IntFlag" } else { "IntEnum" };
            self.code.line(format!("class {}({base}):", group.type_name()));
            self.code.indent();
            self.code
                .line(format!("\"\"\"The `{}*` constants.\"\"\"", group.prefix));
            self.code.blank();
            for member in &group.members {
                let name = python_ident(&member.short);
                let name = if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    format!("N{name}")
                } else {
                    name
                };
                self.code.line(format!("{name} = {}", member.value));
            }
            self.code.dedent();
            self.code.blank();
            self.code.blank();
        }
        for constant in &lib.loose {
            self.code.line(format!("{} = {}", constant.name, constant.value));
        }
        if !lib.loose.is_empty() {
            self.code.blank();
            self.code.blank();
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // records

    fn field_annotation(&self, shape: &Shape, tagged_kind: Option<&str>) -> String {
        match shape {
            Shape::Scalar(scalar) => tagged_kind.map_or_else(|| self.scalar_annotation(scalar), str::to_owned),
            Shape::Text => "str".to_owned(),
            Shape::Record(inner) => record_class(inner),
            Shape::RecordPtr(inner) => format!("Optional[{}]", record_class(inner)),
            Shape::Array(Elem::Scalar(scalar)) => format!("list[{}]", self.scalar_annotation(scalar)),
            Shape::Array(Elem::Record(inner)) => format!("list[{}]", record_class(inner)),
            Shape::Vtable(inner) => format!("Optional[{}]", vtable_class(inner)),
            Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => "Any".to_owned(),
        }
    }

    fn default_of(&self, shape: &Shape) -> String {
        match shape {
            Shape::Scalar(Scalar::Prim(Prim::F64)) => "0.0".to_owned(),
            Shape::Scalar(_) => "0".to_owned(),
            Shape::Text => "\"\"".to_owned(),
            Shape::Record(inner) => format!("field(default_factory={})", record_class(inner)),
            Shape::RecordPtr(_) | Shape::Vtable(_) => "None".to_owned(),
            Shape::Array(_) => "field(default_factory=list)".to_owned(),
            Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => "None".to_owned(),
        }
    }

    /// A field a script sees and writes: not the sink, not a raw pointer.
    fn fields_of<'r>(&self, record: &'r StructDef) -> Vec<&'r super::model::FieldDef> {
        record
            .visible_fields()
            .filter(|field| !matches!(field.shape, Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_)))
            .collect()
    }

    fn record(&mut self, record: &StructDef) -> Result<(), String> {
        let class = record_class(&record.name);
        let tagged = self.lib.tagged_record(&record.name).cloned();
        let fields = self.fields_of(record);
        self.code.line("@dataclass");
        self.code.line(format!("class {class}:"));
        self.code.indent();
        self.docstring(&record.doc);
        self.code.blank();
        for field in &fields {
            let kind_type = (field.name == "kind")
                .then(|| tagged.as_ref().map(|tagged| tagged.type_name.clone()))
                .flatten();
            let annotation = self.field_annotation(&field.shape, kind_type.as_deref());
            self.code.line(format!(
                "{}: {annotation} = {}",
                python_ident(&field.name),
                if self.stub {
                    "...".to_owned()
                } else {
                    self.default_of(&field.shape)
                }
            ));
        }
        if fields.is_empty() {
            self.code.line("pass");
        }
        if let Some(tagged) = &tagged {
            for variant in &tagged.variants {
                self.variant(&class, tagged, variant, record)?;
            }
        }
        self.code.blank();
        if self.stub {
            self.code.line("def _to_dyn(self) -> dict[str, Any]: ...");
            self.code.blank();
            self.code.line("@classmethod");
            self.code
                .line(format!("def _from_dyn(cls, data: dict[str, Any]) -> {class}: ..."));
        } else {
            self.emit_to_dyn(record, &fields);
            self.code.blank();
            self.emit_from_dyn(&class, record, &fields);
        }
        if self.stub {
            for line in sugar_stub(&class) {
                self.code.line(*line);
            }
        }
        self.code.dedent();
        self.code.blank();
        self.code.blank();
        Ok(())
    }

    fn variant(
        &mut self,
        class: &str,
        tagged: &super::model::Tagged,
        variant: &super::model::Variant,
        record: &StructDef,
    ) -> Result<(), String> {
        let method = format!("of_{}", variant.short.to_ascii_lowercase());
        let mut params = Vec::new();
        let mut passes = Vec::new();
        for name in &variant.fields {
            let field = record
                .fields
                .iter()
                .find(|field| &field.name == name)
                .ok_or_else(|| format!("{} has no field {name}", record.name))?;
            let annotation = self.field_annotation(&field.shape, None);
            let default = self.default_of(&field.shape);
            let ident = python_ident(name);
            params.push(format!(
                "{ident}: {annotation} = {}",
                if self.stub { "..." } else { &default }
            ));
            passes.push(format!("{ident}={ident}"));
        }
        let signature = if params.is_empty() {
            "cls".to_owned()
        } else {
            format!("cls, {}", params.join(", "))
        };
        self.code.blank();
        self.code.line("@classmethod");
        if self.stub {
            self.code.line(format!("def {method}({signature}) -> {class}: ..."));
        } else {
            self.code.line(format!("def {method}({signature}) -> {class}:"));
            self.code.indent();
            for line in &variant.doc {
                self.code.line(format!("# {line}"));
            }
            let kind = format!("kind={}.{}", tagged.type_name, python_ident(&variant.short));
            let args = std::iter::once(kind).chain(passes).collect::<Vec<_>>().join(", ");
            self.code.line(format!("return cls({args})"));
            self.code.dedent();
        }
        Ok(())
    }

    fn emit_to_dyn(&mut self, _record: &StructDef, fields: &[&super::model::FieldDef]) {
        self.code.line("def _to_dyn(self) -> dict[str, Any]:");
        self.code.indent();
        self.code.line("return {");
        self.code.indent();
        for field in fields {
            let ident = python_ident(&field.name);
            let expr = match &field.shape {
                Shape::Scalar(_) | Shape::Text => format!("self.{ident}"),
                Shape::Record(_) => format!("self.{ident}._to_dyn()"),
                Shape::RecordPtr(_) => format!("None if self.{ident} is None else self.{ident}._to_dyn()"),
                Shape::Vtable(inner) => format!(
                    "None if self.{ident} is None else {}(self.{ident})",
                    adapter_class(inner)
                ),
                Shape::Array(Elem::Scalar(_)) => format!("list(self.{ident})"),
                Shape::Array(Elem::Record(_)) => format!("[item._to_dyn() for item in self.{ident}]"),
                // Lent by the library: a script does not write them.
                Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => continue,
            };
            self.code.line(format!("{:?}: {expr},", field.name));
        }
        self.code.dedent();
        self.code.line("}");
        self.code.dedent();
    }

    fn emit_from_dyn(&mut self, class: &str, _record: &StructDef, fields: &[&super::model::FieldDef]) {
        self.code.line("@classmethod");
        self.code
            .line(format!("def _from_dyn(cls, data: dict[str, Any]) -> {class}:"));
        self.code.indent();
        self.code.line("return cls(");
        self.code.indent();
        let tagged = self.lib.tagged_record(&_record.name).cloned();
        for field in fields {
            let ident = python_ident(&field.name);
            let key = format!("{:?}", field.name);
            let expr = match &field.shape {
                Shape::Scalar(_) if field.name == "kind" && tagged.is_some() => {
                    let type_name = tagged
                        .as_ref()
                        .map(|tagged| tagged.type_name.clone())
                        .unwrap_or_default();
                    format!("_b.kind({type_name}, data[{key}])")
                }
                Shape::Scalar(_) | Shape::Text => format!("data[{key}]"),
                Shape::Record(inner) => format!("{}._from_dyn(data[{key}])", record_class(inner)),
                Shape::RecordPtr(inner) => format!(
                    "None if data[{key}] is None else {}._from_dyn(data[{key}])",
                    record_class(inner)
                ),
                Shape::Array(Elem::Scalar(_)) => format!("list(data[{key}])"),
                Shape::Array(Elem::Record(inner)) => {
                    format!("[{}._from_dyn(item) for item in data[{key}]]", record_class(inner))
                }
                Shape::Vtable(_) | Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => continue,
            };
            self.code.line(format!("{ident}={expr},"));
        }
        self.code.dedent();
        self.code.line(")");
        self.code.dedent();
    }

    // -----------------------------------------------------------------------------------------------------------------
    // vtables: the base class a script subclasses, and the adapter the native layer calls

    fn callback_conv(&self, callback: &Callback) -> Vec<String> {
        callback
            .params
            .iter()
            .filter(|param| !param.out)
            .map(|param| match &param.shape {
                Shape::Record(record) => format!("{}._from_dyn", record_class(record)),
                _ => String::new(),
            })
            .collect()
    }

    fn vtable(&mut self, record: &StructDef) -> Result<(), String> {
        let base = vtable_class(&record.name);
        let adapter = adapter_class(&record.name);
        // The base class: what a script subclasses. The required functions are defined; an optional one is the script's to add.
        self.code.line(format!("class {base}:"));
        self.code.indent();
        self.docstring(&record.doc);
        self.code.blank();
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
                    let annotation = match &param.shape {
                        Shape::Record(inner) => record_class(inner),
                        Shape::Scalar(scalar) => self.scalar_annotation(scalar),
                        _ => "Any".to_owned(),
                    };
                    format!("{}: {annotation}", python_ident(&param.name))
                })
                .collect();
            let out_record = callback.params.iter().find(|param| param.out);
            let returns = match (&callback.ret, out_record) {
                (_, Some(param)) => match &param.shape {
                    Shape::Record(inner) => format!("Optional[{}]", record_class(inner)),
                    _ => "Any".to_owned(),
                },
                (CallbackRet::Void, _) => "None".to_owned(),
                (CallbackRet::Status, _) => {
                    if callback.params.iter().any(|param| {
                        matches!(&param.shape, Shape::Record(record)
                            if self.lib.record(record).is_some_and(|def| def.fields.iter().any(|f| matches!(f.shape, Shape::Sink(_)))))
                    }) {
                        "Sequence[Placement]".to_owned()
                    } else {
                        "Optional[bool]".to_owned()
                    }
                }
                (CallbackRet::Value(prim), _) => prim_annotation(*prim).to_owned(),
            };
            let signature = std::iter::once("self".to_owned())
                .chain(params)
                .collect::<Vec<_>>()
                .join(", ");
            if field.optional && !self.stub {
                // Optional: documented, not defined, so the library is told the object has no such function.
                self.code
                    .line(format!("# Optional: `def {}({signature}) -> {returns}`", field.name));
                for line in &field.doc {
                    self.code.line(format!("#   {line}"));
                }
                continue;
            }
            self.code
                .line(format!("def {}({signature}) -> {returns}:", python_ident(&field.name)));
            self.code.indent();
            self.docstring(&field.doc);
            if self.stub {
                self.code.line("...");
            } else {
                self.code.line("raise NotImplementedError");
            }
            self.code.dedent();
            self.code.blank();
        }
        self.code.dedent();
        self.code.blank();
        if self.stub {
            self.code.blank();
            return Ok(());
        }
        // The adapter: gives each function of the object the typed records, and the library the records it wants back.
        self.code.line(format!("class {adapter}:"));
        self.code.indent();
        self.code.line(format!(
            "\"\"\"Presents a `{base}` to the native layer: the records of each call as classes.\"\"\""
        ));
        self.code.blank();
        self.code.line("def __init__(self, target: Any) -> None:");
        self.code.indent();
        self.code.line("self._target = target");
        for field in record.visible_fields() {
            let Shape::Callback(callback) = &field.shape else {
                continue;
            };
            let callback = self
                .lib
                .callback(callback)
                .ok_or_else(|| format!("no callback {callback}"))?;
            let name = python_ident(&field.name);
            let convs = self.callback_conv(callback);
            let params: Vec<String> = callback
                .params
                .iter()
                .filter(|param| !param.out)
                .map(|param| python_ident(&param.name))
                .collect();
            let args: Vec<String> = params
                .iter()
                .zip(&convs)
                .map(|(param, conv)| {
                    if conv.is_empty() {
                        param.clone()
                    } else {
                        format!("{conv}({param})")
                    }
                })
                .collect();
            let call = format!("target.{name}({})", args.join(", "));
            let emits = callback.params.iter().any(|param| {
                matches!(&param.shape, Shape::Record(record)
                    if self.lib.record(record).is_some_and(|def| def.fields.iter().any(|f| matches!(f.shape, Shape::Sink(_)))))
            }) && callback.ret == CallbackRet::Status;
            let out_record = callback.params.iter().any(|param| param.out);
            let body = if emits {
                format!("[_b.dyn(item) for item in ({call} or ())]")
            } else if out_record {
                format!("_b.dyn({call})")
            } else {
                call
            };
            self.code.line(format!("if hasattr(target, {:?}):", field.name));
            self.code.indent();
            self.code
                .line(format!("self.{name} = lambda {}: {body}", params.join(", ")));
            self.code.dedent();
        }
        self.code.dedent();
        self.code.dedent();
        self.code.blank();
        self.code.blank();
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------------------------
    // classes

    fn arg_annotation(&self, classes: &Classes, arg: &Arg) -> String {
        match arg {
            Arg::Value { ty, .. } => match ty {
                Scalar::Handle(handle) => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("{class} | int"),
                    None => "int".to_owned(),
                },
                Scalar::Prim(prim) => prim_annotation(*prim).to_owned(),
            },
            Arg::Text { .. } => "str".to_owned(),
            Arg::Record { record, .. } => format!("{} | dict[str, Any]", record_class(record)),
            Arg::Records { record, .. } => format!("Sequence[{} | dict[str, Any]]", record_class(record)),
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
                                Shape::Record(record) => record_class(record),
                                _ => "Any".to_owned(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                format!("Callable[[{}], Any]", params.join(", "))
            }
            _ => "Any".to_owned(),
        }
    }

    /// The annotation of what a call returns.
    fn return_annotation(&self, classes: &Classes, function: &Function) -> String {
        let outs: Vec<&Arg> = function.outs().collect();
        let one = |arg: &Arg| -> String {
            match arg {
                Arg::Out { ty, .. } => match ty {
                    Scalar::Handle(handle) => match self.class_of_handle(classes, handle) {
                        Some(class) => format!("Optional[{class}]"),
                        None => "Optional[int]".to_owned(),
                    },
                    Scalar::Prim(prim) => prim_annotation(*prim).to_owned(),
                },
                Arg::OutRecord { record, .. } => match self.lib.record(record).and_then(|def| def.list_of.as_ref()) {
                    Some(Elem::Scalar(_)) => "list[int]".to_owned(),
                    Some(Elem::Record(inner)) => format!("list[{}]", record_class(inner)),
                    None => record_class(record),
                },
                Arg::OutRecords { record, .. } => format!("list[{}]", record_class(record)),
                Arg::OutText { .. } => "str".to_owned(),
                _ => "Any".to_owned(),
            }
        };
        let base = match outs.len() {
            0 => match &function.ret {
                Ret::Status if function.flag => "bool".to_owned(),
                Ret::Status => "int".to_owned(),
                Ret::Handle(handle) => self
                    .class_of_handle(classes, handle)
                    .unwrap_or_else(|| "int".to_owned()),
                Ret::Value(prim) => prim_annotation(*prim).to_owned(),
                Ret::Void => "None".to_owned(),
            },
            1 => one(outs[0]),
            _ => format!(
                "tuple[{}]",
                outs.iter().map(|arg| one(arg)).collect::<Vec<_>>().join(", ")
            ),
        };
        if function.flag && !outs.is_empty() {
            format!("Optional[{base}]")
        } else {
            base
        }
    }

    /// The Python expression that turns the raw result `raw` of a call into what the signature says.
    fn wrap(&self, classes: &Classes, function: &Function) -> String {
        let outs: Vec<&Arg> = function.outs().collect();
        let wrap_one = |arg: &Arg, value: &str| -> String {
            match arg {
                Arg::Out {
                    ty: Scalar::Handle(handle),
                    ..
                } => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("_b.wrap({class}, {value})"),
                    None => value.to_owned(),
                },
                Arg::OutRecord { record, .. } => match self.lib.record(record).and_then(|def| def.list_of.as_ref()) {
                    Some(Elem::Scalar(_)) => format!("list({value})"),
                    Some(Elem::Record(inner)) => {
                        format!("[{}._from_dyn(item) for item in {value}]", record_class(inner))
                    }
                    None => format!("{}._from_dyn({value})", record_class(record)),
                },
                Arg::OutRecords { record, .. } => {
                    format!("[{}._from_dyn(item) for item in {value}]", record_class(record))
                }
                _ => value.to_owned(),
            }
        };
        match outs.len() {
            0 => match &function.ret {
                Ret::Status if function.flag => "bool(raw)".to_owned(),
                Ret::Handle(handle) => match self.class_of_handle(classes, handle) {
                    Some(class) => format!("{class}(raw)"),
                    None => "raw".to_owned(),
                },
                _ => "raw".to_owned(),
            },
            1 => {
                let wrapped = wrap_one(outs[0], "raw");
                if function.flag {
                    format!("None if raw is None else {wrapped}")
                } else {
                    wrapped
                }
            }
            _ => {
                let parts: Vec<String> = outs
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| wrap_one(arg, &format!("raw[{index}]")))
                    .collect();
                format!("({})", parts.join(", "))
            }
        }
    }

    /// The Python expression for one argument, as the native call takes it.
    fn pass(&self, function: &Function, arg: &Arg) -> String {
        let name = python_ident(arg.name().unwrap_or("value"));
        match arg {
            Arg::Callback { callback, .. } => {
                let convs = self
                    .lib
                    .callback(callback)
                    .map(|callback| self.callback_conv(callback))
                    .unwrap_or_default();
                let list = convs
                    .iter()
                    .map(|conv| {
                        if conv.is_empty() {
                            "None".to_owned()
                        } else {
                            conv.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = function;
                format!("_b.listener({name}, ({list},))")
            }
            _ => format!("_b.dyn({name})"),
        }
    }

    fn function(
        &mut self,
        classes: &Classes,
        class: Option<&ClassInfo>,
        function: &Function,
        module_level: bool,
    ) -> Result<(), String> {
        let receiver = function.receiver;
        let method = if module_level {
            python_ident(&function.name)
        } else {
            python_ident(&function.verb)
        };
        // The inputs, with the receiver as `self`.
        let mut params: Vec<String> = Vec::new();
        let mut passes: Vec<String> = Vec::new();
        let mut index = 0;
        for (at, arg) in function.args.iter().enumerate() {
            if !arg.is_input() {
                continue;
            }
            let is_receiver = receiver == Some(at) && class.is_some_and(|class| class.attribute.is_some());
            if is_receiver {
                let attribute = class.and_then(|class| class.attribute).unwrap_or("handle");
                passes.push(format!("self.{attribute}"));
            } else {
                let name = python_ident(arg.name().unwrap_or("value"));
                params.push(format!("{name}: {}", self.arg_annotation(classes, arg)));
                passes.push(self.pass(function, arg));
            }
            index += 1;
        }
        let _ = index;
        let instance = receiver.is_some() && class.is_some_and(|class| class.attribute.is_some()) && !module_level;
        let returns = self.return_annotation(classes, function);
        let signature = if instance {
            std::iter::once("self".to_owned())
                .chain(params)
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            params.join(", ")
        };
        if !instance && !module_level {
            self.code.line("@staticmethod");
        }
        self.code.line(format!("def {method}({signature}) -> {returns}:"));
        self.code.indent();
        self.docstring(&function.doc);
        if self.stub {
            self.code.line("...");
        } else {
            let mut call = vec![format!("{:?}", function.name)];
            call.extend(passes);
            self.code.line(format!("raw = _call({})", call.join(", ")));
            self.code.line(format!("return {}", self.wrap(classes, function)));
        }
        self.code.dedent();
        self.code.blank();
        Ok(())
    }

    fn class(&mut self, classes: &Classes, class: &ClassInfo) -> Result<(), String> {
        let lib = self.lib;
        let objects = lib.objects();
        let object: Option<&StdObject> = class
            .object
            .as_ref()
            .and_then(|name| objects.iter().find(|object| &object.name == name));
        self.code.line(if self.stub {
            format!("class {}:", class.name)
        } else {
            format!("class {}(_b.Handle):", class.name)
        });
        self.code.indent();
        match (&class.handle, class.attribute) {
            (_, Some("node")) => self.code.line(format!(
                "\"\"\"A node of the tree: the functions of `{}` that take one.\"\"\"",
                class.object.as_deref().unwrap_or("")
            )),
            (Some(handle), _) => {
                let doc = lib.handle(handle).map(|handle| handle.doc.clone()).unwrap_or_default();
                self.docstring(&doc);
            }
            _ => self.code.line(format!(
                "\"\"\"Functions of `{}` that need no handle.\"\"\"",
                class.object.as_deref().unwrap_or("")
            )),
        }
        self.code.blank();
        if let Some(attribute) = class.attribute {
            if self.stub {
                self.code.line(format!("{attribute}: int"));
                self.code.line(format!(
                    "def __init__(self, {attribute}: int | {}) -> None: ...",
                    class.name
                ));
            } else {
                self.code.line(format!("__slots__ = ({attribute:?},)"));
                self.code.blank();
                self.code.line(format!(
                    "def __init__(self, {attribute}: int | {}) -> None:",
                    class.name
                ));
                self.code.indent();
                self.code.line(format!("self.{attribute} = _b.ident({attribute})"));
                self.code.dedent();
            }
            self.code.blank();
        }
        if let Some(object) = object {
            for &index in &object.functions {
                let function = &lib.functions[index];
                if !function.scriptable {
                    continue;
                }
                self.function(classes, Some(class), function, false)?;
            }
        }
        // A function with no object that takes this class's handle first is its method too (`remove` of a registration).
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
                    self.function(classes, Some(class), &method, false)?;
                }
            }
        }
        // A begin / commit / abort triple is one scoped call.
        for bracket in lib.brackets() {
            if class.object.as_deref() != Some(bracket.object.as_str()) {
                continue;
            }
            self.bracket(classes, class, &bracket)?;
        }
        if self.stub {
            for line in sugar_stub(&class.name) {
                self.code.line(*line);
            }
        }
        if self.stub && class.attribute.is_none() && object.is_none() {
            self.code.line("...");
        }
        self.code.dedent();
        self.code.blank();
        self.code.blank();
        Ok(())
    }

    fn bracket(&mut self, classes: &Classes, class: &ClassInfo, bracket: &super::model::Bracket) -> Result<(), String> {
        let lib = self.lib;
        let begin = &lib.functions[bracket.begin];
        let commit = &lib.functions[bracket.commit];
        let begin_inputs: Vec<&Arg> = begin.inputs().collect();
        let params: Vec<String> = begin_inputs
            .iter()
            .map(|arg| {
                format!(
                    "{}: {}",
                    python_ident(arg.name().unwrap_or("value")),
                    self.arg_annotation(classes, arg)
                )
            })
            .collect();
        let names: Vec<String> = begin_inputs
            .iter()
            .map(|arg| python_ident(arg.name().unwrap_or("value")))
            .collect();
        let returns = self.return_annotation(classes, commit);
        let signature = params
            .iter()
            .cloned()
            .chain(std::iter::once("body: Callable[[], Any]".to_owned()))
            .collect::<Vec<_>>()
            .join(", ");
        self.code.line("@staticmethod");
        self.code.line(format!("def run({signature}) -> {returns}:"));
        self.code.indent();
        self.code.line(format!(
            "\"\"\"{}: `{}`, then `body()`, then `{}` (or `{}` when `body` raises).\"\"\"",
            class.name, begin.name, commit.name, lib.functions[bracket.abort].name
        ));
        if self.stub {
            self.code.line("...");
        } else {
            self.code.line(format!(
                "{}.{}({})",
                class.name,
                python_ident(&begin.verb),
                names.join(", ")
            ));
            self.code.line("try:");
            self.code.indent();
            self.code.line("body()");
            self.code.dedent();
            self.code.line("except BaseException:");
            self.code.indent();
            self.code.line(format!(
                "{}.{}()",
                class.name,
                python_ident(&lib.functions[bracket.abort].verb)
            ));
            self.code.line("raise");
            self.code.dedent();
            self.code
                .line(format!("return {}.{}()", class.name, python_ident(&commit.verb)));
        }
        self.code.dedent();
        self.code.blank();
        Ok(())
    }

    fn module_functions(&mut self, classes: &Classes) -> Result<(), String> {
        let lib = self.lib;
        for function in lib
            .functions
            .iter()
            .filter(|function| function.object.is_none() && function.scriptable)
        {
            self.function(classes, None, function, true)?;
        }
        Ok(())
    }

    fn footer(&mut self, classes: &Classes) {
        let _ = classes;
        if self.stub {
            self.code.line("# The functions of `_std_sugar.py`:");
            self.code.line("def clients() -> Collection: ...");
            self.code.line("def workspaces() -> Collection: ...");
            self.code.line("def outputs() -> Collection: ...");
        } else {
            self.code.blank();
            self.code.line(
                "from . import _std_sugar  # noqa: E402  the hand-written spellings: len(collection), client.window",
            );
            self.code.blank();
            self.code.line("_std_sugar.install(globals())");
        }
    }
}

/// The members `_std_sugar.py` adds to a generated class, as a type checker reads them. The module is the one hand-written layer of
/// `gnoblin.std`; what it adds is these spellings and nothing the table could say.
fn sugar_stub(class: &str) -> &'static [&'static str] {
    match class {
        "Collection" => &[
            "def __len__(self) -> int: ...",
            "def __iter__(self) -> Iterator[Any]: ...",
            "def __contains__(self, member: int) -> bool: ...",
            "def items(self) -> Iterator[Any]: ...",
            "def at(self, index: int) -> Optional[int]: ...",
            "def on(self, event: int, callback: Callable[..., Any], replay: bool = ...) -> Registration: ...",
        ],
        "Client" => &["@property", "def window(self) -> Node: ..."],
        "Placement" => &[
            "@staticmethod",
            "def tile(node: int, x: float, y: float, width: float, height: float) -> Placement: ...",
        ],
        "LayoutContext" => &["def inset_area(self) -> Rect: ..."],
        "Workspace" => &[
            "def use(self, layout: Any) -> Any: ...",
            "@classmethod",
            "def create(cls, id: str, *, name: str = ..., layout: Optional[Layout] = ..., gaps: tuple[float, float] = ...) -> Workspace: ...",
        ],
        _ => &[],
    }
}

fn vtable_class(record: &str) -> String {
    short(record).trim_end_matches("Vtable").to_owned()
}

fn adapter_class(record: &str) -> String {
    format!("_{}Adapter", vtable_class(record))
}

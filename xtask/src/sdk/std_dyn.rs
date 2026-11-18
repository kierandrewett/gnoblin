//! The Rust target of the std table's dynamic call layer: `crates/gnoblin-std-dyn/src/generated.rs`.
//!
//! Every language runtime reaches `libgnoblin-std.so.1` through one function, `StdLib::call(ctx, "collection_len", args)`, and
//! this file is the body of it: for each entry of the table a function that converts the `Dyn` arguments to the table's
//! types, calls the entry, converts what it wrote back, and the `repr(C)` record readers and writers and the callback
//! trampolines the entries need. Nothing in it is chosen by hand: the model says how every argument and every field is
//! carried, so a function added to `gnoblin-std-abi` is callable from Python and TypeScript the day this target runs.

use super::model::{Arg, Callback, CallbackRet, Elem, Function, Library, Prim, Ret, Scalar, SdkSpec, Shape, StructDef};
use super::text::{CodeWriter, rust_ident, snake};
use super::{Comment, File, Target};

/// The target.
pub struct StdDynTarget;

const PATH: &str = "crates/gnoblin-std-dyn/src/generated.rs";

impl Target for StdDynTarget {
    fn name(&self) -> &'static str {
        "std-dyn"
    }

    fn owned_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let library = spec.library("std").ok_or("the std ABI is not reflected")?;
        Ok(vec![File {
            path: PATH.to_owned(),
            comment: Some(Comment::Slashes),
            text: render(library)?,
        }])
    }
}

fn rust_prim(prim: Prim) -> &'static str {
    match prim {
        Prim::U8 => "u8",
        Prim::U32 => "u32",
        Prim::I32 => "i32",
        Prim::U64 => "u64",
        Prim::I64 => "i64",
        Prim::Usize => "usize",
        Prim::F64 => "f64",
    }
}

/// The primitive a scalar is carried as: a handle typedef is its integer type, and the core's handles are `u64`.
fn prim_of(library: &Library, scalar: &Scalar) -> Prim {
    match scalar {
        Scalar::Prim(prim) => *prim,
        Scalar::Handle(name) => library.handle(name).map_or(Prim::U64, |handle| handle.repr),
    }
}

fn is_handle(scalar: &Scalar) -> bool {
    matches!(scalar, Scalar::Handle(_))
}

/// `value.to_u32(what)`, a `Result`.
fn try_from_dyn(prim: Prim, value: &str, what: &str) -> String {
    let method = match prim {
        Prim::U8 => "to_u8",
        Prim::U32 => "to_u32",
        Prim::I32 => "to_i32",
        Prim::U64 => "to_u64",
        Prim::I64 => "to_i64",
        Prim::Usize => "to_usize",
        Prim::F64 => "to_f64",
    };
    format!("{value}.{method}({what})")
}

/// `value.to_u32(what)?`.
fn from_dyn(prim: Prim, value: &str, what: &str) -> String {
    format!("{}?", try_from_dyn(prim, value, what))
}

/// The `Dyn` for a number of `prim`.
fn to_dyn(prim: Prim, value: &str) -> String {
    match prim {
        Prim::U8 | Prim::U32 => format!("Dyn::Uint(u64::from({value}))"),
        Prim::U64 => format!("Dyn::Uint({value})"),
        Prim::Usize => format!("Dyn::Uint({value} as u64)"),
        Prim::I32 => format!("Dyn::Int(i64::from({value}))"),
        Prim::I64 => format!("Dyn::Int({value})"),
        Prim::F64 => format!("Dyn::Float({value})"),
    }
}

fn reader(record: &str) -> String {
    format!("read_{}", snake(record))
}

fn writer(record: &str) -> String {
    format!("write_{}", snake(record))
}

fn vtable_builder(record: &str) -> String {
    format!("vtable_{}", snake(record))
}

fn tramp(callback: &str) -> String {
    format!("tramp_{}", snake(callback))
}

fn vt_tramp(record: &str, field: &str) -> String {
    format!("vt_{}_{field}", snake(record))
}

/// What the generator has to emit, found by walking from the functions.
#[derive(Default)]
struct Needs {
    read: Vec<String>,
    write: Vec<String>,
    tramps: Vec<String>,
    vtables: Vec<String>,
}

impl Needs {
    fn add(list: &mut Vec<String>, name: &str) -> bool {
        if list.iter().any(|seen| seen == name) {
            false
        } else {
            list.push(name.to_owned());
            true
        }
    }

    fn need_read(&mut self, library: &Library, record: &str) {
        if !Needs::add(&mut self.read, record) {
            return;
        }
        if let Some(def) = library.record(record) {
            for field in &def.fields {
                match &field.shape {
                    Shape::Record(inner) | Shape::RecordPtr(inner) | Shape::Array(Elem::Record(inner)) => {
                        self.need_read(library, inner);
                    }
                    _ => {}
                }
            }
        }
    }

    fn need_write(&mut self, library: &Library, record: &str) {
        if !Needs::add(&mut self.write, record) {
            return;
        }
        if let Some(def) = library.record(record) {
            for field in &def.fields {
                match &field.shape {
                    Shape::Record(inner) | Shape::RecordPtr(inner) | Shape::Array(Elem::Record(inner)) => {
                        self.need_write(library, inner);
                    }
                    Shape::Vtable(inner) => self.need_vtable(library, inner),
                    _ => {}
                }
            }
        }
    }

    fn need_vtable(&mut self, library: &Library, record: &str) {
        if !Needs::add(&mut self.vtables, record) {
            return;
        }
        if let Some(def) = library.record(record) {
            for field in &def.fields {
                if let Shape::Callback(callback) = &field.shape
                    && let Some(callback) = library.callback(callback)
                {
                    self.need_callback(library, callback);
                }
            }
        }
    }

    /// A callback's parameters are records the library fills for the script (read), records the script's answer fills (write)
    /// and, for a record with a sink, the records the answer emits (write).
    fn need_callback(&mut self, library: &Library, callback: &Callback) {
        for param in &callback.params {
            if let Shape::Record(record) = &param.shape {
                if param.out {
                    self.need_write(library, record);
                } else {
                    self.need_read(library, record);
                    if let Some(def) = library.record(record) {
                        for field in &def.fields {
                            if let Shape::Sink(emit) = &field.shape
                                && let Some(emit) = library.callback(emit)
                            {
                                for emitted in &emit.params {
                                    if let Shape::Record(inner) = &emitted.shape {
                                        self.need_write(library, inner);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn collect(library: &Library) -> Needs {
        let mut needs = Needs::default();
        for function in library.functions.iter().filter(|function| function.scriptable) {
            for arg in &function.args {
                match arg {
                    Arg::Record { record, .. } | Arg::Records { record, .. } => needs.need_write(library, record),
                    Arg::OutRecord { record, .. } | Arg::OutRecords { record, .. } => needs.need_read(library, record),
                    Arg::Callback { callback, .. } => {
                        if let Some(def) = library.callback(callback) {
                            Needs::add(&mut needs.tramps, callback);
                            needs.need_callback(library, def);
                        }
                    }
                    _ => {}
                }
            }
        }
        needs
    }
}

fn render(library: &Library) -> Result<String, String> {
    let needs = Needs::collect(library);
    let mut code = CodeWriter::new("    ");
    code.line("//! Every function of `libgnoblin-std`'s table, callable by name with dynamic values. Written by `cargo xtask gen-sdks`.");
    code.blank();
    code.line("use core::ffi::c_void;");
    code.line("use core::mem::{offset_of, size_of, zeroed};");
    code.line("use std::rc::Rc;");
    code.blank();
    code.line("use gnoblin_core_abi::{E_OK, E_PANIC, E_TYPE, GnoblinStr};");
    code.line("use gnoblin_std_abi::*;");
    code.blank();
    code.line("use crate::{Arena, Callable, Dyn, MapReader, StdError, StdLib, contain, copy_str, drop_user, slice_of, status_of, user_of};");
    code.blank();
    functions_table(&mut code, library);
    dispatch(&mut code, library);
    for function in &library.functions {
        if function.scriptable {
            call_function(&mut code, library, function)?;
        }
    }
    for record in &needs.read {
        let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
        read_record(&mut code, library, def)?;
    }
    for record in &needs.write {
        let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
        write_record(&mut code, library, def)?;
    }
    for record in &needs.vtables {
        vtable(&mut code, library, record)?;
    }
    for name in &needs.tramps {
        let callback = library.callback(name).ok_or_else(|| format!("no callback {name}"))?;
        trampoline(&mut code, library, &tramp(name), callback, None)?;
    }
    Ok(code.finish())
}

/// The objects whose functions work on a generation that has not made the library its desktop: the built-in layouts, the drag machines
/// and the pure geometry of drags and gestures (they read nothing of the library's state), and animation, styles and the scene (the
/// library attaches its model alone for them). A call to one never starts the desktop.
const STANDALONE_OBJECTS: &[&str] = &["builtin", "drag", "gesture", "easing", "transition", "style", "scene"];

fn functions_table(code: &mut CodeWriter, library: &Library) {
    code.line("/// One entry of the table as a script sees it.");
    code.line("pub struct FunctionInfo {");
    code.indent();
    code.line("/// The table field, which is the name a call takes.");
    code.line("pub name: &'static str,");
    code.line("/// The arguments a call passes, after the context, in order.");
    code.line("pub inputs: &'static [&'static str],");
    code.line("/// Whether the call layer can carry it; a function with a raw pointer argument is native only.");
    code.line("pub scriptable: bool,");
    code.line("/// Whether it works on a generation that has not made the library its desktop (the built-in layouts, animation, styles and the scene):");
    code.line("/// a call to it never opens the desktop. The library attaches its model by itself when such a call is the first.");
    code.line("pub standalone: bool,");
    code.dedent();
    code.line("}");
    code.blank();
    code.line("/// The table, in order.");
    code.line("pub const FUNCTIONS: &[FunctionInfo] = &[");
    code.indent();
    for function in &library.functions {
        let inputs: Vec<String> = function
            .inputs()
            .filter_map(Arg::name)
            .map(|name| format!("{name:?}"))
            .collect();
        code.line(format!(
            "FunctionInfo {{ name: {:?}, inputs: &[{}], scriptable: {}, standalone: {} }},",
            function.name,
            inputs.join(", "),
            function.scriptable,
            STANDALONE_OBJECTS.contains(&function.object.as_deref().unwrap_or(""))
        ));
    }
    code.dedent();
    code.line("];");
    code.blank();
}

fn dispatch(code: &mut CodeWriter, library: &Library) {
    code.line(
        "pub(crate) fn dispatch(lib: &StdLib, ctx: u64, function: &str, args: &[Dyn]) -> Result<Dyn, StdError> {",
    );
    code.indent();
    code.line("match function {");
    code.indent();
    for function in library.functions.iter().filter(|function| function.scriptable) {
        code.line(format!(
            "{:?} => {}(lib, ctx, args),",
            function.name,
            rust_ident(&function.name)
        ));
    }
    code.line("other => Err(StdError::unsupported(other)),");
    code.dedent();
    code.line("}");
    code.dedent();
    code.line("}");
    code.blank();
}

fn call_function(code: &mut CodeWriter, library: &Library, function: &Function) -> Result<(), String> {
    let name = &function.name;
    let inputs: Vec<&Arg> = function.inputs().filter(|arg| !matches!(arg, Arg::Ctx)).collect();
    code.line("#[allow(unused_mut, unused_variables)]");
    code.line(format!(
        "fn {}(lib: &StdLib, ctx: u64, args: &[Dyn]) -> Result<Dyn, StdError> {{",
        rust_ident(name)
    ));
    code.indent();
    code.line(format!(
        "if !lib.has(offset_of!(GnoblinStdApi, {name}) + size_of::<usize>()) {{"
    ));
    code.indent();
    code.line(format!("return Err(StdError::unsupported({name:?}));"));
    code.dedent();
    code.line("}");
    code.line(format!("if args.len() != {} {{", inputs.len()));
    code.indent();
    let wanted: Vec<&str> = inputs.iter().filter_map(|arg| arg.name()).collect();
    code.line(format!(
        "return Err(StdError::type_error(format!(\"`{name}` takes {} arguments ({}), not {{}}\", args.len())));",
        inputs.len(),
        wanted.join(", ")
    ));
    code.dedent();
    code.line("}");
    code.line("let api = lib.api();");
    code.line("let mut arena = Arena::new();");

    let mut call_args: Vec<String> = Vec::new();
    let mut index = 0;
    let mut outs: Vec<(String, &Arg)> = Vec::new();
    for arg in &function.args {
        match arg {
            Arg::Ctx => call_args.push("ctx".to_owned()),
            Arg::Value { name: arg_name, ty } => {
                let value = from_dyn(
                    prim_of(library, ty),
                    &format!("args[{index}]"),
                    &format!("\"argument `{arg_name}` of `{name}`\""),
                );
                code.line(format!("let {} = {value};", rust_ident(arg_name)));
                call_args.push(rust_ident(arg_name));
                index += 1;
            }
            Arg::Text { name: arg_name } => {
                code.line(format!(
                    "let {0} = arena.text(args[{index}].to_text(\"argument `{arg_name}` of `{name}`\")?);",
                    rust_ident(arg_name)
                ));
                call_args.push(format!("{0}.ptr, {0}.len", rust_ident(arg_name)));
                index += 1;
            }
            Arg::Record { name: arg_name, record } => {
                code.line(format!(
                    "let {0} = {1}(&mut arena, &args[{index}], \"argument `{arg_name}` of `{name}`\")?;",
                    rust_ident(arg_name),
                    writer(record)
                ));
                call_args.push(format!("&raw const {}", rust_ident(arg_name)));
                index += 1;
            }
            Arg::Records { name: arg_name, record } => {
                let what = format!("\"argument `{arg_name}` of `{name}`\"");
                code.line(format!(
                    "let {0}: Vec<{record}> = args[{index}].to_list({what})?.iter().map(|item| {1}(&mut arena, item, {what})).collect::<Result<_, _>>()?;",
                    rust_ident(arg_name),
                    writer(record)
                ));
                call_args.push(format!("{0}.as_ptr(), {0}.len()", rust_ident(arg_name)));
                index += 1;
            }
            Arg::Callback {
                name: arg_name,
                callback,
            } => {
                let what = format!("\"argument `{arg_name}` of `{name}`\"");
                code.line(format!(
                    "let {0} = arena.user(args[{index}].to_callable({what})?);",
                    rust_ident(arg_name)
                ));
                call_args.push(format!(
                    "{}, {}, Some(drop_user)",
                    tramp(callback),
                    rust_ident(arg_name)
                ));
                index += 1;
            }
            Arg::Out { name: arg_name, ty } => {
                let prim = prim_of(library, ty);
                code.line(format!(
                    "let mut {}: {} = {};",
                    rust_ident(arg_name),
                    rust_prim(prim),
                    if prim == Prim::F64 { "0.0" } else { "0" }
                ));
                call_args.push(format!("&raw mut {}", rust_ident(arg_name)));
                outs.push((rust_ident(arg_name), arg));
            }
            Arg::OutRecord { name: arg_name, record } => {
                let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
                code.line(format!(
                    "// SAFETY: an all-zero `{record}` is valid: numbers, null pointers and absent functions."
                ));
                code.line(format!(
                    "let mut {}: {record} = unsafe {{ zeroed() }};",
                    rust_ident(arg_name)
                ));
                if def.extensible {
                    code.line(format!("{}.size = size_of::<{record}>() as u32;", rust_ident(arg_name)));
                }
                call_args.push(format!("&raw mut {}", rust_ident(arg_name)));
                outs.push((rust_ident(arg_name), arg));
            }
            Arg::OutRecords {
                name: arg_name,
                record,
                count,
            } => {
                let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
                code.line(format!(
                    "// SAFETY: an all-zero `{record}` is valid: numbers, null pointers and absent functions."
                ));
                code.line(format!(
                    "let mut {}: Vec<{record}> = (0..{}).map(|_| unsafe {{ zeroed() }}).collect();",
                    rust_ident(arg_name),
                    rust_ident(count)
                ));
                if def.extensible {
                    code.line(format!(
                        "for slot in &mut {} {{ slot.size = size_of::<{record}>() as u32; }}",
                        rust_ident(arg_name)
                    ));
                }
                call_args.push(format!("{}.as_mut_ptr()", rust_ident(arg_name)));
                outs.push((rust_ident(arg_name), arg));
            }
            Arg::OutText { name: arg_name } => {
                code.line(format!(
                    "let mut {}: GnoblinStr = GnoblinStr {{ ptr: core::ptr::null(), len: 0 }};",
                    rust_ident(arg_name)
                ));
                call_args.push(format!("&raw mut {}", rust_ident(arg_name)));
                outs.push((rust_ident(arg_name), arg));
            }
            Arg::Raw { .. } => return Err(format!("`{name}` has a raw argument and is not scriptable")),
        }
    }
    code.line("// SAFETY: every pointer passed is a live local or lives in the arena for the call, and the table entry is present.");
    code.line(format!(
        "let returned = unsafe {{ (api.{name})({}) }};",
        call_args.join(", ")
    ));
    match &function.ret {
        Ret::Status => code.line(format!("let status = lib.check({name:?}, returned)?;")),
        Ret::Handle(_) => code.line(format!("let handle = lib.check_handle({name:?}, returned)?;")),
        Ret::Value(_) | Ret::Void => {}
    }
    code.line("arena.adopt();");
    if function.flag && !outs.is_empty() {
        code.line("if status == 0 {");
        code.indent();
        code.line("return Ok(Dyn::Nil);");
        code.dedent();
        code.line("}");
    }
    let mut results: Vec<String> = Vec::new();
    for (ident, arg) in &outs {
        results.push(match arg {
            Arg::Out { ty, .. } => {
                let prim = prim_of(library, ty);
                let value = to_dyn(prim, ident);
                if is_handle(ty) {
                    format!("if {ident} == 0 {{ Dyn::Nil }} else {{ {value} }}")
                } else {
                    value
                }
            }
            Arg::OutRecord { record, .. } => format!("{}(&{ident})", reader(record)),
            Arg::OutRecords { record, .. } => {
                format!("Dyn::List({ident}.iter().map({}).collect())", reader(record))
            }
            Arg::OutText { .. } => format!("Dyn::Text(unsafe {{ copy_str({ident}) }})"),
            _ => String::new(),
        });
    }
    let value = match (results.len(), &function.ret) {
        (1, _) => results.remove(0),
        (0, Ret::Status) => "Dyn::Int(i64::from(status))".to_owned(),
        (0, Ret::Handle(_)) => "Dyn::Uint(handle)".to_owned(),
        (0, Ret::Value(prim)) => to_dyn(*prim, "returned"),
        (0, Ret::Void) => "Dyn::Nil".to_owned(),
        _ => format!("Dyn::List(vec![{}])", results.join(", ")),
    };
    if value.contains("unsafe {") || value.contains("copy_str") {
        code.line("// SAFETY: the library lent the text for this call; it is copied here.");
    }
    code.line(format!("Ok({value})"));
    code.dedent();
    code.line("}");
    code.blank();
    Ok(())
}

/// The `Dyn` of one field of a record, read from `record.field`.
fn read_field(library: &Library, record: &StructDef, field: &super::model::FieldDef) -> Option<String> {
    let access = format!("record.{}", rust_ident(&field.name));
    Some(match &field.shape {
        Shape::Scalar(scalar) => to_dyn(prim_of(library, scalar), &access),
        Shape::Text => format!("Dyn::Text(unsafe {{ copy_str({access}) }})"),
        Shape::Record(inner) => format!("{}(&{access})", reader(inner)),
        Shape::RecordPtr(inner) => format!("unsafe {{ {access}.as_ref() }}.map_or(Dyn::Nil, {})", reader(inner)),
        Shape::Array(elem) => {
            let (ptr, count) = array_fields(record, field);
            let list = format!("unsafe {{ slice_of(record.{ptr}, record.{count}) }}");
            match elem {
                Elem::Scalar(scalar) => format!(
                    "Dyn::List({list}.iter().map(|item| {}).collect())",
                    to_dyn(prim_of(library, scalar), "*item")
                ),
                Elem::Record(inner) => format!("Dyn::List({list}.iter().map({}).collect())", reader(inner)),
            }
        }
        Shape::Sink(_) | Shape::Vtable(_) | Shape::Callback(_) | Shape::Pointer(_) => return None,
    })
}

/// The pointer and count field names of an array field: the merged `items` is `ptr` and `len`, any other is `x` and `n_x`.
fn array_fields(_record: &StructDef, field: &super::model::FieldDef) -> (String, String) {
    if field.name == "items" {
        ("ptr".to_owned(), "len".to_owned())
    } else {
        (rust_ident(&field.name), format!("n_{}", field.name))
    }
}

fn read_record(code: &mut CodeWriter, library: &Library, record: &StructDef) -> Result<(), String> {
    code.line(format!(
        "fn {}(record: &{}) -> Dyn {{",
        reader(&record.name),
        record.name
    ));
    code.indent();
    code.line("// SAFETY: the library filled the record; the texts and arrays it lends are readable for this call.");
    if let Some(Elem::Scalar(scalar)) = &record.list_of {
        code.line(format!(
            "Dyn::List(unsafe {{ slice_of(record.ptr, record.len) }}.iter().map(|item| {}).collect())",
            to_dyn(prim_of(library, scalar), "*item")
        ));
    } else if let Some(Elem::Record(inner)) = &record.list_of {
        code.line(format!(
            "Dyn::List(unsafe {{ slice_of(record.ptr, record.len) }}.iter().map({}).collect())",
            reader(inner)
        ));
    } else {
        code.line("Dyn::Map(vec![");
        code.indent();
        for field in record.visible_fields() {
            if let Some(expr) = read_field(library, record, field) {
                code.line(format!("({:?}.to_owned(), {expr}),", field.name));
            }
        }
        code.dedent();
        code.line("])");
    }
    code.dedent();
    code.line("}");
    code.blank();
    Ok(())
}

fn write_record(code: &mut CodeWriter, library: &Library, record: &StructDef) -> Result<(), String> {
    let name = &record.name;
    code.line("#[allow(unused_mut)]");
    code.line(format!(
        "fn {}(arena: &mut Arena, value: &Dyn, what: &str) -> Result<{name}, StdError> {{",
        writer(name)
    ));
    code.indent();
    code.line("let mut reader = MapReader::new(value, what)?;");
    code.line(format!(
        "// SAFETY: an all-zero `{name}` is valid: numbers, null pointers and absent functions."
    ));
    code.line(format!("let mut record: {name} = unsafe {{ zeroed() }};"));
    if record.extensible {
        code.line(format!("record.size = size_of::<{name}>() as u32;"));
    }
    for field in record.visible_fields() {
        let ident = rust_ident(&field.name);
        let key = format!("{:?}", field.name);
        match &field.shape {
            Shape::Scalar(scalar) => {
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!(
                    "record.{ident} = {};",
                    from_dyn(prim_of(library, scalar), "item", &format!("&reader.path({key})"))
                ));
                code.dedent();
                code.line("}");
            }
            Shape::Text => {
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!(
                    "record.{ident} = arena.text(item.to_text(&reader.path({key}))?);"
                ));
                code.dedent();
                code.line("}");
            }
            Shape::Record(inner) => {
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!(
                    "record.{ident} = {}(arena, item, &reader.path({key}))?;",
                    writer(inner)
                ));
                code.dedent();
                code.line("}");
            }
            Shape::RecordPtr(inner) => {
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!(
                    "let nested = {}(arena, item, &reader.path({key}))?;",
                    writer(inner)
                ));
                code.line(format!("record.{ident} = arena.keep(nested);"));
                code.dedent();
                code.line("}");
            }
            Shape::Vtable(inner) => {
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!("let object = item.to_callable(&reader.path({key}))?;"));
                code.line(format!("let table = {}(&object)?;", vtable_builder(inner)));
                code.line(format!("record.{ident} = arena.keep(table);"));
                code.line("record.user = arena.user(object);");
                code.line("record.drop_user = Some(drop_user);");
                code.dedent();
                code.line("}");
            }
            Shape::Array(elem) => {
                let (ptr, count) = array_fields(record, field);
                code.line(format!("if let Some(item) = reader.take({key}) {{"));
                code.indent();
                code.line(format!("let list = item.to_list(&reader.path({key}))?;"));
                match elem {
                    Elem::Scalar(scalar) => {
                        code.line("let mut values = Vec::with_capacity(list.len());");
                        code.line("for entry in list {");
                        code.indent();
                        code.line(format!(
                            "values.push({});",
                            from_dyn(prim_of(library, scalar), "entry", &format!("&reader.path({key})"))
                        ));
                        code.dedent();
                        code.line("}");
                    }
                    Elem::Record(inner) => {
                        code.line(format!(
                            "let values: Vec<{inner}> = list.iter().map(|entry| {}(arena, entry, &reader.path({key}))).collect::<Result<_, _>>()?;",
                            writer(inner)
                        ));
                    }
                }
                // The array lives as long as the arena, which outlives the call.
                code.line("let kept = arena.keep(values);");
                code.line("// SAFETY: `kept` points at the vector the arena owns for the call.");
                code.line(format!("record.{ptr} = unsafe {{ (*kept).as_ptr() }};"));
                code.line(format!("record.{count} = unsafe {{ (*kept).len() }};"));
                code.dedent();
                code.line("}");
            }
            // Lent by the library or built by it: a script does not write them.
            Shape::Sink(_) | Shape::Callback(_) | Shape::Pointer(_) => {}
        }
    }
    code.line("reader.finish()?;");
    code.line("Ok(record)");
    code.dedent();
    code.line("}");
    code.blank();
    Ok(())
}

fn vtable(code: &mut CodeWriter, library: &Library, record: &str) -> Result<(), String> {
    let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
    code.line(format!(
        "fn {}(object: &Rc<dyn Callable>) -> Result<{record}, StdError> {{",
        vtable_builder(record)
    ));
    code.indent();
    // A function a vtable must have is checked first; one it may have is given when the object has it.
    for field in def.visible_fields() {
        if matches!(field.shape, Shape::Callback(_)) && !field.optional {
            code.line(format!("if !object.has_method({:?}) {{", field.name));
            code.indent();
            code.line(format!(
                "return Err(StdError::type_error(\"the object has no method `{}`\"));",
                field.name
            ));
            code.dedent();
            code.line("}");
        }
    }
    code.line(format!("Ok({record} {{"));
    code.indent();
    for field in &def.fields {
        let ident = rust_ident(&field.name);
        match &field.shape {
            Shape::Callback(_) if field.optional => code.line(format!(
                "{ident}: if object.has_method({:?}) {{ Some({}) }} else {{ None }},",
                field.name,
                vt_tramp(record, &field.name)
            )),
            Shape::Callback(_) => code.line(format!("{ident}: {},", vt_tramp(record, &field.name))),
            _ if field.name == "size" => code.line(format!("size: size_of::<{record}>() as u32,")),
            _ => code.line(format!("{ident}: 0,")),
        }
    }
    code.dedent();
    code.line("})");
    code.dedent();
    code.line("}");
    code.blank();
    for field in def.visible_fields() {
        if let Shape::Callback(callback) = &field.shape {
            let callback = library
                .callback(callback)
                .ok_or_else(|| format!("no callback {callback}"))?;
            trampoline(
                code,
                library,
                &vt_tramp(record, &field.name),
                callback,
                Some(&field.name),
            )?;
        }
    }
    Ok(())
}

/// The Rust type of a callback parameter, as the C function receives it.
fn param_type(library: &Library, shape: &Shape, out: bool) -> Result<String, String> {
    Ok(match shape {
        Shape::Scalar(scalar) => rust_prim(prim_of(library, scalar)).to_owned(),
        Shape::Record(record) => format!("*{} {record}", if out { "mut" } else { "const" }),
        other => return Err(format!("a callback parameter of shape {other:?} is not carried")),
    })
}

/// A function the library calls: it runs the script's function (`method` is `None`) or the method `method` of its object.
fn trampoline(
    code: &mut CodeWriter,
    library: &Library,
    name: &str,
    callback: &Callback,
    method: Option<&str>,
) -> Result<(), String> {
    let mut params = vec!["user: *mut c_void".to_owned()];
    for param in &callback.params {
        params.push(format!(
            "{}: {}",
            rust_ident(&param.name),
            param_type(library, &param.shape, param.out)?
        ));
    }
    let ret = match &callback.ret {
        CallbackRet::Void => String::new(),
        CallbackRet::Status => " -> i32".to_owned(),
        CallbackRet::Value(prim) => format!(" -> {}", rust_prim(*prim)),
    };
    code.line(format!("unsafe extern \"C\" fn {name}({}){ret} {{", params.join(", ")));
    code.indent();
    if let CallbackRet::Value(prim) = &callback.ret {
        code.line(format!(
            "let mut answer: {} = 0{};",
            rust_prim(*prim),
            if *prim == Prim::F64 { ".0" } else { "" }
        ));
    }
    code.line("let code = contain(|| {");
    code.indent();
    code.line(
        "// SAFETY: `user` is the object the arena made for this registration, which the library has not dropped yet.",
    );
    code.line("let object = unsafe { user_of(user) };");
    code.line("let mut args: Vec<Dyn> = Vec::new();");
    let mut out_record: Option<(&str, &str)> = None;
    let mut sink_param: Option<(&str, &str, &str)> = None;
    for param in &callback.params {
        let ident = rust_ident(&param.name);
        match &param.shape {
            Shape::Scalar(scalar) => code.line(format!("args.push({});", to_dyn(prim_of(library, scalar), &ident))),
            Shape::Record(record) if param.out => out_record = Some((&param.name, record)),
            Shape::Record(record) => {
                code.line("// SAFETY: the library passes a record that is valid until the callback returns.");
                code.line(format!("args.push({}(unsafe {{ &*{ident} }}));", reader(record)));
                if let Some(def) = library.record(record) {
                    for field in &def.fields {
                        // Only a callback that answers with a status emits: a hook is told, and answers nothing.
                        if let Shape::Sink(_) = &field.shape
                            && callback.ret == CallbackRet::Status
                        {
                            sink_param = Some((&param.name, record, &field.name));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    match method {
        Some(method) => code.line(format!("let result = object.invoke_method({method:?}, args);")),
        None => code.line("let result = object.invoke(args);"),
    }
    if let Some((param, record, emit_field)) = sink_param {
        let def = library.record(record).ok_or_else(|| format!("no record {record}"))?;
        let Some(Shape::Sink(emit)) = def
            .fields
            .iter()
            .find(|field| field.name == emit_field)
            .map(|field| field.shape.clone())
        else {
            return Err(format!("{record} has no sink"));
        };
        let emit = library.callback(&emit).ok_or_else(|| format!("no callback {emit}"))?;
        let emitted = match emit.params.first().map(|param| &param.shape) {
            Some(Shape::Record(emitted)) => emitted.clone(),
            _ => return Err(format!("the emit function of {record} takes no record")),
        };
        let ident = rust_ident(param);
        code.line("let items = match result {");
        code.indent();
        code.line("Ok(Dyn::List(items)) => items,");
        code.line("Ok(Dyn::Nil) => Vec::new(),");
        code.line("Ok(_) => return E_TYPE,");
        code.line("Err(_) => return E_PANIC,");
        code.dedent();
        code.line("};");
        code.line("for item in &items {");
        code.indent();
        code.line("let mut arena = Arena::new();");
        code.line(format!(
            "let Ok(emitted) = {}(&mut arena, item, \"{emitted}\") else {{ return E_TYPE }};",
            writer(&emitted)
        ));
        code.line(
            "// SAFETY: the context is valid until the callback returns, and `emit` is valid for this call only.",
        );
        code.line(format!(
            "let code = match unsafe {{ {ident}.as_ref() }} {{ Some(cx) => unsafe {{ (cx.emit)(cx.sink, &raw const emitted) }}, None => E_PANIC }};"
        ));
        code.line("if code != E_OK {");
        code.indent();
        code.line("return code;");
        code.dedent();
        code.line("}");
        code.dedent();
        code.line("}");
        code.line("E_OK");
    } else if let Some((param, record)) = out_record {
        let ident = rust_ident(param);
        code.line("match result {");
        code.indent();
        code.line("Ok(Dyn::Nil) => 0,");
        code.line("Ok(item) => {");
        code.indent();
        code.line("let mut arena = Arena::new();");
        code.line(format!("match {}(&mut arena, &item, \"{record}\") {{", writer(record)));
        code.indent();
        code.line("// SAFETY: the library passes a record it owns for this call.");
        code.line(format!("Ok(filled) => {{ unsafe {{ *{ident} = filled }}; 1 }}"));
        code.line("Err(_) => E_TYPE,");
        code.dedent();
        code.line("}");
        code.dedent();
        code.line("}");
        code.line("Err(_) => E_PANIC,");
        code.dedent();
        code.line("}");
    } else {
        match &callback.ret {
            CallbackRet::Value(prim) => {
                code.line("match &result {");
                code.indent();
                code.line(format!(
                    "Ok(item) => {{ answer = {}.unwrap_or_default(); E_OK }}",
                    try_from_dyn(*prim, "item", "\"answer\"")
                ));
                code.line("Err(_) => E_PANIC,");
                code.dedent();
                code.line("}");
            }
            CallbackRet::Status => code.line("status_of(&result)"),
            CallbackRet::Void => {
                code.line("let _ = result;");
                code.line("E_OK");
            }
        }
    }
    code.dedent();
    code.line("});");
    match &callback.ret {
        CallbackRet::Void => code.line("let _ = code;"),
        CallbackRet::Status => code.line("code"),
        CallbackRet::Value(_) => {
            code.line("let _ = code;");
            code.line("answer");
        }
    }
    code.dedent();
    code.line("}");
    code.blank();
    Ok(())
}

//! The one description every SDK target reads.
//!
//! Three sources feed it and nothing else does:
//!
//! 1. the schema catalog (`gnoblin-schema`): classes, properties, methods, signals and error codes of the object model;
//! 2. the core ABI (`gnoblin-core-abi`): constants, handle typedefs, callbacks, `repr(C)` records and the function table;
//! 3. the std ABI (`gnoblin-std-abi`): the same, for the library above the core.
//!
//! The ABI crates already describe themselves as data (`reflect`: every declaration made by the `abi_*!` macros produces a
//! `reflect` entry from the same tokens, with the compiler's own layouts). This module parses those type strings once
//! (`abi_c::parse`, the parser the C header uses) and classifies what it finds into the shapes a script runtime can
//! marshal: a function's arguments become [`Arg`]s (text, a record, a slice, an out value, a callback), a record's fields
//! become [`Shape`]s, and a function whose name starts with an object (`collection_len`) joins that object's class.
//!
//! Nothing here knows a language. A target turns a [`SdkSpec`] into files; `docs/sdk-generator.md` is the contract.

// The std half of the model (records, callbacks, the function table, objects) is classified and tested but has no renderer yet:
// the Python and TypeScript std bindings are the plan in design/modular-framework.md, section 25.2.
#![allow(dead_code)]
use std::path::Path;

use gnoblin_core_abi::reflect::{ConstDef, Reflection};
use gnoblin_schema::{Catalog, Namespace, ObjectType};

use crate::abi_c::{self, FnSig, Ty};
use crate::abi_check::TARGETS;

use super::text::doc_lines;

/// Schema files, relative to the workspace root.
pub const SCHEMA_DIR: &str = gnoblin_schema::proto::SCHEMA_DIR;

/// Everything a target may read.
pub struct SdkSpec {
    /// The schema catalog: the object model of the compositor.
    pub catalog: Catalog,
    /// [`Catalog::fingerprint`]: what `gnoblin_catalog_hash` reports and every generated header records.
    pub catalog_hash: u64,
    /// The libraries' ABIs, core first.
    pub libraries: Vec<Library>,
    /// The workspace root the specification was loaded from: the tools a target runs find their configuration there.
    pub root: std::path::PathBuf,
}

impl SdkSpec {
    /// Loads the catalog from `root` (the workspace root) and reflects both ABIs.
    pub fn load(root: &Path) -> Result<SdkSpec, String> {
        let catalog = Catalog::load_dir(&root.join(SCHEMA_DIR))
            .map_err(|error| format!("the schema catalog does not load: {error}"))?;
        let catalog_hash = catalog.fingerprint();
        let mut libraries: Vec<Library> = Vec::new();
        for target in TARGETS {
            let library = Library::from_reflection(target.name, &(target.reflection)(), libraries.first())?;
            libraries.push(library);
        }
        Ok(SdkSpec {
            catalog,
            catalog_hash,
            libraries,
            root: root.to_path_buf(),
        })
    }

    /// The library called `name` (`core` or `std`).
    pub fn library(&self, name: &str) -> Option<&Library> {
        self.libraries.iter().find(|library| library.name == name)
    }

    /// The core library, which every target needs.
    pub fn core(&self) -> &Library {
        self.library("core").expect("the core ABI is always reflected")
    }

    /// The namespace called `name`.
    pub fn namespace(&self, name: &str) -> Namespace {
        self.catalog
            .namespaces()
            .into_iter()
            .find(|namespace| namespace.as_str() == name)
            .unwrap_or_else(|| panic!("the catalog has no `{name}` namespace"))
    }

    /// The classes of the `tree` namespace that derive from `node`, bases before the classes that derive from them. `node`
    /// itself is the first when `include_node` is set. A class that is not a node (a protocol message, a record) is not here.
    pub fn node_classes(&self, include_node: bool) -> Vec<ObjectType> {
        let tree = self.namespace("tree");
        let objects: Vec<&ObjectType> = self.catalog.objects(&tree).collect();
        let derives_node = |object: &ObjectType| {
            let mut base = object.base.as_deref();
            let mut steps = 0;
            while let Some(name) = base {
                if name == "node" {
                    return true;
                }
                base = objects
                    .iter()
                    .find(|other| other.name == name)
                    .and_then(|other| other.base.as_deref());
                steps += 1;
                if steps > 32 {
                    return false;
                }
            }
            false
        };
        let mut pending: Vec<ObjectType> = objects
            .iter()
            .filter(|object| (include_node && object.name == "node") || (object.name != "node" && derives_node(object)))
            .map(|object| (*object).clone())
            .collect();
        // Bases first: a class is placed once its base is, which keeps the catalog's order otherwise.
        let mut ordered: Vec<ObjectType> = Vec::new();
        while !pending.is_empty() {
            let position = pending
                .iter()
                .position(|object| {
                    object.base.as_deref().is_none_or(|base| {
                        base == "node" && !include_node || ordered.iter().any(|placed| placed.name == base)
                    })
                })
                .unwrap_or(0);
            ordered.push(pending.remove(position));
        }
        ordered
    }
}

/// One library's ABI.
pub struct Library {
    /// `core` or `std`.
    pub name: String,
    /// The SONAME.
    pub soname: String,
    /// `(major, minor)` of this build of the ABI crate.
    pub abi: (u32, u32),
    /// Constants that belong together (`STD_EVENT_*`), in declaration order.
    pub groups: Vec<ConstGroup>,
    /// Constants that stand alone.
    pub loose: Vec<Const>,
    /// Handle typedefs (`GnoblinStdCollection = u64`).
    pub handles: Vec<HandleDef>,
    /// Callback typedefs.
    pub callbacks: Vec<Callback>,
    /// `repr(C)` records, other than the function table.
    pub records: Vec<StructDef>,
    /// The function table's entries, in table order.
    pub functions: Vec<Function>,
}

/// An integer constant.
#[derive(Clone, Debug)]
pub struct Const {
    /// The full ABI name (`STD_EVENT_ADDED`).
    pub name: String,
    /// The name inside its group (`ADDED`).
    pub short: String,
    /// The value.
    pub value: i128,
    /// The doc comment, one entry per line.
    pub doc: Vec<String>,
}

/// Constants that share a prefix: an enumeration or a set of flag bits.
#[derive(Clone, Debug)]
pub struct ConstGroup {
    /// The group's name in PascalCase without the library prefix (`Event` for `STD_EVENT_*`).
    pub name: String,
    /// The prefix the members share (`STD_EVENT_`).
    pub prefix: String,
    /// True when every member is a distinct single bit, so the group is a flag set and not an enumeration.
    pub flags: bool,
    /// The members.
    pub members: Vec<Const>,
}

impl ConstGroup {
    /// The name languages give the group's type: `EventKind` for an enumeration, `PlacementFlags` for a set of bits. The
    /// suffix keeps it from colliding with a record or a class of the same name (`Event`, `Collection`).
    pub fn type_name(&self) -> String {
        format!("{}{}", self.name, if self.flags { "Flags" } else { "Kind" })
    }
}

/// A handle typedef.
#[derive(Clone, Debug)]
pub struct HandleDef {
    /// `GnoblinStdCollection`.
    pub name: String,
    /// The integer type it is (`u64`).
    pub repr: Prim,
    /// The doc comment.
    pub doc: Vec<String>,
}

/// The integer and float types the ABI uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    U8,
    U32,
    I32,
    U64,
    I64,
    Usize,
    F64,
}

impl Prim {
    fn parse(name: &str) -> Option<Prim> {
        Some(match name {
            "u8" => Prim::U8,
            "u32" => Prim::U32,
            "i32" => Prim::I32,
            "u64" => Prim::U64,
            "i64" => Prim::I64,
            "usize" => Prim::Usize,
            "f64" => Prim::F64,
            _ => return None,
        })
    }

    /// True for `f64`.
    pub fn is_float(self) -> bool {
        self == Prim::F64
    }

    /// True when a JavaScript number (a double) cannot hold every value: 64-bit integers.
    pub fn is_wide(self) -> bool {
        matches!(self, Prim::U64 | Prim::I64 | Prim::Usize)
    }
}

/// A scalar a script can pass or receive: a number or a handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scalar {
    /// A primitive.
    Prim(Prim),
    /// A handle typedef, by name.
    Handle(String),
}

/// What a record field is, as far as a script is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A number or a handle.
    Scalar(Scalar),
    /// A `GnoblinStr`: UTF-8 text.
    Text,
    /// Another record, by value.
    Record(String),
    /// A callback typedef (optional or not).
    Callback(String),
    /// A pointer that is not part of a recognised pair: scripts cannot represent it.
    Pointer(String),
}

/// One field of a record.
#[derive(Clone, Debug)]
pub struct FieldDef {
    /// The field name.
    pub name: String,
    /// What it is.
    pub shape: Shape,
    /// The doc comment.
    pub doc: Vec<String>,
}

/// A `repr(C)` record.
#[derive(Clone, Debug)]
pub struct StructDef {
    /// `GnoblinStdPlacement`.
    pub name: String,
    /// The doc comment.
    pub doc: Vec<String>,
    /// The fields in order.
    pub fields: Vec<FieldDef>,
    /// True when the first field is `size: u32`: the caller sets it and the library fills what the size covers.
    pub extensible: bool,
    /// True when every field is something a script can hold (no raw pointer, no callback).
    pub plain: bool,
}

impl StructDef {
    /// The fields a script sees: not the `size` of an extensible record, and not `reserved`.
    pub fn visible_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields
            .iter()
            .filter(move |field| !(field.name == "reserved" || (self.extensible && field.name == "size")))
    }
}

/// A callback typedef.
#[derive(Clone, Debug)]
pub struct Callback {
    /// `GnoblinStdEventFn`.
    pub name: String,
    /// The doc comment.
    pub doc: Vec<String>,
    /// The parameters after the leading `user` pointer.
    pub params: Vec<CallbackParam>,
    /// What it returns.
    pub ret: CallbackRet,
}

/// One parameter of a callback.
#[derive(Clone, Debug)]
pub struct CallbackParam {
    /// The name.
    pub name: String,
    /// What it is: a record pointer is a record, a scalar a scalar.
    pub shape: Shape,
}

/// What a callback returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallbackRet {
    /// Nothing.
    Void,
    /// An `i32` status (`E_OK` or an error code).
    Status,
    /// A number.
    Value(Prim),
}

/// One argument of a table function, after pairs are merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// The generation context: the runtime supplies it.
    Ctx,
    /// A scalar passed by value.
    Value { name: String, ty: Scalar },
    /// `(*const u8, usize)`: a string.
    Text { name: String },
    /// `*const Record`: one record read by the library.
    Record { name: String, record: String },
    /// `(*const Record, usize)`: an array of records.
    Records { name: String, record: String },
    /// `*mut T` of a scalar: a value the library writes; the call returns it.
    Out { name: String, ty: Scalar },
    /// `*mut Record`: a record the library fills; the call returns it.
    OutRecord { name: String, record: String },
    /// `(Callback, *mut c_void user, Option<DropFn> drop_user)`: a closure the library keeps.
    Callback { name: String, callback: String },
    /// Anything else: the function is native-only.
    Raw { name: String, ty: String },
}

impl Arg {
    /// The script-visible name, or `None` for the context.
    pub fn name(&self) -> Option<&str> {
        match self {
            Arg::Ctx => None,
            Arg::Value { name, .. }
            | Arg::Text { name }
            | Arg::Record { name, .. }
            | Arg::Records { name, .. }
            | Arg::Out { name, .. }
            | Arg::OutRecord { name, .. }
            | Arg::Callback { name, .. }
            | Arg::Raw { name, .. } => Some(name),
        }
    }

    /// True for the arguments a caller passes in (everything but the context and the outs).
    pub fn is_input(&self) -> bool {
        !matches!(self, Arg::Ctx | Arg::Out { .. } | Arg::OutRecord { .. })
    }
}

/// What a table function returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ret {
    /// `i32`: negative is an error, otherwise a value (often 0 or 1).
    Status,
    /// A handle typedef where zero means failure.
    Handle(String),
    /// A number.
    Value(Prim),
    /// Nothing.
    Void,
}

/// One entry of the function table.
#[derive(Clone, Debug)]
pub struct Function {
    /// The table field and so the function's name (`collection_len`).
    pub name: String,
    /// The doc comment.
    pub doc: Vec<String>,
    /// The arguments, context included, in call order.
    pub args: Vec<Arg>,
    /// The return.
    pub ret: Ret,
    /// The object the name starts with (`collection`), when the name has the `object_verb` shape.
    pub object: Option<String>,
    /// The name without the object (`len`); the whole name when there is no object.
    pub verb: String,
    /// The argument that is the receiver: the input scalar named like the object (`collection`).
    pub receiver: Option<usize>,
    /// True when no argument is [`Arg::Raw`] and every record it names is plain: a script runtime can bind it.
    pub scriptable: bool,
}

impl Function {
    /// The arguments a caller passes, in order.
    pub fn inputs(&self) -> impl Iterator<Item = &Arg> {
        self.args.iter().filter(|arg| arg.is_input())
    }

    /// The values the call hands back besides the return: the outs.
    pub fn outs(&self) -> impl Iterator<Item = &Arg> {
        self.args
            .iter()
            .filter(|arg| matches!(arg, Arg::Out { .. } | Arg::OutRecord { .. }))
    }
}

/// An object of the std library's model: the functions that start with its name.
#[derive(Clone, Debug)]
pub struct StdObject {
    /// `collection`.
    pub name: String,
    /// The receiver's type, when the object has instance functions.
    pub receiver: Option<Scalar>,
    /// The functions, in table order.
    pub functions: Vec<usize>,
}

impl Library {
    /// The record called `name`.
    pub fn record(&self, name: &str) -> Option<&StructDef> {
        self.records.iter().find(|record| record.name == name)
    }

    /// The callback typedef called `name`.
    pub fn callback(&self, name: &str) -> Option<&Callback> {
        self.callbacks.iter().find(|callback| callback.name == name)
    }

    /// The handle typedef called `name`.
    pub fn handle(&self, name: &str) -> Option<&HandleDef> {
        self.handles.iter().find(|handle| handle.name == name)
    }

    /// The prefix this library's constants carry (`STD_` for std, empty for the core).
    pub fn const_prefix(&self) -> &'static str {
        if self.name == "std" { "STD_" } else { "" }
    }

    /// The table's functions grouped by the object their name starts with, in first-appearance order. Functions with no
    /// object (`open`, `stats`, `remove`) are not listed.
    pub fn objects(&self) -> Vec<StdObject> {
        let mut objects: Vec<StdObject> = Vec::new();
        for (index, function) in self.functions.iter().enumerate() {
            let Some(name) = &function.object else { continue };
            let position = match objects.iter().position(|object| &object.name == name) {
                Some(position) => position,
                None => {
                    objects.push(StdObject {
                        name: name.clone(),
                        receiver: None,
                        functions: Vec::new(),
                    });
                    objects.len() - 1
                }
            };
            let object = &mut objects[position];
            object.functions.push(index);
            if object.receiver.is_none()
                && let Some(at) = function.receiver
                && let Arg::Value { ty, .. } = &function.args[at]
            {
                object.receiver = Some(ty.clone());
            }
        }
        objects
    }

    /// Builds a library from its ABI crate's reflection. `parent` is the library below it (the core, for std): its handles
    /// are the std library's own to name.
    pub fn from_reflection(name: &str, reflection: &Reflection, parent: Option<&Library>) -> Result<Library, String> {
        let prefix = if name == "std" { "STD_" } else { "" };
        let (groups, loose) = group_consts(reflection.consts, prefix);
        let handles = reflection
            .aliases
            .iter()
            .map(|alias| {
                let repr = match abi_c::parse(alias.ty)? {
                    Ty::Named(inner) => Prim::parse(&inner)
                        .ok_or_else(|| format!("handle `{}` is not an integer: {}", alias.name, alias.ty))?,
                    other => return Err(format!("handle `{}` has the unsupported type {other:?}", alias.name)),
                };
                Ok(HandleDef {
                    name: alias.name.to_owned(),
                    repr,
                    doc: doc_lines(alias.doc),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let ctx = Classifier {
            handles: parent
                .into_iter()
                .flat_map(|parent| parent.handles.iter())
                .chain(handles.iter())
                .map(|handle| handle.name.clone())
                .collect(),
            records: reflection.structs.iter().map(|layout| layout.name.to_owned()).collect(),
            callbacks: reflection
                .callbacks
                .iter()
                .map(|callback| callback.name.to_owned())
                .collect(),
        };
        let callbacks = reflection
            .callbacks
            .iter()
            .map(|callback| ctx.callback(callback.name, callback.ty, callback.doc))
            .collect::<Result<Vec<_>, String>>()?;
        let mut records = Vec::new();
        let mut functions = Vec::new();
        for layout in reflection.structs {
            if layout.name.ends_with("Api") && layout.fields.iter().any(|field| field.ty.contains("extern")) {
                for field in layout.fields.iter().filter(|field| field.ty.contains("extern")) {
                    functions.push(ctx.function(field.name, field.ty, field.doc)?);
                }
                continue;
            }
            records.push(ctx.record(layout.name, layout.doc, layout.fields)?);
        }
        // A record is plain when every field is, and a nested record is plain when it is: settle that to a fixed point.
        loop {
            let plain: Vec<(String, bool)> = records
                .iter()
                .map(|record| {
                    let ok = record.fields.iter().all(|field| match &field.shape {
                        Shape::Scalar(_) | Shape::Text => true,
                        Shape::Record(inner) => records.iter().any(|other| &other.name == inner && other.plain),
                        Shape::Callback(_) | Shape::Pointer(_) => false,
                    });
                    (record.name.clone(), ok)
                })
                .collect();
            let mut changed = false;
            for (record, (_, ok)) in records.iter_mut().zip(plain) {
                if record.plain != ok {
                    record.plain = ok;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for function in &mut functions {
            function.scriptable = function.args.iter().all(|arg| match arg {
                Arg::Raw { .. } => false,
                Arg::Record { record, .. } | Arg::Records { record, .. } | Arg::OutRecord { record, .. } => {
                    records.iter().any(|other| &other.name == record && other.plain)
                }
                _ => true,
            });
        }
        Ok(Library {
            name: name.to_owned(),
            soname: reflection.soname.to_owned(),
            abi: (reflection.major, reflection.minor),
            groups,
            loose,
            handles,
            callbacks,
            records,
            functions,
        })
    }
}

/// The names the type classifier needs to tell a record from a handle from a callback.
struct Classifier {
    handles: Vec<String>,
    records: Vec<String>,
    callbacks: Vec<String>,
}

fn prim_of(ty: &Ty) -> Option<Prim> {
    match ty {
        Ty::Named(name) => Prim::parse(name),
        _ => None,
    }
}

impl Classifier {
    fn scalar(&self, ty: &Ty) -> Option<Scalar> {
        match ty {
            Ty::Named(name) if self.handles.contains(name) => Some(Scalar::Handle(name.clone())),
            other => prim_of(other).map(Scalar::Prim),
        }
    }

    /// `*const T` or `*mut T`: the pointee and whether it is mutable.
    fn pointee<'a>(&self, ty: &'a Ty) -> Option<(&'a Ty, bool)> {
        match ty {
            Ty::Ptr { mutable, to } => Some((to, *mutable)),
            _ => None,
        }
    }

    fn record_name<'a>(&self, ty: &'a Ty) -> Option<&'a str> {
        match ty {
            Ty::Named(name) if self.records.contains(name) => Some(name),
            _ => None,
        }
    }

    fn shape(&self, ty: &Ty) -> Shape {
        if let Some(scalar) = self.scalar(ty) {
            return Shape::Scalar(scalar);
        }
        match ty {
            Ty::Named(name) if name == "GnoblinStr" => Shape::Text,
            Ty::Named(name) if self.records.contains(name) => Shape::Record(name.clone()),
            Ty::Named(name) if self.callbacks.contains(name) => Shape::Callback(name.clone()),
            Ty::Option(inner) => match inner.as_ref() {
                Ty::Named(name) if self.callbacks.contains(name) => Shape::Callback(name.clone()),
                other => Shape::Pointer(abi_c::canonical(other)),
            },
            other => Shape::Pointer(abi_c::canonical(other)),
        }
    }

    fn callback(&self, name: &str, ty: &str, doc: &str) -> Result<Callback, String> {
        let Ty::Fn(sig) = abi_c::parse(ty)? else {
            return Err(format!("callback `{name}` is not a function type: {ty}"));
        };
        // The first parameter is the `user` pointer the library hands back; a script closure holds its own state.
        if sig.params.is_empty() {
            return Err(format!("callback `{name}` takes no `user` pointer"));
        }
        let params = sig.params.iter().skip(1);
        let params = params
            .enumerate()
            .map(|(index, (param, ty))| {
                let shape = match self.pointee(ty) {
                    Some((inner, _)) => match self.record_name(inner) {
                        Some(record) => Shape::Record(record.to_owned()),
                        None => self.shape(ty),
                    },
                    None => self.shape(ty),
                };
                CallbackParam {
                    name: param.clone().unwrap_or_else(|| format!("arg{index}")),
                    shape,
                }
            })
            .collect();
        let ret = match sig.ret.as_deref() {
            None => CallbackRet::Void,
            Some(ty) => match prim_of(ty) {
                Some(Prim::I32) => CallbackRet::Status,
                Some(prim) => CallbackRet::Value(prim),
                None => return Err(format!("callback `{name}` returns an unsupported type")),
            },
        };
        Ok(Callback {
            name: name.to_owned(),
            doc: doc_lines(doc),
            params,
            ret,
        })
    }

    fn record(
        &self,
        name: &str,
        doc: &str,
        fields: &[gnoblin_core_abi::reflect::FieldLayout],
    ) -> Result<StructDef, String> {
        let fields = fields
            .iter()
            .map(|field| {
                let ty = abi_c::parse(field.ty)?;
                Ok(FieldDef {
                    name: field.name.to_owned(),
                    shape: self.shape(&ty),
                    doc: doc_lines(field.doc),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let extensible = fields
            .first()
            .is_some_and(|field| field.name == "size" && field.shape == Shape::Scalar(Scalar::Prim(Prim::U32)));
        Ok(StructDef {
            name: name.to_owned(),
            doc: doc_lines(doc),
            fields,
            extensible,
            plain: true,
        })
    }

    fn function(&self, name: &str, ty: &str, doc: &str) -> Result<Function, String> {
        let Ty::Fn(sig) = abi_c::parse(ty)? else {
            return Err(format!("table entry `{name}` is not a function type: {ty}"));
        };
        let args = self.args(&sig)?;
        let ret = match sig.ret.as_deref() {
            None => Ret::Void,
            Some(ty) => match self.scalar(ty) {
                Some(Scalar::Prim(Prim::I32)) => Ret::Status,
                Some(Scalar::Handle(handle)) => Ret::Handle(handle),
                Some(Scalar::Prim(prim)) => Ret::Value(prim),
                None => return Err(format!("table entry `{name}` returns an unsupported type")),
            },
        };
        let (object, verb) = match name.split_once('_') {
            Some((object, verb)) if !verb.is_empty() => (Some(object.to_owned()), verb.to_owned()),
            _ => (None, name.to_owned()),
        };
        let receiver = object.as_ref().and_then(|object| {
            args.iter()
                .position(|arg| matches!(arg, Arg::Value { name, .. } if name == object))
        });
        Ok(Function {
            name: name.to_owned(),
            doc: doc_lines(doc),
            args,
            ret,
            object,
            verb,
            receiver,
            scriptable: true,
        })
    }

    fn args(&self, sig: &FnSig) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        let params = &sig.params;
        let mut index = 0;
        while index < params.len() {
            let (name, ty) = &params[index];
            let name = name.clone().unwrap_or_else(|| format!("arg{index}"));
            let next = params.get(index + 1);
            let after = params.get(index + 2);
            // The generation context, always first.
            if index == 0 && matches!(ty, Ty::Named(ty) if ty == "GnoblinCtx") {
                args.push(Arg::Ctx);
                index += 1;
                continue;
            }
            // A callback registration: `cb, user: *mut c_void, drop_user: Option<GnoblinDropFn>`.
            if let Ty::Named(callback) = ty
                && self.callbacks.contains(callback)
                && let Some((Some(user), Ty::Ptr { mutable: true, .. })) = next
                && user == "user"
                && let Some((_, Ty::Option(_))) = after
            {
                args.push(Arg::Callback {
                    name,
                    callback: callback.clone(),
                });
                index += 3;
                continue;
            }
            // A string or an array: a pointer followed by its length.
            if let Some((pointee, false)) = self.pointee(ty)
                && let Some((_, Ty::Named(len))) = next
                && len == "usize"
            {
                if matches!(pointee, Ty::Named(inner) if inner == "u8") {
                    args.push(Arg::Text { name });
                    index += 2;
                    continue;
                }
                if let Some(record) = self.record_name(pointee) {
                    args.push(Arg::Records {
                        name,
                        record: record.to_owned(),
                    });
                    index += 2;
                    continue;
                }
            }
            match self.pointee(ty) {
                Some((pointee, false)) if self.record_name(pointee).is_some() => {
                    args.push(Arg::Record {
                        name,
                        record: self.record_name(pointee).unwrap_or_default().to_owned(),
                    });
                }
                Some((pointee, true)) => {
                    if let Some(record) = self.record_name(pointee) {
                        args.push(Arg::OutRecord {
                            name,
                            record: record.to_owned(),
                        });
                    } else if let Some(scalar) = self.scalar(pointee) {
                        args.push(Arg::Out { name, ty: scalar });
                    } else {
                        args.push(Arg::Raw {
                            name,
                            ty: abi_c::canonical(ty),
                        });
                    }
                }
                Some(_) => args.push(Arg::Raw {
                    name,
                    ty: abi_c::canonical(ty),
                }),
                None => match self.scalar(ty) {
                    Some(scalar) => args.push(Arg::Value { name, ty: scalar }),
                    None => args.push(Arg::Raw {
                        name,
                        ty: abi_c::canonical(ty),
                    }),
                },
            }
            index += 1;
        }
        Ok(args)
    }
}

/// Splits the constants into groups (`STD_EVENT_*`) and loose ones. A group is the first word after the library prefix
/// when at least two constants share it.
fn group_consts(consts: &[ConstDef], library_prefix: &str) -> (Vec<ConstGroup>, Vec<Const>) {
    let key_of = |name: &str| -> String {
        let rest = name.strip_prefix(library_prefix).unwrap_or(name);
        rest.split('_').next().unwrap_or(rest).to_owned()
    };
    let mut keys: Vec<String> = Vec::new();
    for constant in consts {
        let key = key_of(constant.name);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mut groups = Vec::new();
    let mut loose = Vec::new();
    for key in keys {
        let members: Vec<&ConstDef> = consts.iter().filter(|constant| key_of(constant.name) == key).collect();
        let prefix = format!("{library_prefix}{key}_");
        let build = |constant: &ConstDef, strip: &str| Const {
            name: constant.name.to_owned(),
            short: constant.name.strip_prefix(strip).unwrap_or(constant.name).to_owned(),
            value: constant.value,
            doc: doc_lines(constant.doc),
        };
        if members.len() < 2 {
            loose.extend(members.iter().map(|constant| build(constant, library_prefix)));
            continue;
        }
        let members: Vec<Const> = members.iter().map(|constant| build(constant, &prefix)).collect();
        let flags = members.len() >= 2
            && members
                .iter()
                .all(|member| member.value > 0 && (member.value & (member.value - 1)) == 0)
            && {
                let mut values: Vec<i128> = members.iter().map(|member| member.value).collect();
                values.sort_unstable();
                values.windows(2).all(|pair| pair[0] != pair[1])
            }
            && members.iter().any(|member| member.value > 1);
        groups.push(ConstGroup {
            name: group_name(&key),
            prefix,
            flags,
            members,
        });
    }
    (groups, loose)
}

/// The PascalCase name of a constant group. `E` is the error codes.
fn group_name(key: &str) -> String {
    match key {
        "E" => "Error".to_owned(),
        other => super::text::pascal(&other.to_ascii_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SdkSpec {
        SdkSpec::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("xtask lives in the workspace"),
        )
        .expect("the specification loads")
    }

    #[test]
    fn the_catalog_hash_is_the_schema_fingerprint() {
        let spec = spec();
        assert_eq!(spec.catalog_hash, spec.catalog.fingerprint());
        assert_ne!(spec.catalog_hash, 0);
    }

    #[test]
    fn the_core_table_is_reflected_in_full() {
        let spec = spec();
        let core = spec.core();
        assert!(core.functions.len() >= 40, "{} functions", core.functions.len());
        let write_set = core
            .functions
            .iter()
            .find(|function| function.name == "write_set")
            .expect("write_set");
        assert!(matches!(write_set.args[0], Arg::Ctx));
        assert!(core.groups.iter().any(|group| group.name == "Error"));
        assert!(
            core.records.iter().all(|record| record.name != "GnoblinApi"),
            "the table is not a record"
        );
    }

    #[test]
    fn std_functions_group_into_objects_with_receivers() {
        let spec = spec();
        let std = spec.library("std").expect("std is reflected");
        let objects = std.objects();
        let names: Vec<&str> = objects.iter().map(|object| object.name.as_str()).collect();
        for expected in ["collection", "client", "workspace", "policy", "output"] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }
        let collection = objects
            .iter()
            .find(|object| object.name == "collection")
            .expect("collection");
        assert_eq!(
            collection.receiver,
            Some(Scalar::Handle("GnoblinStdCollection".to_owned()))
        );
        let client = objects.iter().find(|object| object.name == "client").expect("client");
        assert_eq!(client.receiver, Some(Scalar::Handle("GnoblinNode".to_owned())));
    }

    #[test]
    fn std_arguments_are_classified() {
        let spec = spec();
        let std = spec.library("std").expect("std is reflected");
        let find = |name: &str| {
            std.functions
                .iter()
                .find(|function| function.name == name)
                .expect("function")
        };
        let listen = find("collection_listen");
        assert!(
            listen
                .args
                .iter()
                .any(|arg| matches!(arg, Arg::Callback { callback, .. } if callback == "GnoblinStdEventFn"))
        );
        assert_eq!(listen.ret, Ret::Handle("GnoblinStdReg".to_owned()));
        let by_id = find("workspace_by_id");
        assert!(
            by_id
                .args
                .iter()
                .any(|arg| matches!(arg, Arg::Text { name } if name == "id"))
        );
        assert!(by_id.args.iter().any(|arg| matches!(arg, Arg::Out { .. })));
        let wher = find("collection_where");
        assert!(
            wher.args
                .iter()
                .any(|arg| matches!(arg, Arg::Records { record, .. } if record == "GnoblinStdQueryTerm"))
        );
        assert_eq!(find("collection_len").receiver, Some(1));
        assert!(find("collection_len").scriptable);
    }

    #[test]
    fn constants_group_by_prefix_and_flags_are_distinct_bits() {
        let spec = spec();
        let std = spec.library("std").expect("std is reflected");
        let event = std
            .groups
            .iter()
            .find(|group| group.name == "Event")
            .expect("STD_EVENT_*");
        assert!(!event.flags);
        assert_eq!(event.members[0].short, "ADDED");
        let placement = std
            .groups
            .iter()
            .find(|group| group.name == "Placement")
            .expect("STD_PLACEMENT_*");
        assert!(placement.flags);
    }
}

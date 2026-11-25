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

use std::path::Path;

use gnoblin_core_abi::reflect::{ConstDef, Reflection};
use gnoblin_schema::{Catalog, Namespace, ObjectType};

use crate::abi_c::{self, FnSig, Ty};

use super::text::doc_lines;

/// Schema files, relative to the workspace root.
pub const SCHEMA_DIR: &str = gnoblin_schema::proto::SCHEMA_DIR;

/// The catalogs of the namespaces the libraries serve, relative to the workspace root: `gnoblin.model` beside the core's ABI,
/// `gnoblin.std` beside the std library's. One file per module, one lock per file.
pub const MODEL_DIRS: [&str; 2] = ["crates/gnoblin-core-abi/schema", "crates/gnoblin-std-abi/schema"];

/// Everything a target may read.
pub struct SdkSpec {
    /// The schema catalog: the object model of the compositor.
    pub catalog: Catalog,
    /// [`Catalog::fingerprint`]: what `gnoblin_catalog_hash` reports and every generated header records.
    pub catalog_hash: u64,
    /// The catalogs of the namespaces the libraries serve through the object protocol (`gnoblin.model`, `gnoblin.std`), loaded
    /// together so a class of one can name a class of the other.
    pub objects: Catalog,
    /// The libraries' ABIs, core first.
    pub libraries: Vec<Library>,
    /// The workspace root the specification was loaded from, where the locks of the namespaces are (`surface::module_units`). `None`
    /// for a specification loaded from the packaged crates ([`SdkSpec::load_packaged`]), which have no workspace.
    pub root: Option<std::path::PathBuf>,
}

impl SdkSpec {
    /// Loads the catalog from `root` (the workspace root) and reflects both ABIs.
    pub fn load(root: &Path) -> Result<SdkSpec, String> {
        let model_dirs = MODEL_DIRS.map(|dir| root.join(dir));
        SdkSpec::load_from(&root.join(SCHEMA_DIR), &model_dirs, Some(root))
    }

    /// Loads the catalog from the schema files the crates this generator was compiled with carry (`gnoblin_schema::proto::SCHEMA_PATH`,
    /// `gnoblin_core_abi::SCHEMA_PATH`, `gnoblin_std_abi::SCHEMA_PATH`), and reflects both ABIs. A build script uses it: in the
    /// workspace those are the workspace's own files, and in a crate from crates.io they are the files the packaged crates ship.
    pub fn load_packaged() -> Result<SdkSpec, String> {
        let model_dirs = [
            std::path::PathBuf::from(gnoblin_core_abi::SCHEMA_PATH),
            std::path::PathBuf::from(gnoblin_std_abi::SCHEMA_PATH),
        ];
        SdkSpec::load_from(Path::new(gnoblin_schema::proto::SCHEMA_PATH), &model_dirs, None)
    }

    /// The directories every specification reads, in the order of [`SdkSpec::load_packaged`]: what a build script reruns on.
    pub fn packaged_dirs() -> [&'static str; 3] {
        [
            gnoblin_schema::proto::SCHEMA_PATH,
            gnoblin_core_abi::SCHEMA_PATH,
            gnoblin_std_abi::SCHEMA_PATH,
        ]
    }

    /// Loads the schema catalog from `schema`, the catalogs of the libraries from `model_dirs` (core, then std), and reflects both ABIs.
    fn load_from(schema: &Path, model_dirs: &[std::path::PathBuf; 2], root: Option<&Path>) -> Result<SdkSpec, String> {
        let catalog =
            Catalog::load_dir(schema).map_err(|error| format!("the schema catalog does not load: {error}"))?;
        let catalog_hash = catalog.fingerprint();
        let dirs: Vec<&Path> = model_dirs.iter().map(std::path::PathBuf::as_path).collect();
        // The built-in layouts are the std catalog's file `layouts.toml`, written from `gnoblin_std_abi::layouts::LAYOUT_KINDS`
        // (`layout_schema.rs`) and never on disk.
        let layouts_origin = model_dirs[1]
            .join(crate::layout_schema::ORIGIN_FILE)
            .display()
            .to_string();
        let layouts = crate::layout_schema::schema_text();
        let objects = Catalog::load_dirs_with(&dirs, &[], &[(layouts_origin.as_str(), layouts.as_str())])
            .map_err(|error| format!("the catalogs of the libraries do not load: {error}"))?;
        let mut libraries: Vec<Library> = Vec::new();
        for (name, reflection) in [
            ("core", gnoblin_core_abi::reflect::reflection as fn() -> Reflection),
            ("std", gnoblin_std_abi::reflection),
        ] {
            let library = Library::from_reflection(name, &reflection(), libraries.first())?;
            libraries.push(library);
        }
        Ok(SdkSpec {
            catalog,
            catalog_hash,
            objects,
            libraries,
            root: root.map(Path::to_path_buf),
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
    /// The records one of whose fields (`kind`) says which of the others hold the value.
    pub tagged: Vec<Tagged>,
}

/// A record with a `kind` that picks the fields in use (`GnoblinStdProp`, `GnoblinStdStyleEntry`).
#[derive(Clone, Debug)]
pub struct Tagged {
    /// The record.
    pub record: String,
    /// The name languages give the group's type (`PropKind`).
    pub type_name: String,
    /// One per kind, in the group's order.
    pub variants: Vec<Variant>,
}

/// One kind of a [`Tagged`] record.
#[derive(Clone, Debug)]
pub struct Variant {
    /// The constant's short name (`NUMBER`).
    pub short: String,
    /// The fields the kind reads, in the record's order of mention.
    pub fields: Vec<String>,
    /// The constant's doc.
    pub doc: Vec<String>,
}

/// The transition shape: `begin`, a script closure, and `commit` or `abort`.
#[derive(Clone, Debug)]
pub struct Bracket {
    /// The object (`transition`).
    pub object: String,
    /// Indexes of the three functions in the table.
    pub begin: usize,
    /// The one that ends staging and returns the result.
    pub commit: usize,
    /// The one that ends staging and drops what was staged.
    pub abort: usize,
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
    /// `*const Record`, null when absent: an optional nested record.
    RecordPtr(String),
    /// A pointer and the count of what it points at, as two fields (`children` and `n_children`, or `ptr` and `len`): a
    /// list the library lends for the call.
    Array(Elem),
    /// `sink: *mut c_void` and `emit: Callback`, as two fields: the callee gives each result to `emit(sink, result)`.
    Sink(String),
    /// `vtable: *const Record`, `user: *mut c_void` and `drop_user: Option<DropFn>`, as three fields: an object whose
    /// methods the library calls (the record names the functions), and the destructor of the object.
    Vtable(String),
    /// A pointer that is not part of a recognised pair: scripts cannot represent it.
    Pointer(String),
}

/// What an [`Shape::Array`] holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Elem {
    /// Numbers or handles.
    Scalar(Scalar),
    /// Records, by value.
    Record(String),
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
    /// A callback that may be absent (`Option<Fn>`).
    pub optional: bool,
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
    /// True when every field is something a script can hold: a scalar, text, a record that is, a list, a sink or a vtable.
    /// A callback or a raw pointer that is not part of a recognised pair is not.
    pub plain: bool,
    /// The record is nothing but a pointer and its count (`GnoblinStdMembers`): a script sees the list, not the record.
    pub list_of: Option<Elem>,
    /// Every field after `size` and `reserved` is a callback: the record is the table of functions of a [`Shape::Vtable`].
    pub vtable: bool,
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
    /// A `*mut Record`: the script does not receive it, the callback fills it from what the script answers.
    pub out: bool,
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
    /// `*mut Record` with room for `count` of them (the doc says "room for `count`"): the call returns the list.
    OutRecords {
        name: String,
        record: String,
        count: String,
    },
    /// `*mut GnoblinStr`: text the library lends; the call returns a copy.
    OutText { name: String },
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
            | Arg::OutRecords { name, .. }
            | Arg::OutText { name }
            | Arg::Callback { name, .. }
            | Arg::Raw { name, .. } => Some(name),
        }
    }

    /// True for the arguments a caller passes in (everything but the context and the outs).
    pub fn is_input(&self) -> bool {
        !matches!(
            self,
            Arg::Ctx | Arg::Out { .. } | Arg::OutRecord { .. } | Arg::OutRecords { .. } | Arg::OutText { .. }
        )
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
    /// True when no argument is [`Arg::Raw`], every record it names is plain and every callback it takes is scriptable: a
    /// script runtime can bind it.
    pub scriptable: bool,
    /// A status the doc says is 1 or 0 (`1 when it moved, 0 when it was refused`): a script sees a boolean, or, when the call
    /// has an out, the out or nothing.
    pub flag: bool,
}

impl Function {
    /// The arguments a caller passes, in order.
    pub fn inputs(&self) -> impl Iterator<Item = &Arg> {
        self.args.iter().filter(|arg| arg.is_input())
    }

    /// The values the call hands back besides the return: the outs.
    pub fn outs(&self) -> impl Iterator<Item = &Arg> {
        self.args.iter().filter(|arg| {
            matches!(
                arg,
                Arg::Out { .. } | Arg::OutRecord { .. } | Arg::OutRecords { .. } | Arg::OutText { .. }
            )
        })
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

    /// The `begin` / `commit` / `abort` triples of the table: a script wraps its closure in them.
    pub fn brackets(&self) -> Vec<Bracket> {
        let find = |object: &str, verb: &str| {
            self.functions
                .iter()
                .position(|function| function.object.as_deref() == Some(object) && function.verb == verb)
        };
        self.objects()
            .into_iter()
            .filter_map(|object| {
                Some(Bracket {
                    begin: find(&object.name, "begin")?,
                    commit: find(&object.name, "commit")?,
                    abort: find(&object.name, "abort")?,
                    object: object.name,
                })
            })
            .collect()
    }

    /// The tagged record called `name`.
    pub fn tagged_record(&self, name: &str) -> Option<&Tagged> {
        self.tagged.iter().find(|tagged| tagged.record == name)
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
            .map(|callback| ctx.callback(callback.name, callback.ty))
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
            let plain: Vec<bool> = records
                .iter()
                .map(|record| {
                    let is_plain = |name: &str| records.iter().any(|other| other.name == name && other.plain);
                    record.fields.iter().all(|field| match &field.shape {
                        Shape::Scalar(_) | Shape::Text | Shape::Sink(_) => true,
                        Shape::Record(inner) | Shape::RecordPtr(inner) | Shape::Array(Elem::Record(inner)) => {
                            is_plain(inner)
                        }
                        Shape::Array(Elem::Scalar(_)) => true,
                        Shape::Vtable(inner) => records.iter().any(|other| other.name == *inner && other.vtable),
                        Shape::Callback(_) | Shape::Pointer(_) => false,
                    })
                })
                .collect();
            let mut changed = false;
            for (record, ok) in records.iter_mut().zip(plain) {
                if record.plain != ok {
                    record.plain = ok;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        resolve_receivers(&mut functions);
        let unscriptable: Vec<bool> = records
            .iter()
            .map(|record| {
                record
                    .fields
                    .iter()
                    .any(|field| matches!(&field.shape, Shape::Vtable(inner) if !vtable_scriptable(&callbacks, &records, inner)))
            })
            .collect();
        for (record, bad) in records.iter_mut().zip(unscriptable) {
            if bad {
                record.plain = false;
            }
        }
        let scriptable: Vec<bool> = functions
            .iter()
            .map(|function| {
                function.args.iter().all(|arg| match arg {
                    Arg::Raw { .. } => false,
                    Arg::Record { record, .. }
                    | Arg::Records { record, .. }
                    | Arg::OutRecord { record, .. }
                    | Arg::OutRecords { record, .. } => {
                        records.iter().any(|other| &other.name == record && other.plain)
                    }
                    Arg::Callback { callback, .. } => callback_scriptable(&callbacks, &records, callback),
                    _ => true,
                })
            })
            .collect();
        for (function, ok) in functions.iter_mut().zip(scriptable) {
            function.scriptable = ok;
        }
        let tagged = tagged_records(&groups, &records);
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
            tagged,
        })
    }
}

/// A callback a script can answer: every parameter is a number, text or a plain record.
fn callback_scriptable(callbacks: &[Callback], records: &[StructDef], name: &str) -> bool {
    callbacks.iter().any(|callback| {
        callback.name == name
            && callback.params.iter().all(|param| match &param.shape {
                Shape::Scalar(_) | Shape::Text => true,
                Shape::Record(record) => records.iter().any(|other| &other.name == record && other.plain),
                _ => false,
            })
    })
}

/// A vtable record a script can stand in for: every function it holds is a callback a script can answer.
fn vtable_scriptable(callbacks: &[Callback], records: &[StructDef], name: &str) -> bool {
    records.iter().any(|record| {
        record.name == name
            && record.vtable
            && record.fields.iter().all(|field| match &field.shape {
                Shape::Callback(callback) => callback_scriptable(callbacks, records, callback),
                _ => true,
            })
    })
}

/// A function whose receiver is not named like its object still has one when its first handle argument is the type the object's
/// other functions take as receiver (`collection_where(parent)` of `collection_len(collection)`).
fn resolve_receivers(functions: &mut [Function]) {
    let mut types: Vec<(String, Scalar)> = Vec::new();
    for function in functions.iter() {
        if let (Some(object), Some(at)) = (&function.object, function.receiver)
            && let Arg::Value { ty, .. } = &function.args[at]
            && !types.iter().any(|(name, _)| name == object)
        {
            types.push((object.clone(), ty.clone()));
        }
    }
    for function in functions.iter_mut() {
        let (None, Some(object)) = (function.receiver, function.object.as_ref()) else {
            continue;
        };
        let Some((_, ty)) = types.iter().find(|(name, _)| name == object) else {
            continue;
        };
        let first = function.args.iter().position(|arg| arg.is_input());
        if let Some(at) = first
            && matches!(&function.args[at], Arg::Value { ty: found, .. } if found == ty && matches!(found, Scalar::Handle(_)))
        {
            function.receiver = Some(at);
        }
    }
}

/// The records whose `kind` field a constant group names (`STD_PROP_*` for `GnoblinStdProp`), and which fields each kind
/// reads. The constants' docs say it: "`GnoblinStdProp.kind`: a number in `a`".
fn tagged_records(groups: &[ConstGroup], records: &[StructDef]) -> Vec<Tagged> {
    let mut tagged = Vec::new();
    for group in groups {
        // Every member's doc opens with the field it is a value of: `GnoblinStdProp.kind`.
        let owner = |member: &Const| -> Option<String> {
            let doc = member.doc.join(" ");
            let at = doc.find(".kind")?;
            let start = doc[..at].rfind('`')? + 1;
            Some(doc[start..at].to_owned())
        };
        let Some(first) = group.members.first().and_then(owner) else {
            continue;
        };
        if !group
            .members
            .iter()
            .all(|member| owner(member).as_deref() == Some(first.as_str()))
        {
            continue;
        }
        let Some(record) = records
            .iter()
            .find(|record| record.name == first && record.fields.iter().any(|field| field.name == "kind"))
        else {
            continue;
        };
        let variants = group
            .members
            .iter()
            .map(|member| {
                let doc = member.doc.join(" ");
                // The backticked words after the opening `Record.kind`.
                let rest = doc.find(".kind").map_or("", |at| &doc[at + 5..]);
                let mut fields: Vec<String> = Vec::new();
                for word in rest.split('`').skip(2).step_by(2) {
                    if word != "kind"
                        && record.fields.iter().any(|field| field.name == word)
                        && !fields.iter().any(|seen| seen == word)
                    {
                        fields.push(word.to_owned());
                    }
                }
                Variant {
                    short: member.short.clone(),
                    fields,
                    doc: member.doc.clone(),
                }
            })
            .collect();
        tagged.push(Tagged {
            record: record.name.clone(),
            type_name: group.type_name(),
            variants,
        });
    }
    tagged
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

    fn callback(&self, name: &str, ty: &str) -> Result<Callback, String> {
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
                let (shape, out) = match self.pointee(ty) {
                    Some((inner, mutable)) => match self.record_name(inner) {
                        Some(record) => (Shape::Record(record.to_owned()), mutable),
                        None => (self.shape(ty), false),
                    },
                    None => (self.shape(ty), false),
                };
                CallbackParam {
                    name: param.clone().unwrap_or_else(|| format!("arg{index}")),
                    shape,
                    out,
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
            params,
            ret,
        })
    }

    /// What a pointer points at when it is a list's element: a number, a handle or a record.
    fn elem(&self, ty: &Ty) -> Option<Elem> {
        if let Some(scalar) = self.scalar(ty) {
            return Some(Elem::Scalar(scalar));
        }
        self.record_name(ty).map(|record| Elem::Record(record.to_owned()))
    }

    fn record(
        &self,
        name: &str,
        doc: &str,
        fields: &[gnoblin_core_abi::reflect::FieldLayout],
    ) -> Result<StructDef, String> {
        let parsed = fields
            .iter()
            .map(|field| Ok((field, abi_c::parse(field.ty)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let mut out: Vec<FieldDef> = Vec::new();
        let mut index = 0;
        while index < parsed.len() {
            let (field, ty) = &parsed[index];
            let next = parsed.get(index + 1);
            let after = parsed.get(index + 2);
            let def = |name: &str, shape: Shape, doc: &str| FieldDef {
                name: name.to_owned(),
                shape,
                doc: doc_lines(doc),
                optional: false,
            };
            // `sink` and `emit`: the callee hands each result to the function.
            if field.name == "sink"
                && matches!(ty, Ty::Ptr { mutable: true, .. })
                && let Some((emit, Ty::Named(callback))) = next
                && self.callbacks.contains(callback)
            {
                out.push(def(emit.name, Shape::Sink(callback.clone()), emit.doc));
                index += 2;
                continue;
            }
            if let Ty::Ptr { mutable: false, to } = ty {
                // A table of functions and the object they act on: `vtable`, `user`, `drop_user`.
                if let Some(record) = self.record_name(to)
                    && let Some((user, Ty::Ptr { mutable: true, .. })) = next
                    && user.name == "user"
                    && let Some((drop_user, Ty::Option(_))) = after
                    && drop_user.name == "drop_user"
                {
                    out.push(def(field.name, Shape::Vtable(record.to_owned()), field.doc));
                    index += 3;
                    continue;
                }
                // A pointer and its count.
                if let Some(elem) = self.elem(to)
                    && let Some((count, Ty::Named(len))) = next
                    && len == "usize"
                    && ((field.name == "ptr" && count.name == "len") || count.name == format!("n_{}", field.name))
                {
                    let name = if field.name == "ptr" { "items" } else { field.name };
                    out.push(def(name, Shape::Array(elem), field.doc));
                    index += 2;
                    continue;
                }
                // A record the field may point at.
                if let Some(record) = self.record_name(to) {
                    out.push(def(field.name, Shape::RecordPtr(record.to_owned()), field.doc));
                    index += 1;
                    continue;
                }
            }
            let mut plain = def(field.name, self.shape(ty), field.doc);
            plain.optional = matches!(ty, Ty::Option(_));
            out.push(plain);
            index += 1;
        }
        let extensible = out
            .first()
            .is_some_and(|field| field.name == "size" && field.shape == Shape::Scalar(Scalar::Prim(Prim::U32)));
        let list_of = match out.as_slice() {
            [
                FieldDef {
                    shape: Shape::Array(elem),
                    ..
                },
            ] => Some(elem.clone()),
            _ => None,
        };
        let vtable = {
            let mut own = out
                .iter()
                .filter(|field| !(field.name == "reserved" || field.name == "size"));
            let first = own.next();
            first.is_some_and(|field| matches!(field.shape, Shape::Callback(_)))
                && own.all(|field| matches!(field.shape, Shape::Callback(_)))
        };
        Ok(StructDef {
            name: name.to_owned(),
            doc: doc_lines(doc),
            fields: out,
            extensible,
            plain: true,
            list_of,
            vtable,
        })
    }

    fn function(&self, name: &str, ty: &str, doc: &str) -> Result<Function, String> {
        let Ty::Fn(sig) = abi_c::parse(ty)? else {
            return Err(format!("table entry `{name}` is not a function type: {ty}"));
        };
        let mut args = self.args(&sig)?;
        // A record the library fills "with room for `count`" is a list of that many.
        for at in 0..args.len() {
            if let Arg::OutRecord { name: out, record } = &args[at]
                && let Some(from) = doc.find("room for `")
            {
                let count: String = doc[from + 10..].chars().take_while(|c| *c != '`').collect();
                if args
                    .iter()
                    .any(|arg| matches!(arg, Arg::Value { name, ty: Scalar::Prim(Prim::Usize) } if *name == count))
                {
                    args[at] = Arg::OutRecords {
                        name: out.clone(),
                        record: record.clone(),
                        count,
                    };
                }
            }
        }
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
        let flag = ret == Ret::Status && doc.contains("1 when");
        Ok(Function {
            name: name.to_owned(),
            doc: doc_lines(doc),
            args,
            ret,
            object,
            verb,
            receiver,
            scriptable: true,
            flag,
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
                    if matches!(pointee, Ty::Named(text) if text == "GnoblinStr") {
                        args.push(Arg::OutText { name });
                    } else if let Some(record) = self.record_name(pointee) {
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
        // The constants' docs name the field they are values of: a group of `.kind`, `.mode`, `.phase` or `.state` values is an
        // enumeration even when its numbers happen to be distinct bits (`STD_LAYOUT_FLOATING` 1 and `STD_LAYOUT_FOREIGN` 2).
        let enumerated = members.iter().all(|member| {
            let doc = member.doc.join(" ");
            [".kind`", ".mode`", ".phase`", ".state`"]
                .iter()
                .any(|field| doc.contains(field))
        });
        let flags = members.len() >= 2
            && !enumerated
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
        SdkSpec::load(&crate::workspace_root()).expect("the specification loads")
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

    #[test]
    fn the_three_record_shapes_and_the_tagged_values_are_named() {
        let spec = spec();
        let std = spec.library("std").expect("std is reflected");
        // A pointer and its length is a list, whole (`GnoblinStdMembers`) or as a field (`children`).
        let members = std.record("GnoblinStdMembers").expect("GnoblinStdMembers");
        assert!(matches!(members.list_of, Some(Elem::Scalar(_))), "{members:?}");
        let context = std.record("GnoblinStdLayoutContext").expect("GnoblinStdLayoutContext");
        assert!(
            context
                .fields
                .iter()
                .any(|field| field.name == "children" && matches!(field.shape, Shape::Array(Elem::Scalar(_))))
        );
        // `sink` and `emit` are one field.
        assert!(
            context
                .fields
                .iter()
                .any(|field| matches!(&field.shape, Shape::Sink(callback) if callback == "GnoblinStdEmitFn"))
        );
        assert!(context.fields.iter().all(|field| field.name != "sink"));
        // A vtable, its `user` and its `drop_user` are one field, and the vtable record is a table of functions.
        let layout = std.record("GnoblinStdLayoutSpec").expect("GnoblinStdLayoutSpec");
        assert!(
            layout
                .fields
                .iter()
                .any(|field| matches!(&field.shape, Shape::Vtable(record) if record == "GnoblinStdLayoutVtable"))
        );
        assert!(
            layout
                .fields
                .iter()
                .all(|field| field.name != "user" && field.name != "drop_user")
        );
        assert!(std.record("GnoblinStdLayoutVtable").is_some_and(|record| record.vtable));
        // Every record a script can build or read is plain.
        for name in [
            "GnoblinStdPlacement",
            "GnoblinStdLayoutContext",
            "GnoblinStdLayoutSpec",
            "GnoblinStdWorkspaceSpec",
        ] {
            assert!(std.record(name).is_some_and(|record| record.plain), "{name}");
        }
        // The tagged values say which fields each kind reads.
        let prop = std.tagged_record("GnoblinStdProp").expect("GnoblinStdProp is tagged");
        assert_eq!(prop.type_name, "PropKind");
        let triple = prop
            .variants
            .iter()
            .find(|variant| variant.short == "TRIPLE")
            .expect("TRIPLE");
        assert_eq!(triple.fields, ["a", "b", "c"]);
        let entry = std
            .tagged_record("GnoblinStdStyleEntry")
            .expect("GnoblinStdStyleEntry is tagged");
        let border = entry
            .variants
            .iter()
            .find(|variant| variant.short == "BORDER")
            .expect("BORDER");
        assert_eq!(border.fields, ["a", "b", "color", "inset"]);
        assert!(std.tagged_record("GnoblinStdOption").is_some());
    }

    #[test]
    fn the_transition_is_a_bracket_and_the_ring_is_a_list() {
        let spec = spec();
        let std = spec.library("std").expect("std is reflected");
        let brackets = std.brackets();
        let transition = brackets
            .iter()
            .find(|bracket| bracket.object == "transition")
            .expect("transition");
        assert_eq!(std.functions[transition.begin].name, "transition_begin");
        assert_eq!(std.functions[transition.commit].name, "transition_commit");
        assert_eq!(std.functions[transition.abort].name, "transition_abort");
        let ring = std
            .functions
            .iter()
            .find(|function| function.name == "layout_ring")
            .expect("layout_ring");
        assert!(
            ring.args
                .iter()
                .any(|arg| matches!(arg, Arg::OutRecords { count, .. } if count == "count"))
        );
        assert!(ring.scriptable);
        // Every function of the table is something a script can call.
        let unreachable: Vec<&str> = std
            .functions
            .iter()
            // `object_api` hands out the object protocol's own table: a runtime reads that table itself, not through the call layer.
            .filter(|function| !function.scriptable && function.name != "object_api")
            .map(|function| function.name.as_str())
            .collect();
        assert!(unreachable.is_empty(), "native only: {unreachable:?}");
    }
}

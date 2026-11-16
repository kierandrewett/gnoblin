//! The ABI described as data.
//!
//! Every type, constant, callback and export of the ABI is declared once, by one of the macros in `macros.rs`, and the
//! same declaration produces both the Rust item and an entry here. So the layout manifest (`abi.lock`) and the C header
//! are computed from the definitions the compiler used, offsets and sizes included, and cannot drift from them. Nothing
//! reads Rust source text.

/// One field of a `repr(C)` struct, as rustc laid it out.
#[derive(Clone, Copy, Debug)]
pub struct FieldLayout {
    /// The field name.
    pub name: &'static str,
    /// The field type as written in the source (`stringify!`; the consumers normalise whitespace).
    pub ty: &'static str,
    /// Byte offset from the start of the struct.
    pub offset: usize,
    /// Byte size of the field.
    pub size: usize,
    /// The field's doc comment, one `///` line per line.
    pub doc: &'static str,
}

/// One `repr(C)` struct.
#[derive(Clone, Copy, Debug)]
pub struct StructLayout {
    /// The struct name.
    pub name: &'static str,
    /// Total size in bytes.
    pub size: usize,
    /// Alignment in bytes.
    pub align: usize,
    /// The struct's doc comment.
    pub doc: &'static str,
    /// The fields, in declaration order.
    pub fields: &'static [FieldLayout],
}

impl StructLayout {
    /// True when the struct starts with `size: u32`, the caller-sets-the-size convention that lets fields be appended
    /// without breaking a caller built against an older layout. Only such a struct may grow within a major version.
    pub fn is_extensible(&self) -> bool {
        self.fields
            .first()
            .is_some_and(|field| field.name == "size" && field.ty == "u32")
    }
}

/// A type alias that becomes a C `typedef` (the handle types).
#[derive(Clone, Copy, Debug)]
pub struct AliasDef {
    /// The alias name.
    pub name: &'static str,
    /// The aliased type.
    pub ty: &'static str,
    /// The doc comment.
    pub doc: &'static str,
}

/// A function-pointer type that becomes a C `typedef`.
#[derive(Clone, Copy, Debug)]
pub struct CallbackDef {
    /// The typedef name.
    pub name: &'static str,
    /// The function-pointer type.
    pub ty: &'static str,
    /// The doc comment.
    pub doc: &'static str,
}

/// An integer constant that becomes a C `#define`.
#[derive(Clone, Copy, Debug)]
pub struct ConstDef {
    /// The constant name, without the `GNOBLIN_` prefix the header adds.
    pub name: &'static str,
    /// The Rust integer type.
    pub ty: &'static str,
    /// The value.
    pub value: i128,
    /// The doc comment.
    pub doc: &'static str,
}

/// A symbol the core library exports.
#[derive(Clone, Copy, Debug)]
pub struct ExportDef {
    /// The symbol name.
    pub symbol: &'static str,
    /// The function type, parameter names included.
    pub ty: &'static str,
    /// The doc comment.
    pub doc: &'static str,
}

/// Everything the ABI declares, for generators and the `abi.lock` check.
#[derive(Clone, Copy, Debug)]
pub struct Reflection {
    /// ABI major version.
    pub major: u32,
    /// ABI minor version.
    pub minor: u32,
    /// Library patch level (the `<patch>` of `libgnoblin-core.so.<major>.<minor>.<patch>`).
    pub patch: u32,
    /// The SONAME the core carries.
    pub soname: &'static str,
    /// The direct exports of the core library.
    pub exports: &'static [ExportDef],
    /// Integer constants: error codes, kinds, flags.
    pub consts: &'static [ConstDef],
    /// Handle typedefs.
    pub aliases: &'static [AliasDef],
    /// Callback typedefs.
    pub callbacks: &'static [CallbackDef],
    /// Struct layouts, in the order they must appear in a C header.
    pub structs: &'static [StructLayout],
}

/// The ABI of this build of the crate.
pub fn reflection() -> Reflection {
    Reflection {
        major: crate::ABI_MAJOR,
        minor: crate::ABI_MINOR,
        patch: crate::ABI_PATCH,
        soname: crate::SONAME,
        exports: crate::types::EXPORTS,
        consts: crate::types::CONSTS,
        aliases: crate::types::ALIASES,
        callbacks: crate::types::CALLBACKS,
        structs: crate::types::STRUCTS,
    }
}

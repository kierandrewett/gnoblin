//! The declaration macros. Each emits the Rust item and one `reflect` entry from the same tokens.
//!
//! A doc comment on an item is captured and carried into the entry so the generated C header keeps it. A field may carry
//! `#[cfg(...)]`, which gates both the field and its entry.
//!
//! The macros are exported (hidden from the docs) so that `gnoblin-std-abi`, the C table of `libgnoblin-std`, declares its
//! types with the same code and is described by the same `reflect` data, `abi.lock` check and C header generator.

/// Handle typedefs: `pub type GnoblinCtx = u64;`.
#[macro_export]
#[doc(hidden)]
macro_rules! abi_aliases {
    ($(
        $(#[doc = $doc:literal])*
        pub type $name:ident = $ty:ty;
    )*) => {
        $(
            $(#[doc = $doc])*
            pub type $name = $ty;
        )*
        /// The handle typedefs, in declaration order.
        pub const ALIASES: &[$crate::reflect::AliasDef] = &[
            $($crate::reflect::AliasDef {
                name: stringify!($name),
                ty: stringify!($ty),
                doc: concat!($($doc, "\n"),*),
            }),*
        ];
    };
}

/// Callback typedefs: `pub type GnoblinDropFn = unsafe extern "C" fn(user: *mut c_void);`.
#[macro_export]
#[doc(hidden)]
macro_rules! abi_callbacks {
    ($(
        $(#[doc = $doc:literal])*
        pub type $name:ident = $ty:ty;
    )*) => {
        $(
            $(#[doc = $doc])*
            pub type $name = $ty;
        )*
        /// The callback typedefs, in declaration order.
        pub const CALLBACKS: &[$crate::reflect::CallbackDef] = &[
            $($crate::reflect::CallbackDef {
                name: stringify!($name),
                ty: stringify!($ty),
                doc: concat!($($doc, "\n"),*),
            }),*
        ];
    };
}

/// Integer constants: `pub const E_OK: i32 = 0;`.
#[macro_export]
#[doc(hidden)]
macro_rules! abi_consts {
    ($(
        $(#[doc = $doc:literal])*
        pub const $name:ident : $ty:ident = $value:expr;
    )*) => {
        $(
            $(#[doc = $doc])*
            pub const $name: $ty = $value;
        )*
        /// The constants, in declaration order.
        pub const CONSTS: &[$crate::reflect::ConstDef] = &[
            $($crate::reflect::ConstDef {
                name: stringify!($name),
                ty: stringify!($ty),
                value: $value as i128,
                doc: concat!($($doc, "\n"),*),
            }),*
        ];
    };
}

/// `repr(C)` structs. Declaration order is the C header's order, so a struct used by value comes before its user.
#[macro_export]
#[doc(hidden)]
macro_rules! abi_structs {
    ($(
        $(#[doc = $sdoc:literal])*
        pub struct $name:ident {
            $(
                $(#[doc = $fdoc:literal])*
                $(#[cfg($fcfg:meta)])*
                pub $field:ident : $fty:ty
            ),* $(,)?
        }
    )*) => {
        $(
            $(#[doc = $sdoc])*
            #[repr(C)]
            #[derive(Clone, Copy, Debug)]
            #[allow(clippy::type_complexity)]
            pub struct $name {
                $(
                    $(#[doc = $fdoc])*
                    $(#[cfg($fcfg)])*
                    pub $field: $fty,
                )*
            }
        )*
        /// The struct layouts, in declaration order.
        pub const STRUCTS: &[$crate::reflect::StructLayout] = &[
            $($crate::reflect::StructLayout {
                name: stringify!($name),
                size: ::core::mem::size_of::<$name>(),
                align: ::core::mem::align_of::<$name>(),
                doc: concat!($($sdoc, "\n"),*),
                fields: &[
                    $(
                        $(#[cfg($fcfg)])*
                        $crate::reflect::FieldLayout {
                            name: stringify!($field),
                            ty: stringify!($fty),
                            offset: ::core::mem::offset_of!($name, $field),
                            size: ::core::mem::size_of::<$fty>(),
                            doc: concat!($($fdoc, "\n"),*),
                        },
                    )*
                ],
            }),*
        ];
    };
}

/// The direct exports of the core library. Each entry names a type alias for the function, so the implementation and
/// the SDK can assert at compile time that their definition has the declared signature.
#[macro_export]
#[doc(hidden)]
macro_rules! abi_exports {
    ($(
        $(#[doc = $doc:literal])*
        $alias:ident => $symbol:ident = $ty:ty;
    )*) => {
        $(
            $(#[doc = $doc])*
            pub type $alias = $ty;
        )*
        /// The direct exports, in declaration order.
        pub const EXPORTS: &[$crate::reflect::ExportDef] = &[
            $($crate::reflect::ExportDef {
                symbol: stringify!($symbol),
                ty: stringify!($ty),
                doc: concat!($($doc, "\n"),*),
            }),*
        ];
    };
}

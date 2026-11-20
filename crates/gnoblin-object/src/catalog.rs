//! A namespace's catalog as static data: what `gnoblin-sdkgen` writes into a library at build time from the schema files.
//!
//! A library serves its classes from a [`CatalogDef`]. Class ids are the position in [`CatalogDef::classes`] plus one, and a member's
//! id is its position in [`ClassDef::members`] plus one, so the tables are the ids' only index. The generator orders both by name
//! and puts a base class's members first, which is what keeps a base member's id the same in every class derived from it.

use gnoblin_core_abi as abi;

/// The type of a property, parameter, field or result.
#[derive(Debug)]
pub struct TypeDef {
    /// The value kind it crosses as (`VALUE_*`): a record is `VALUE_RECORD`, an enum `VALUE_ENUM`, a class `VALUE_OBJECT`, a callback
    /// `VALUE_CLOSURE`.
    pub kind: u32,
    /// The class of an object, record or enum, as the catalog names it (qualified by namespace when it is another namespace's);
    /// empty for any other type.
    pub class: &'static str,
    /// The element type of a list.
    pub elem: Option<&'static TypeDef>,
}

impl TypeDef {
    /// A type that names no class.
    pub const fn plain(kind: u32) -> TypeDef {
        TypeDef {
            kind,
            class: "",
            elem: None,
        }
    }
}

/// A method's parameter or a signal's field.
#[derive(Debug)]
pub struct ArgDef {
    /// The name.
    pub name: &'static str,
    /// The type.
    pub ty: &'static TypeDef,
    /// A call may leave it out (it is `VALUE_NONE`); only a trailing parameter is optional.
    pub optional: bool,
}

/// What a property reads as before anything wrote it, when the catalog says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultDef {
    /// The catalog gives none: the empty value of the property's type.
    None,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A number.
    Double(f64),
    /// Text.
    Text(&'static str),
    /// A member of an enum, by name.
    Enum(&'static str),
}

/// One member of a class.
#[derive(Debug)]
pub struct MemberDef {
    /// The name, `snake_case`.
    pub name: &'static str,
    /// `MEMBER_PROPERTY`, `MEMBER_METHOD`, `MEMBER_SIGNAL` or `MEMBER_VALUE`.
    pub kind: u32,
    /// `MEMBER_*` flags.
    pub flags: u32,
    /// A property's type, a method's result (`VALUE_NONE` for none); a signal has none.
    pub ty: &'static TypeDef,
    /// A method's parameters, a signal's fields.
    pub args: &'static [ArgDef],
    /// `args` as value kinds, for `catalog_each`.
    pub arg_kinds: &'static [u32],
    /// The names of `args`, separated by single spaces, for `catalog_each`.
    pub arg_names: &'static str,
    /// A property's default.
    pub default: DefaultDef,
    /// The existing typed table function that serves the member (`fn:<name>`), or empty.
    pub abi: &'static str,
}

impl MemberDef {
    /// How many arguments a call must give: those before the first optional one.
    pub fn required(&self) -> usize {
        self.args.iter().take_while(|arg| !arg.optional).count()
    }

    /// True for a function of the class (no receiver).
    pub fn is_static(&self) -> bool {
        self.flags & abi::MEMBER_STATIC != 0
    }
}

/// One class.
#[derive(Debug)]
pub struct ClassDef {
    /// The name, `snake_case`, unique in the namespace.
    pub name: &'static str,
    /// `CLASS_HANDLE`, `CLASS_RECORD`, `CLASS_INTERFACE`, `CLASS_SERVICE` or `CLASS_ENUM`.
    pub kind: u32,
    /// `CLASS_FLAG_*` flags.
    pub flags: u32,
    /// The base class's name, or empty. A name with a dot is another namespace's class.
    pub base: &'static str,
    /// How many of the first members belong to a base class of another namespace: that library serves them, with the ids they have
    /// there, and an object of this class forwards them to its base object. Zero when the base is in this namespace or there is none.
    pub foreign_members: usize,
    /// The class's members, a base class's first.
    pub members: &'static [MemberDef],
}

/// A namespace's catalog.
#[derive(Debug)]
pub struct CatalogDef {
    /// The namespace (`gnoblin.std`).
    pub namespace: &'static str,
    /// The catalog's hash (`Catalog::namespace_fingerprint`).
    pub hash: u64,
    /// The classes, ordered by name.
    pub classes: &'static [ClassDef],
}

impl CatalogDef {
    /// The id of the class `name`.
    pub fn class_id(&self, name: &str) -> Option<u32> {
        self.classes
            .iter()
            .position(|class| class.name == name)
            .map(|position| position as u32 + 1)
    }

    /// The class with id `id`.
    pub fn class(&self, id: u32) -> Option<&'static ClassDef> {
        let position = usize::try_from(id.checked_sub(1)?).ok()?;
        self.classes.get(position)
    }

    /// True when class `id` is `ancestor` or derives from it, by the bases the catalog names.
    pub fn derives(&self, id: u32, ancestor: u32) -> bool {
        let mut current = id;
        // A chain is as long as the classes are many; the bound stops a catalog that loops.
        for _ in 0..=self.classes.len() {
            if current == ancestor {
                return true;
            }
            let Some(class) = self.class(current) else {
                return false;
            };
            match self.class_id(class.base) {
                Some(base) => current = base,
                None => return false,
            }
        }
        false
    }
}

impl ClassDef {
    /// The id of the member `name` of `kind`.
    pub fn member_id(&self, kind: u32, name: &str) -> Option<u32> {
        self.members
            .iter()
            .position(|member| member.kind == kind && member.name == name)
            .map(|position| position as u32 + 1)
    }

    /// The member with id `id`.
    pub fn member(&self, id: u32) -> Option<&'static MemberDef> {
        let position = usize::try_from(id.checked_sub(1)?).ok()?;
        self.members.get(position)
    }

    /// The properties, in order: the fields of a record.
    pub fn properties(&self) -> impl Iterator<Item = &'static MemberDef> {
        self.members.iter().filter(|member| member.kind == abi::MEMBER_PROPERTY)
    }
}

impl ClassDef {
    /// The namespace and the name of the base class when it is another namespace's.
    pub fn foreign_base(&self) -> Option<(&'static str, &'static str)> {
        self.base.rsplit_once('.')
    }
}

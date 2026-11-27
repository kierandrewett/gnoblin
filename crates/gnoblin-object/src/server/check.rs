//! The checks the catalog implies: a value an implementation receives has the type its member declared.
//!
//! A script's number is a double and a Rust caller's is an integer, so a whole number is accepted for an integer type and an integer
//! for a number. Everything else must match, and the answer is `E_TYPE` naming the member and the two kinds.

use gnoblin_core_abi as abi;

use super::Library;
use super::object::CtxState;
use crate::catalog::{MemberDef, TypeDef};
use crate::error::{Fail, Res};
use crate::value::Val;

/// What a type is called in a message.
fn describe(ty: &TypeDef) -> String {
    match ty.kind {
        abi::VALUE_BOOL => "a boolean".to_owned(),
        abi::VALUE_INT => "an integer".to_owned(),
        abi::VALUE_UINT => "an unsigned integer".to_owned(),
        abi::VALUE_DOUBLE => "a number".to_owned(),
        abi::VALUE_TEXT => "text".to_owned(),
        abi::VALUE_SECRET => "secret text".to_owned(),
        abi::VALUE_BYTES => "bytes".to_owned(),
        abi::VALUE_NODE => "a node".to_owned(),
        abi::VALUE_LIST => match ty.elem {
            Some(elem) => format!("a list of {}", describe(elem)),
            None => "a list".to_owned(),
        },
        abi::VALUE_RECORD => format!("a `{}` record", ty.class),
        abi::VALUE_OBJECT => format!("a `{}` object", ty.class),
        abi::VALUE_ENUM => format!("a `{}` value", ty.class),
        abi::VALUE_ANY => "none, a boolean, a number, text or a list of those".to_owned(),
        abi::VALUE_CLOSURE => "a closure".to_owned(),
        _ => "a value".to_owned(),
    }
}

impl Library {
    /// `value` as the type `ty` asks for, or `E_TYPE` naming `what`.
    pub(crate) fn coerce(&self, st: &CtxState, ty: &TypeDef, value: Val, what: &str) -> Res<Val> {
        let wrong =
            |value: &Val| Fail::type_error(format!("{what} must be {}, got {}", describe(ty), value.kind_name()));
        match ty.kind {
            abi::VALUE_BOOL => match value {
                Val::Bool(_) => Ok(value),
                other => Err(wrong(&other)),
            },
            abi::VALUE_INT => value.int().map(Val::Int).map_err(|_| wrong(&value)),
            abi::VALUE_UINT => value.uint().map(Val::Uint).map_err(|_| wrong(&value)),
            abi::VALUE_DOUBLE => value.double().map(Val::Double).map_err(|_| wrong(&value)),
            abi::VALUE_TEXT => match value {
                Val::Text(_) => Ok(value),
                other => Err(wrong(&other)),
            },
            abi::VALUE_SECRET => match value {
                Val::Text(text) | Val::Secret(text) => Ok(Val::Secret(text)),
                other => Err(wrong(&other)),
            },
            abi::VALUE_BYTES => match value {
                Val::Bytes(_) => Ok(value),
                other => Err(wrong(&other)),
            },
            abi::VALUE_NODE => match value {
                Val::Node(_) => Ok(value),
                Val::Uint(id) => Ok(Val::Node(id)),
                other => Err(wrong(&other)),
            },
            abi::VALUE_CLOSURE => match value {
                Val::Closure(_) => Ok(value),
                other => Err(wrong(&other)),
            },
            abi::VALUE_LIST => match value {
                Val::List(items) => {
                    let Some(elem) = ty.elem else {
                        return Ok(Val::List(items));
                    };
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, item)| self.coerce(st, elem, item, &format!("{what}[{index}]")))
                        .collect::<Res<Vec<_>>>()
                        .map(Val::List)
                }
                other => Err(wrong(&other)),
            },
            abi::VALUE_RECORD => self.coerce_record(st, ty, value, what),
            abi::VALUE_OBJECT => self.coerce_object(st, ty, value, what),
            abi::VALUE_ENUM => self.coerce_enum(ty, value, what),
            abi::VALUE_ANY if value.is_plain() => Ok(value),
            abi::VALUE_ANY => Err(wrong(&value)),
            _ => Ok(value),
        }
    }

    fn coerce_record(&self, st: &CtxState, ty: &TypeDef, value: Val, what: &str) -> Res<Val> {
        let (class, fields) = match value {
            Val::Record { class, fields } => (class, fields),
            other => {
                return Err(Fail::type_error(format!(
                    "{what} must be {}, got {}",
                    describe(ty),
                    other.kind_name()
                )));
            }
        };
        // A record of another namespace's catalog is checked by the library that serves it.
        let Some(id) = self.catalog.class_id(ty.class) else {
            return Ok(Val::Record { class, fields });
        };
        if class != 0 && class != id {
            return Err(Fail::type_error(format!("{what} must be a `{}` record", ty.class)));
        }
        let def = self
            .catalog
            .class(id)
            .ok_or_else(|| Fail::err(format!("the record class `{}` is not in the catalog", ty.class)))?;
        let properties: Vec<&MemberDef> = def.properties().collect();
        if fields.len() > properties.len() {
            return Err(Fail::type_error(format!(
                "{what}: a `{}` record has {} fields, got {}",
                ty.class,
                properties.len(),
                fields.len()
            )));
        }
        let mut checked = Vec::with_capacity(properties.len());
        let mut given = fields.into_iter();
        for property in properties {
            let field = given.next().unwrap_or(Val::None);
            checked.push(if field.is_none() {
                field
            } else {
                self.coerce(st, property.ty, field, &format!("{what}.{}", property.name))?
            });
        }
        Ok(Val::Record {
            class: id,
            fields: checked,
        })
    }

    fn coerce_object(&self, st: &CtxState, ty: &TypeDef, value: Val, what: &str) -> Res<Val> {
        let (class, obj) = match value {
            Val::Object { class, obj } => (class, obj),
            other => {
                return Err(Fail::type_error(format!(
                    "{what} must be {}, got {}",
                    describe(ty),
                    other.kind_name()
                )));
            }
        };
        let Some(wanted) = self.catalog.class_id(ty.class) else {
            // An object of another namespace: the library that serves it knows its handles.
            return Ok(Val::Object { class, obj });
        };
        let actual = st
            .objects
            .borrow()
            .get(obj)
            .map(|entry| entry.class)
            .ok_or_else(|| Fail::stale(format!("{what} names an object that is gone")))?;
        if !self.catalog.derives(actual, wanted) {
            return Err(Fail::type_error(format!(
                "{what} must be {}, got an object of another class",
                describe(ty)
            )));
        }
        Ok(Val::Object { class: actual, obj })
    }

    fn coerce_enum(&self, ty: &TypeDef, value: Val, what: &str) -> Res<Val> {
        let (class, member) = match value {
            Val::Enum { class, value } => (class, value),
            other => {
                return Err(Fail::type_error(format!(
                    "{what} must be {}, got {}",
                    describe(ty),
                    other.kind_name()
                )));
            }
        };
        let Some(id) = self.catalog.class_id(ty.class) else {
            return Ok(Val::Enum { class, value: member });
        };
        if class != 0 && class != id {
            return Err(Fail::type_error(format!("{what} must be a `{}` value", ty.class)));
        }
        let count = self.catalog.class(id).map_or(0, |def| {
            def.members.iter().filter(|m| m.kind == abi::MEMBER_VALUE).count()
        });
        if member == 0 || member as usize > count {
            return Err(Fail::type_error(format!("{what} is not a value of `{}`", ty.class)));
        }
        Ok(Val::Enum {
            class: id,
            value: member,
        })
    }

    /// The arguments of a call to `member`, checked: no more than it takes, none of the required ones missing, each of its type, and
    /// the optional ones left out padded with [`Val::None`].
    pub(crate) fn check_args(&self, st: &CtxState, class: &str, member: &MemberDef, args: Vec<Val>) -> Res<Vec<Val>> {
        let name = format!("`{class}.{}`", member.name);
        if args.len() > member.args.len() {
            return Err(Fail::type_error(format!(
                "{name} takes {} argument(s), got {}",
                member.args.len(),
                args.len()
            )));
        }
        if args.len() < member.required() {
            return Err(Fail::type_error(format!(
                "{name} needs {} argument(s), got {}",
                member.required(),
                args.len()
            )));
        }
        let mut checked = Vec::with_capacity(member.args.len());
        let mut given = args.into_iter();
        for arg in member.args {
            let value = given.next().unwrap_or(Val::None);
            if value.is_none() {
                if !arg.optional {
                    return Err(Fail::type_error(format!("{name}: `{}` is required", arg.name)));
                }
                checked.push(value);
            } else {
                checked.push(self.coerce(st, arg.ty, value, &format!("{name}: `{}`", arg.name))?);
            }
        }
        Ok(checked)
    }
}

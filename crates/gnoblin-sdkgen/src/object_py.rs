//! The Python classes of a namespace served through the object protocol: `gnoblin/objects/<module>.py` and its `.pyi`.
//!
//! The classes call the built-in module `_gnoblin_objects` (`gnoblin-runtime-python`'s `object_native.rs`) through the small runtime in
//! `_base.py`. A value crosses as Python's own: a record is a `dataclass` (the native layer sees it as a `dict` of its fields, in the
//! catalog's order, and hands one back as a `tuple`), an enum is an `IntEnum`, a handle is an instance of a generated class and a closure is
//! a callable. One renderer writes the module and its stub: the stub is the same text with every body `...`.

use std::collections::BTreeSet;

use gnoblin_schema::{CallbackDecl, DefaultValue, EnumDecl, Field, Method, ObjectKind, ObjectType, Property, Type};

use super::object_script::{ScriptView, quote};
use super::objects::{ClassMember, Member, check_foreign, slots_of};
use super::text::{CodeWriter, pascal, python_ident, snake};

/// `gnoblin/objects/__init__.py`: the names a script imports.
pub(super) const INIT_PY: &str = r#""""The classes of the libraries' object protocol.

`gnoblin.objects.std` has the classes of `gnoblin.std` and `gnoblin.objects.model` those of `gnoblin.model`, written from the catalog the
libraries serve. This package holds what they share.
"""

from ._base import Listener, Timing

__all__ = ["Listener", "Timing"]
"#;

/// `gnoblin/objects/__init__.pyi`.
pub(super) const INIT_PYI: &str = r#"from dataclasses import dataclass, field

class Listener:
    """A listener of an object's signal or property. It lasts until it is stopped or the generation ends."""

    def stop(self) -> None:
        """Stops hearing. A second call does nothing."""
        ...

@dataclass
class Timing:
    """How a property is written over a time."""

    seconds: float
    delay: float = 0.0
    tween: int = 0
    samples: list[float] = field(default_factory=list)

__all__ = ["Listener", "Timing"]
"#;

/// `gnoblin/objects/_base.py`: the runtime the generated classes share.
pub(super) const BASE_PY: &str = r#""""The runtime the generated object classes share: the native module, the handle of a library object and the checks of values that go in."""

from __future__ import annotations

import dataclasses
import enum
from typing import Any, Callable

import _gnoblin_objects as native

_classes: dict[tuple[str, str], type] = {}


def open(namespace: str) -> Any:
    """The namespace the library serves. Raises ImportError when the library is not installed."""
    return native.open(namespace)


def ids(namespace: Any, name: str, names: list[str]) -> Callable[[], list[int]]:
    """The ids of the class `name` and its members `names` (`p:text`, `m:insert`, `s:changed`), asked of the library when first used."""
    found: list[int] | None = None

    def lookup() -> list[int]:
        nonlocal found
        if found is None:
            found = namespace.ids(name, names)
        return found

    return lookup


def same(value: Any) -> Any:
    return value


def opt(value: Any, convert: Callable[[Any], Any]) -> Any:
    return None if value is None else convert(value)


def need(value: Any, what: str) -> Any:
    if value is None:
        raise TypeError(f"{what} is required")
    return value


def enum_(cls: type[enum.IntEnum], value: Any) -> Any:
    """`value` as a member of `cls`: a member, or the name the catalog gives it (`"follow-mouse"`)."""
    if isinstance(value, cls):
        return value
    if isinstance(value, str):
        member = cls.__members__.get(value.upper().replace("-", "_"))
        if member is not None:
            return member
    names = ", ".join(member.name.lower() for member in cls)
    raise TypeError(f"{value!r} is not a value of {cls.__name__} ({names})")


class Listener:
    """A listener of an object's signal or property. It lasts until it is stopped or the generation ends, not until it is collected."""

    __slots__ = ("_namespace", "_registration")

    def __init__(self, namespace: Any, registration: int) -> None:
        self._namespace = namespace
        self._registration = registration

    def stop(self) -> None:
        """Stops hearing. A second call does nothing."""
        registration, self._registration = self._registration, 0
        if registration:
            self._namespace.unlisten(registration)


@dataclasses.dataclass
class Timing:
    """How a property is written over a time."""

    seconds: float
    delay: float = 0.0
    tween: int = 0
    samples: list[float] = dataclasses.field(default_factory=list)


def timing(value: Timing) -> tuple[float, float, int, list[float]]:
    if not isinstance(value, Timing):
        raise TypeError(f"expected a Timing, got {type(value).__name__}")
    return (float(value.seconds), float(value.delay), int(value.tween), list(value.samples))


def register(namespace: str, name: str, cls: type) -> None:
    """Remembers the class a library object of the name `name` is read as."""
    _classes[(namespace, name)] = cls


def wrap(namespace: str, handle: Any, fallback: type) -> Any:
    """An instance of the class the library says `handle` is, or of `fallback` when the script has no such class."""
    cls = _classes.get((namespace, handle.class_name), fallback)
    return cls._wrap(handle)


class _Held:
    """What an object the library made and a script object the library calls through have in common: the handle the library knows it by."""

    __slots__ = ()

    @classmethod
    def _wrap(cls, handle: Any) -> Any:
        instance = object.__new__(cls)
        instance._h = handle
        return instance


class Implementor(_Held):
    """The base of the generated interface classes: an object the library calls through, once it has been given to a call."""

    _h: Any = None


def ref(value: Any, implement: Callable[[Any], Any] | None = None) -> Any:
    """An object on its way in: a library object, or a script object that implements an interface (`implement` makes the library's side)."""
    handle = getattr(value, "_h", None)
    if handle is None:
        if implement is None or not isinstance(value, Implementor):
            raise TypeError(f"expected an object the library made, got {type(value).__name__}")
        handle = implement(value)
        value._h = handle
    return handle


def callback(fn: Callable[..., Any], decoders: tuple[Callable[[Any], Any], ...], encode: Callable[[Any], Any]) -> Callable[[int, list[Any]], Any]:
    """A function the library calls: `decoders` read its arguments and `encode` writes what it answers."""
    if not callable(fn):
        raise TypeError(f"expected a callable, got {type(fn).__name__}")

    def run(member: int, args: list[Any]) -> Any:
        return encode(fn(*[decode(arg) for decode, arg in zip(decoders, args)]))

    return run


class Object(_Held):
    """A library object a script holds. It is released when the script's last reference goes."""

    __slots__ = ("_h", "__weakref__")

    _IDS: Callable[[], list[int]]
    _EVENTS: dict[str, tuple[int, tuple[Callable[[Any], Any], ...]]]
    _NS: Any

    def __init__(self, handle: Any) -> None:
        self._h = handle

    def destroy(self) -> None:
        """Ends the object and its children, whoever else holds it."""
        self._h.destroy()

    def on(self, event: str, handler: Callable[..., Any] | None = None) -> Any:
        """Calls `handler` each time `event` happens, until the returned listener is stopped. Without a handler it is a decorator."""
        spec = self._EVENTS.get(event)
        if spec is None:
            raise TypeError(f"no event {event!r}; the events are {', '.join(self._EVENTS)}")
        at, decoders = spec

        def attach(fn: Callable[..., Any]) -> Listener:
            if not callable(fn):
                raise TypeError(f"expected a callable, got {type(fn).__name__}")

            def run(member: int, args: list[Any]) -> None:
                fn(*[decode(arg) for decode, arg in zip(decoders, args)])

            return Listener(self._NS, self._h.listen(self._IDS()[at], run))

        if handler is not None:
            return attach(handler)

        def decorator(fn: Callable[..., Any]) -> Callable[..., Any]:
            attach(fn)
            return fn

        return decorator

    def __repr__(self) -> str:
        return f"<{type(self).__name__} {self._h!r}>"
"#;

fn class_name(object: &ObjectType) -> String {
    object.export.py.clone().unwrap_or_else(|| pascal(&object.name))
}

fn member_name(name: &str, export: &Option<String>) -> String {
    python_ident(&export.clone().unwrap_or_else(|| snake(name)))
}

fn param_name(field: &Field) -> String {
    python_ident(&snake(&field.name))
}

/// The name of the member of an enum: the value in capitals.
fn enum_member(value: &str) -> String {
    let name = value.replace('-', "_").to_ascii_uppercase();
    if name.starts_with(|ch: char| ch.is_ascii_digit()) {
        format!("V_{name}")
    } else {
        name
    }
}

/// Whether a record's fields force keyword-only construction: a field that must be given follows one that has a default.
fn keyword_only(object: &ObjectType) -> bool {
    let mut defaulted = false;
    for property in &object.properties {
        if property.default.is_some() || property.optional {
            defaulted = true;
        } else if defaulted {
            return true;
        }
    }
    false
}

/// An event of a class: a signal, or a property's change.
struct ScriptEvent {
    /// Whether the catalog declares it as a signal (the change of a property is the other kind).
    signal: bool,
    name: String,
    at: usize,
    decoders: Vec<String>,
    params: Vec<(String, String)>,
}

/// A member of a class's lookup list, as the runtime is asked for it.
struct MemberLookup {
    kind: char,
    name: String,
}

struct Py<'a> {
    view: &'a ScriptView<'a>,
    stub: bool,
}

impl<'a> Py<'a> {
    /// A Python expression for a schema default of the type `ty`.
    fn default_expr(&self, ty: &Type, default: &DefaultValue) -> Result<String, String> {
        Ok(match (default, ty) {
            (DefaultValue::Bool(value), _) => (if *value { "True" } else { "False" }).to_owned(),
            (DefaultValue::Int(value), Type::Double) => format!("{value}.0"),
            (DefaultValue::Int(value), _) => value.to_string(),
            (DefaultValue::Uint(value), Type::Double) => format!("{value}.0"),
            (DefaultValue::Uint(value), _) => value.to_string(),
            (DefaultValue::Double(value), _) => format!("{value:?}"),
            (DefaultValue::Text(value), Type::Enum(name)) => {
                format!("{}.{}", pascal(&self.view.enum_decl(name)?.name), enum_member(value))
            }
            (DefaultValue::Text(value), _) => quote(value),
            (DefaultValue::List(items), Type::List(inner)) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| self.default_expr(inner, item))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            (other, ty) => return Err(format!("the default {other:?} does not fit the type `{ty}`")),
        })
    }

    /// The Python type of `ty`. `input` is the form a parameter takes: an enum may be given by its name.
    fn ty(&self, ty: &Type, input: bool) -> Result<String, String> {
        Ok(match ty {
            Type::Bool => "bool".to_owned(),
            Type::Int | Type::Uint => "int".to_owned(),
            Type::Double => "float".to_owned(),
            Type::Text => "str".to_owned(),
            Type::Bytes => "bytes".to_owned(),
            Type::Enum(name) => {
                let class = pascal(&self.view.enum_decl(name)?.name);
                if input { format!("{class} | str") } else { class }
            }
            Type::ObjectRef(name) => class_name(self.view.class(name)?),
            Type::List(inner) => format!("list[{}]", self.ty(inner, input)?),
            Type::Callback(name) => {
                let declared = self.view.callback(name)?;
                let params: Vec<String> = declared
                    .params
                    .iter()
                    .map(|field| self.ty(&field.ty, false))
                    .collect::<Result<_, _>>()?;
                let returns = match &declared.returns {
                    Some(ty) => self.ty(ty, true)?,
                    None => "None".to_owned(),
                };
                format!("Callable[[{}], {returns}]", params.join(", "))
            }
        })
    }

    /// The type a property or a result is read as: an object or a record is absent only where the catalog marks it `optional`.
    fn ty_read(&self, ty: &Type, optional: bool) -> Result<String, String> {
        let text = self.ty(ty, false)?;
        Ok(if optional && matches!(ty, Type::ObjectRef(_)) {
            format!("{text} | None")
        } else {
            text
        })
    }

    fn enc(&self, ty: &Type, e: &str) -> Result<String, String> {
        Ok(match ty {
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes => e.to_owned(),
            Type::Enum(name) => format!("_enc_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                match class.kind {
                    ObjectKind::Record => format!("_enc_rec_{}({e})", snake(&class.name)),
                    ObjectKind::Interface => format!("_B.ref({e}, _implement_{})", snake(&class.name)),
                    _ => format!("_B.ref({e})"),
                }
            }
            Type::List(inner) => {
                let item = self.enc(inner, "item")?;
                if item == "item" {
                    e.to_owned()
                } else {
                    format!("[{item} for item in {e}]")
                }
            }
            Type::Callback(name) => format!("_cb_{}({e})", snake(&self.view.callback(name)?.name)),
        })
    }

    /// What the library sent as `e`, as the script value. `absent` says an object or a record may be none (a property's value is, a
    /// call's answer is not).
    fn dec(&self, ty: &Type, e: &str, absent: bool) -> Result<String, String> {
        Ok(match ty {
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes => e.to_owned(),
            Type::Enum(name) => format!("_dec_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                let (function, call) = match class.kind {
                    ObjectKind::Record => (
                        format!("_dec_rec_{}", snake(&class.name)),
                        format!("_dec_rec_{}({e})", snake(&class.name)),
                    ),
                    _ => (
                        format!("lambda handle: _B.wrap(_NS_NAME, handle, {})", class_name(class)),
                        format!("_B.wrap(_NS_NAME, {e}, {})", class_name(class)),
                    ),
                };
                if absent {
                    format!("_B.opt({e}, {function})")
                } else {
                    call
                }
            }
            Type::List(inner) => {
                let item = self.dec(inner, "item", false)?;
                if item == "item" {
                    e.to_owned()
                } else {
                    format!("[{item} for item in {e}]")
                }
            }
            Type::Callback(name) => {
                return Err(format!(
                    "`callback:{name}` can be given to the library, not read back from it"
                ));
            }
        })
    }

    fn decoder(&self, ty: &Type, absent: bool) -> Result<String, String> {
        let body = self.dec(ty, "value", absent)?;
        Ok(if body == "value" {
            "_B.same".to_owned()
        } else {
            format!("lambda value: {body}")
        })
    }

    /// `header` and its body: one `...` line in a stub, the body indented in a module.
    fn def(&self, c: &mut CodeWriter, header: &str, doc: &str, body: &[String]) {
        if self.stub {
            c.line(format!("{header}: ..."));
        } else {
            c.line(format!("{header}:"));
            c.indent();
            c.line(format!("\"\"\"{doc}\"\"\""));
            for line in body {
                c.line(line);
            }
            c.dedent();
        }
    }

    fn enum_source(&self, c: &mut CodeWriter, declared: &EnumDecl) {
        let name = pascal(&declared.name);
        c.line(format!("class {name}(enum.IntEnum):"));
        c.indent();
        c.line(format!(
            "\"\"\"The enum `{}`: {}.\"\"\"",
            declared.name,
            declared
                .values
                .iter()
                .map(|value| format!("`{value}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (index, value) in declared.values.iter().enumerate() {
            c.line(format!("{} = {}", enum_member(value), index + 1));
        }
        c.dedent();
        c.blank();
        if !self.stub {
            let helper = snake(&declared.name);
            c.line(format!("def _enc_enum_{helper}(value):"));
            c.line(format!("    return _B.enum_({name}, value)"));
            c.blank();
            c.line(format!("def _dec_enum_{helper}(value):"));
            c.line(format!("    return {name}(value)"));
            c.blank();
        }
    }

    fn callback_source(&self, c: &mut CodeWriter, declared: &CallbackDecl) -> Result<(), String> {
        if self.stub {
            return Ok(());
        }
        let decoders: Vec<String> = declared
            .params
            .iter()
            .map(|field| self.decoder(&field.ty, false))
            .collect::<Result<_, _>>()?;
        let encode = match &declared.returns {
            Some(ty) => format!("lambda value: {}", self.enc(ty, "value")?),
            None => "_B.same".to_owned(),
        };
        c.line(format!("def _cb_{}(fn):", snake(&declared.name)));
        c.line(format!(
            "    return _B.callback(fn, ({}), {encode})",
            decoders.iter().map(|d| format!("{d},")).collect::<Vec<_>>().join(" ")
        ));
        c.blank();
        Ok(())
    }

    fn record_source(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        if !object.methods.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the record `{}` declares members that are not properties",
                object.name
            ));
        }
        let name = class_name(object);
        c.line(if keyword_only(object) {
            "@dataclasses.dataclass(kw_only=True)"
        } else {
            "@dataclasses.dataclass"
        });
        c.line(format!("class {name}:"));
        c.indent();
        c.line(format!("\"\"\"The record `{}`.\"\"\"", object.name));
        for property in &object.properties {
            let field = member_name(&property.name, &property.extras.export.py);
            let ty = self.ty(&property.ty, false)?;
            match (&property.default, property.optional) {
                (Some(default), _) => {
                    let value = self.default_expr(&property.ty, default)?;
                    if self.stub {
                        c.line(format!("{field}: {ty} = ..."));
                    } else if matches!(property.ty, Type::List(_)) {
                        c.line(format!(
                            "{field}: {ty} = dataclasses.field(default_factory=lambda: {value})"
                        ));
                    } else {
                        c.line(format!("{field}: {ty} = {value}"));
                    }
                }
                (None, true) => c.line(format!(
                    "{field}: {ty} | None = {}",
                    if self.stub { "..." } else { "None" }
                )),
                (None, false) => c.line(format!("{field}: {ty}")),
            }
        }
        c.dedent();
        c.blank();
        if self.stub {
            return Ok(());
        }
        let helper = snake(&object.name);
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        for (index, property) in object.properties.iter().enumerate() {
            let field = member_name(&property.name, &property.extras.export.py);
            let given = format!("record.{field}");
            let value = if property.optional {
                format!("_B.opt({given}, lambda value: {})", self.enc(&property.ty, "value")?)
            } else if property.default.is_some() {
                self.enc(&property.ty, &given)?
            } else {
                self.enc(
                    &property.ty,
                    &format!("_B.need({given}, {})", quote(&format!("{name}.{field}"))),
                )?
            };
            writes.push(format!("{}: {value}", quote(&property.name)));
            let read = format!("fields[{index}]");
            let decoded = if property.optional {
                format!(
                    "_B.opt({read}, lambda value: {})",
                    self.dec(&property.ty, "value", false)?
                )
            } else {
                self.dec(&property.ty, &read, false)?
            };
            reads.push(format!("{field}={decoded}"));
        }
        c.line(format!("def _enc_rec_{helper}(record):"));
        c.line(format!("    if not isinstance(record, {name}):"));
        c.line(format!(
            "        raise TypeError(f\"expected a {name}, got {{type(record).__name__}}\")"
        ));
        c.line(format!("    return {{{}}}", writes.join(", ")));
        c.blank();
        c.line(format!("def _dec_rec_{helper}(fields):"));
        c.line(format!("    return {name}({})", reads.join(", ")));
        c.blank();
        Ok(())
    }

    fn lookups(slots: &[ClassMember<'_>]) -> Vec<MemberLookup> {
        slots
            .iter()
            .map(|slot| match &slot.member {
                Member::Property(property) => MemberLookup {
                    kind: 'p',
                    name: property.name.clone(),
                },
                Member::Method(method) => MemberLookup {
                    kind: 'm',
                    name: method.name.clone(),
                },
                Member::Signal(signal) => MemberLookup {
                    kind: 's',
                    name: signal.name.clone(),
                },
            })
            .collect()
    }

    fn at(lookups: &[MemberLookup], kind: char, name: &str) -> usize {
        lookups
            .iter()
            .position(|lookup| lookup.kind == kind && lookup.name == name)
            .map_or(0, |index| index + 1)
    }

    fn ids_source(&self, c: &mut CodeWriter, object: &ObjectType, lookups: &[MemberLookup]) {
        let names: Vec<String> = lookups
            .iter()
            .map(|lookup| quote(&format!("{}:{}", lookup.kind, lookup.name)))
            .collect();
        c.line(format!(
            "_ids_{} = _B.ids(_NS, {}, [{}])",
            snake(&object.name),
            quote(&object.name),
            names.join(", ")
        ));
    }

    fn params(&self, fields: &[Field]) -> Result<(String, Vec<String>), String> {
        let mut params = Vec::new();
        let mut args = Vec::new();
        for field in fields {
            let name = param_name(field);
            let ty = self.ty(&field.ty, true)?;
            if field.optional {
                params.push(format!("{name}: {ty} | None = None"));
                args.push(format!(
                    "_B.opt({name}, lambda value: {})",
                    self.enc(&field.ty, "value")?
                ));
            } else {
                params.push(format!("{name}: {ty}"));
                args.push(self.enc(&field.ty, &name)?);
            }
        }
        Ok((params.join(", "), args))
    }

    fn handle_source(&self, c: &mut CodeWriter, object: &'a ObjectType) -> Result<(), String> {
        let name = class_name(object);
        let snake_name = snake(&object.name);
        let slots = slots_of(self.view.catalog, &self.view.objects, object);
        for slot in &slots {
            check_foreign(&object.name, slot)?;
        }
        let lookups = Self::lookups(&slots);
        if !self.stub {
            self.ids_source(c, object, &lookups);
        }
        let mut events: Vec<ScriptEvent> = Vec::new();
        for slot in &slots {
            match &slot.member {
                Member::Signal(signal) => {
                    let mut decoders = Vec::new();
                    let mut params = Vec::new();
                    for field in &signal.fields {
                        decoders.push(self.decoder(&field.ty, false)?);
                        params.push((param_name(field), self.ty(&field.ty, false)?));
                    }
                    events.push(ScriptEvent {
                        signal: true,
                        name: member_name(&signal.name, &signal.extras.export.py),
                        at: Self::at(&lookups, 's', &signal.name),
                        decoders,
                        params,
                    });
                }
                Member::Property(property) if !matches!(property.ty, Type::Callback(_)) => events.push(ScriptEvent {
                    signal: false,
                    name: format!("{}_changed", member_name(&property.name, &property.extras.export.py)),
                    at: Self::at(&lookups, 'p', &property.name),
                    decoders: vec![self.decoder(&property.ty, true)?],
                    params: vec![(
                        member_name(&property.name, &None),
                        self.ty_read(&property.ty, property.optional)?,
                    )],
                }),
                _ => {}
            }
        }
        // A signal the catalog names `title_changed` is the event of that name: the property's own change is not told twice under it.
        let signals: BTreeSet<String> = slots
            .iter()
            .filter_map(|slot| match &slot.member {
                Member::Signal(signal) => Some(member_name(&signal.name, &signal.extras.export.py)),
                _ => None,
            })
            .collect();
        events.retain(|event| event.signal || !signals.contains(&event.name));
        let has_instance_members = object.kind == ObjectKind::Handle
            || slots.iter().any(|slot| match &slot.member {
                Member::Method(method) => !method.function,
                _ => true,
            });
        let mut taken: BTreeSet<String> = ["on", "destroy"].iter().map(|s| (*s).to_owned()).collect();
        if object.kind == ObjectKind::Service && has_instance_members {
            taken.insert("get".to_owned());
        }
        let mut claim = |ident: &str| -> Result<(), String> {
            if taken.insert(ident.to_owned()) {
                Ok(())
            } else {
                Err(format!(
                    "`{}`: two members would both be `{ident}` in Python",
                    object.name
                ))
            }
        };

        // A class with nothing but functions is a namespace for them: it is not an object, so it has no `on` and no `destroy`.
        // At run time a class carries the members of its base (the ids number them that way), so it is flat; a stub names the base, so a type
        // checker takes a `Camera3d` where a `SceneNode` is wanted, as the library does.
        let bases = if self.stub {
            match object.base.as_deref().filter(|base| !base.contains('.')) {
                Some(base) => format!("({})", class_name(self.view.class(base)?)),
                None => String::new(),
            }
        } else if has_instance_members {
            "(_B.Object)".to_owned()
        } else {
            String::new()
        };
        c.line(format!("class {name}{bases}:"));
        c.indent();
        c.line(format!(
            "\"\"\"The class `{}` of `{}`.\"\"\"",
            object.name, self.view.namespace
        ));
        if !self.stub && has_instance_members {
            c.line("__slots__ = ()");
            c.line("_NS = _NS");
            c.line(format!("_IDS = staticmethod(_ids_{snake_name})"));
            let table: Vec<String> = events
                .iter()
                .map(|event| {
                    format!(
                        "{}: ({}, ({}))",
                        quote(&event.name),
                        event.at,
                        event
                            .decoders
                            .iter()
                            .map(|d| format!("{d},"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                })
                .collect();
            c.line(format!("_EVENTS = {{{}}}", table.join(", ")));
        }
        if has_instance_members {
            self.constructor_source(c, object, &slots, &lookups)?;
        }
        for slot in &slots {
            match &slot.member {
                Member::Property(property) => self.property_source(c, &snake_name, &lookups, property, &mut claim)?,
                Member::Method(method) => self.method_source(c, &snake_name, &lookups, method, &mut claim)?,
                Member::Signal(_) => {}
            }
        }
        if self.stub && has_instance_members {
            self.on_stubs(c, &events);
        }
        c.dedent();
        c.blank();
        if !self.stub {
            c.line(format!("_B.register(_NS_NAME, {}, {name})", quote(&object.name)));
            c.blank();
        }
        Ok(())
    }

    /// The overloads of `on` in a stub: a handler for each event, and the decorator form.
    fn on_stubs(&self, c: &mut CodeWriter, events: &[ScriptEvent]) {
        for event in events {
            let (name, params) = (&event.name, &event.params);
            let types: Vec<&str> = params.iter().map(|(_, ty)| ty.as_str()).collect();
            let handler = format!("Callable[[{}], object]", types.join(", "));
            c.line("@overload");
            c.line(format!(
                "def on(self, event: Literal[{}], handler: {handler}) -> Listener: ...",
                quote(name)
            ));
            c.line("@overload");
            c.line(format!(
                "def on(self, event: Literal[{}]) -> Callable[[_F], _F]: ...",
                quote(name)
            ));
        }
        if events.is_empty() {
            // The base class has `on` whatever the class: with no event to name it raises, and the stub says what it takes.
            c.line("def on(self, event: str, handler: Callable[..., object] | None = None) -> Listener: ...");
        }
        c.line("def destroy(self) -> None: ...");
    }

    fn constructor_source(
        &self,
        c: &mut CodeWriter,
        object: &ObjectType,
        slots: &[ClassMember<'_>],
        lookups: &[MemberLookup],
    ) -> Result<(), String> {
        let snake_name = snake(&object.name);
        if object.kind == ObjectKind::Service {
            c.line("@classmethod");
            self.def(
                c,
                &format!("def get(cls) -> {}", class_name(object)),
                "The service.",
                &[
                    format!("i = _ids_{snake_name}()"),
                    "return cls(_NS.create(i[0], []))".to_owned(),
                ],
            );
            return Ok(());
        }
        let property_named = |wanted: &str| -> Result<&Property, String> {
            slots
                .iter()
                .find_map(|slot| match &slot.member {
                    Member::Property(property) if property.name == wanted => Some(*property),
                    _ => None,
                })
                .ok_or_else(|| {
                    format!(
                        "`{}`: the constructor names `{wanted}`, which is not a property",
                        object.name
                    )
                })
        };
        match &object.ctor {
            Some(names) => {
                let mut params = vec!["self".to_owned()];
                let mut inits = Vec::new();
                for wanted in names {
                    let property = property_named(wanted)?;
                    let param = member_name(&property.name, &None);
                    params.push(format!("{param}: {}", self.ty(&property.ty, true)?));
                    inits.push(format!(
                        "(i[{}], {})",
                        Self::at(lookups, 'p', &property.name),
                        self.enc(&property.ty, &param)?
                    ));
                }
                self.def(
                    c,
                    &format!("def __init__({}) -> None", params.join(", ")),
                    "Makes the object.",
                    &[
                        format!("i = _ids_{snake_name}()"),
                        format!("_B.Object.__init__(self, _NS.create(i[0], [{}]))", inits.join(", ")),
                    ],
                );
            }
            None => {
                let mut params = vec!["self".to_owned(), "*".to_owned()];
                let mut lines = vec![format!("i = _ids_{snake_name}()"), "inits = []".to_owned()];
                for slot in slots.iter().filter(|slot| !slot.foreign) {
                    if let Member::Property(property) = &slot.member
                        && !property.readonly
                    {
                        let param = member_name(&property.name, &None);
                        params.push(format!("{param}: {} | None = None", self.ty(&property.ty, true)?));
                        lines.push(format!(
                            "if {param} is not None: inits.append((i[{}], {}))",
                            Self::at(lookups, 'p', &property.name),
                            self.enc(&property.ty, &param)?
                        ));
                    }
                }
                if params.len() == 2 {
                    // Nothing to set: a bare `*` with no keyword after it is not Python.
                    params.pop();
                }
                lines.push("_B.Object.__init__(self, _NS.create(i[0], inits))".to_owned());
                self.def(
                    c,
                    &format!("def __init__({}) -> None", params.join(", ")),
                    "Makes the object; each keyword sets the property of its name.",
                    &lines,
                );
            }
        }
        Ok(())
    }

    fn property_source(
        &self,
        c: &mut CodeWriter,
        snake_name: &str,
        lookups: &[MemberLookup],
        property: &Property,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&property.name, &property.extras.export.py);
        claim(&ident)?;
        let at = Self::at(lookups, 'p', &property.name);
        let ids = format!("_ids_{snake_name}()[{at}]");
        let callback = matches!(property.ty, Type::Callback(_));
        let write_ty = self.ty(&property.ty, true)?;
        if callback {
            // Written, never read back: a plain attribute in a stub, a write-only property in the module.
            if self.stub {
                c.line(format!("{ident}: {write_ty}"));
            } else {
                c.line(format!("def _set_{ident}(self, value):"));
                c.line(format!("    self._h.set({ids}, {})", self.enc(&property.ty, "value")?));
                c.line(format!("{ident} = builtins.property(fset=_set_{ident})"));
            }
            return Ok(());
        }
        let read = self.ty_read(&property.ty, property.optional)?;
        c.line("@builtins.property");
        self.def(
            c,
            &format!("def {ident}(self) -> {read}"),
            &format!("The property `{}`.", property.name),
            &[format!(
                "return {}",
                self.dec(&property.ty, &format!("self._h.get({ids})"), true)?
            )],
        );
        if !property.readonly {
            c.line(format!("@{ident}.setter"));
            self.def(
                c,
                &format!("def {ident}(self, value: {write_ty}) -> None"),
                &format!("Sets the property `{}`.", property.name),
                &[format!("self._h.set({ids}, {})", self.enc(&property.ty, "value")?)],
            );
            if property.animated {
                let timed = format!("set_{ident}_timed");
                claim(&timed)?;
                self.def(
                    c,
                    &format!("def {timed}(self, value: {write_ty}, timing: Timing) -> None"),
                    &format!("Sets the property `{}` over a time.", property.name),
                    &[format!(
                        "self._h.set_timed({ids}, {}, _B.timing(timing))",
                        self.enc(&property.ty, "value")?
                    )],
                );
            }
        }
        Ok(())
    }

    fn method_source(
        &self,
        c: &mut CodeWriter,
        snake_name: &str,
        lookups: &[MemberLookup],
        method: &Method,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&method.name, &method.extras.export.py);
        claim(&ident)?;
        let (params, args) = self.params(&method.params)?;
        let at = Self::at(lookups, 'm', &method.name);
        let returns = match &method.returns {
            Some(ty) if method.optional => self.ty_read(ty, true)?,
            Some(ty) => self.ty(ty, false)?,
            None => "None".to_owned(),
        };
        let call = if method.function {
            format!("_NS.call_static(i[0], i[{at}], [{}])", args.join(", "))
        } else {
            format!("self._h.call(i[{at}], [{}])", args.join(", "))
        };
        let result = match &method.returns {
            Some(ty) => format!("return {}", self.dec(ty, &call, method.optional)?),
            None => call,
        };
        let doc = format!(
            "The {} `{}`.",
            if method.function { "function" } else { "method" },
            method.name
        );
        let body = [format!("i = _ids_{snake_name}()"), result];
        if method.function {
            c.line("@staticmethod");
            self.def(c, &format!("def {ident}({params}) -> {returns}"), &doc, &body);
        } else {
            let params = if params.is_empty() {
                "self".to_owned()
            } else {
                format!("self, {params}")
            };
            self.def(c, &format!("def {ident}({params}) -> {returns}"), &doc, &body);
        }
        Ok(())
    }

    fn interface_source(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        if !object.properties.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the interface `{}` declares members that are not methods",
                object.name
            ));
        }
        let name = class_name(object);
        let snake_name = snake(&object.name);
        let lookups: Vec<MemberLookup> = object
            .methods
            .iter()
            .map(|method| MemberLookup {
                kind: 'm',
                name: method.name.clone(),
            })
            .collect();
        if !self.stub {
            self.ids_source(c, object, &lookups);
        }
        c.line(format!(
            "class {name}{}:",
            if self.stub { "" } else { "(_B.Implementor)" }
        ));
        c.indent();
        c.line(format!(
            "\"\"\"The interface `{}` of `{}`: a script subclasses it and defines the methods it wants to answer.\"\"\"",
            object.name, self.view.namespace
        ));
        for method in &object.methods {
            let ident = member_name(&method.name, &method.extras.export.py);
            let (params, _) = self.params(&method.params)?;
            let params = if params.is_empty() {
                "self".to_owned()
            } else {
                format!("self, {params}")
            };
            let returns = match &method.returns {
                Some(ty) => self.ty(ty, true)?,
                None => "None".to_owned(),
            };
            if self.stub {
                c.line(format!("def {ident}({params}) -> {returns}: ..."));
            }
        }
        if self.stub && object.methods.is_empty() {
            c.line("...");
        }
        if !self.stub && object.methods.is_empty() {
            c.line("__slots__ = ()");
        }
        c.dedent();
        c.blank();
        if self.stub {
            return Ok(());
        }
        c.line(format!("def _implement_{snake_name}(obj):"));
        c.indent();
        c.line(format!("i = _ids_{snake_name}()"));
        c.line("implemented = []");
        let mut arms = Vec::new();
        for method in &object.methods {
            let ident = member_name(&method.name, &method.extras.export.py);
            let at = Self::at(&lookups, 'm', &method.name);
            c.line(format!(
                "if callable(getattr(obj, {}, None)): implemented.append(i[{at}])",
                quote(&ident)
            ));
            let reads: Vec<String> = method
                .params
                .iter()
                .enumerate()
                .map(|(index, field)| self.dec(&field.ty, &format!("args[{index}]"), false))
                .collect::<Result<_, _>>()?;
            let call = format!("obj.{ident}({})", reads.join(", "));
            let answer = match &method.returns {
                Some(ty) => self.enc(ty, &call)?,
                None => call,
            };
            arms.push(format!("if member == i[{at}]:\n        return {answer}"));
        }
        c.line("def run(member, args):");
        for arm in &arms {
            for line in arm.lines() {
                c.line(format!("    {line}"));
            }
        }
        c.line("    raise RuntimeError(\"the library called a method this interface does not have\")");
        c.line("return _NS.implement(i[0], implemented, run)");
        c.dedent();
        c.blank();
        Ok(())
    }
}

/// The classes of `view`'s namespace as a Python module, or as its stub.
pub(super) fn render(view: &ScriptView<'_>, stub: bool) -> Result<String, String> {
    let py = Py { view, stub };
    let mut c = CodeWriter::new("    ");
    c.line(format!(
        "\"\"\"The classes of `{}`, over the object protocol.\"\"\"",
        view.namespace
    ));
    c.blank();
    if stub {
        c.line("import builtins");
        c.line("import dataclasses");
        c.line("import enum");
        c.line("from typing import Callable, Literal, TypeVar, overload");
        c.blank();
        c.line("from gnoblin.objects import Listener as Listener, Timing as Timing");
        c.blank();
        c.line("_F = TypeVar(\"_F\", bound=Callable[..., object])");
        c.blank();
    } else {
        c.line("from __future__ import annotations");
        c.blank();
        c.line("import builtins");
        c.line("import dataclasses");
        c.line("import enum");
        c.line("from typing import Callable");
        c.blank();
        c.line("from gnoblin.objects import _base as _B");
        c.line("from gnoblin.objects._base import Timing");
        c.blank();
        c.line(format!("_NS_NAME = {}", quote(view.namespace.as_str())));
        c.line("_NS = _B.open(_NS_NAME)");
    }
    c.blank();
    for declared in view.enums.values() {
        py.enum_source(&mut c, declared);
    }
    for declared in view.callbacks.values() {
        py.callback_source(&mut c, declared)?;
    }
    for object in view.objects.values() {
        match object.kind {
            ObjectKind::Record => py.record_source(&mut c, object)?,
            ObjectKind::Handle | ObjectKind::Service => py.handle_source(&mut c, object)?,
            ObjectKind::Interface => py.interface_source(&mut c, object)?,
        }
    }
    Ok(c.finish())
}

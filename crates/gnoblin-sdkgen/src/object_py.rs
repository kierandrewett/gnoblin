//! The Python classes of a namespace served through the object protocol: `gnoblin/objects/<module>.py` and its `.pyi`.
//!
//! The classes call the built-in module `_gnoblin_objects` (`gnoblin-runtime-python`'s `object_native.rs`) through the small runtime in
//! `_base.py`. A value crosses as Python's own: a record is a `dataclass` (the native layer sees it as a `dict` of its fields, in the
//! catalog's order, and hands one back as a `tuple`), an enum is an `IntEnum`, a handle is an instance of a generated class and a closure is
//! a callable. One renderer writes the module and its stub: the stub is the same text with every body `...`.

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_schema::{CallbackDecl, DefaultValue, EnumDecl, Field, Method, ObjectKind, ObjectType, Property, Type};

use super::object_script::{ScriptView, quote};
use super::objects::{
    ClassMember, Member, augment_finds, augments_of, check_assign, check_foreign, ctor_properties, finds_of,
    is_augment_setter, slots_of,
};
use super::text::{CodeWriter, pascal, python_ident, snake};
use super::values::{self, Absent, CarrySlot, ValueClass, absent, is_family, py_field};

/// `gnoblin/objects/__init__.py`: the names a script imports.
pub(super) const INIT_PY: &str = r#""""The classes of the libraries' object protocol.

`gnoblin.objects.std` has the classes of `gnoblin.std` and `gnoblin.objects.model` those of `gnoblin.model`, written from the catalog the
libraries serve. This package holds what they share.
"""

from ._base import Implementor, Listener, Timing

__all__ = ["Implementor", "Listener", "Timing"]
"#;

/// `gnoblin/objects/__init__.pyi`.
pub(super) const INIT_PYI: &str = r#"from dataclasses import dataclass, field

class Implementor:
    """The base of the generated interface classes: a script subclasses one and defines the methods it wants to answer."""

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

__all__ = ["Implementor", "Listener", "Timing"]
"#;

/// `gnoblin/objects/_base.py`: the runtime the generated classes share.
pub(super) const BASE_PY: &str = r#""""The runtime the generated object classes share: the native module, the handle of a library object and the checks of values that go in."""

from __future__ import annotations

import dataclasses
import enum
import functools
import re
import weakref
from typing import Any, Callable

import _gnoblin_objects as native
from gnoblin.errors import GnoblinError

_classes: dict[tuple[str, str], type] = {}
_naming = False


def _identity(obj: Any) -> str | None:
    """The `id` of `obj` as text, when it has one that can be read: a failed read of it names nothing and is not another failure."""
    global _naming
    if _naming or obj is None:
        return None
    _naming = True
    try:
        ident = getattr(obj, "id", None)
        return str(ident) if isinstance(ident, (str, int)) and not isinstance(ident, bool) else None
    except Exception:
        return None
    finally:
        _naming = False


# The library words a refusal with the names of the catalog: a member is `class.member` and a parameter or a field of a record is its
# own name. A script spells them as the generated class does, so the member that knows both (its `spell` table,
# `(class, {member: (spelling, {name: spelling})})`) writes the script's spelling in their place.
_LABEL = re.compile(
    r"`([a-z][a-z0-9_]*)\.([a-z][a-z0-9_]*)`((?:\[\d+\]|\.[a-z][a-z0-9_]*)*)(?:: `([a-z][a-z0-9_]*)`((?:\[\d+\]|\.[a-z][a-z0-9_]*)*))?"
)
_TARGET = re.compile(r"^([A-Za-z0-9_]+)(#[^.:\s]*)?\.([a-z][a-z0-9_]*)(?=:|\()")
_STEP = re.compile(r"\.([a-z][a-z0-9_]*)")


def _respell(message: str, class_name: str, spell: tuple[str, dict[str, Any]]) -> str:
    """`message` with the catalog's names of the members of the class `spell` describes written as the class `class_name` spells them: the
    label of the member (`class.member`, then a record's fields or a list's indexes, then the parameter a refusal names) and the target a
    library puts first in its own words (`Class#id.member:`). A name `spell` does not know is left as it is."""
    schema, members = spell

    def path(steps: str, names: dict[str, str]) -> str:
        return _STEP.sub(lambda step: "." + names.get(step.group(1), step.group(1)), steps)

    def label(found: re.Match[str]) -> str:
        owner, name, steps, parameter, parameter_steps = found.groups()
        entry = members.get(name) if owner == schema else None
        if entry is None:
            return found.group(0)
        spelling, names = entry
        text = f"`{class_name}.{spelling}`{path(steps, names)}"
        if parameter is not None:
            text += f": `{names.get(parameter, parameter)}`{path(parameter_steps, names)}"
        return text

    def target(found: re.Match[str]) -> str:
        owner, ident, name = found.groups()
        entry = members.get(name) if owner == class_name else None
        return found.group(0) if entry is None else f"{owner}{ident or ''}.{entry[0]}"

    return _TARGET.sub(target, _LABEL.sub(label, message), count=1)


def _named(
    error: GnoblinError, obj: Any, class_name: str, member: str, spell: tuple[str, dict[str, Any]] | None = None
) -> GnoblinError:
    """`error` with the target `Class#id.member` (`Class.member` when there is no object or it has no id), and the message that begins with it.
    The names of the catalog in its words are written as the class spells them when the member gives its `spell` table."""
    if getattr(error, "target", None) is not None:
        return error
    ident = _identity(obj)
    target = f"{class_name}.{member}" if ident is None else f"{class_name}#{ident}.{member}"
    words = str(error) if spell is None else _respell(str(error), class_name, spell)
    for lead in (f"{target}: ", f"{target}(): "):
        if words.startswith(lead):
            words = words[len(lead) :]
            break
    error.target = target
    error.args = (f"{target}: {words}",)
    return error


def named(
    class_name: str | None, member: str, static: bool = False, spell: tuple[str, dict[str, Any]] | None = None
) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Makes a failure of the library in the decorated member a `GnoblinError` that names `class_name` (the class of the object when it is
    none), the object and `member`. A bad argument is a `TypeError`, and is left as it is but for the names of the catalog in its words, which
    are written as the class spells them when the member gives its `spell` table."""

    def wrap(fn: Callable[..., Any]) -> Callable[..., Any]:
        @functools.wraps(fn)
        def run(*args: Any, **kwargs: Any) -> Any:
            try:
                return fn(*args, **kwargs)
            except GnoblinError as error:
                obj = None if static or not args else args[0]
                raise _named(error, obj, class_name or type(obj).__name__, member, spell) from None
            except TypeError as error:
                if spell is not None and error.args and isinstance(error.args[0], str):
                    obj = None if static or not args else args[0]
                    error.args = (_respell(error.args[0], class_name or type(obj).__name__, spell), *error.args[1:])
                raise

        return run

    return wrap


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


def always(value: Any, what: str) -> Any:
    """`value` of the property `what`, which the catalog says always has an object: a none answer is the library's fault."""
    if value is None:
        raise GnoblinError(f"{what} has no object, but the catalog says the library always has one")
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
            try:
                self._namespace.unlisten(registration)
            except GnoblinError as error:
                raise _named(error, None, "Listener", "stop") from None


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


# One library object is one script object: each object the script holds is filed under the identity of its handle (`handle.key`, the
# namespace and the handle), so an object handed over twice is the same Python object both times and `is`, `==`, `in` and a dict key work
# on it. The second delivery's handle is dropped, which releases its reference, since the script object holds one already.
_live: weakref.WeakValueDictionary[Any, Any] = weakref.WeakValueDictionary()


def _remember(handle: Any, instance: Any) -> Any:
    try:
        _live[handle.key] = instance
    except TypeError:
        # A class with no weak reference slot (an implemented interface) is not filed: the script made it and holds it already.
        pass
    return instance


def wrap(namespace: str, handle: Any, fallback: type) -> Any:
    """The script object that holds the library object of `handle`: the one there is, or a new instance of the class the library says it
    is (of `fallback` when the script has no such class)."""
    found = _live.get(handle.key)
    if found is not None:
        return found
    cls = _classes.get((namespace, handle.class_name), fallback)
    return _remember(handle, cls._wrap(handle))


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


def ref_to(value: Any, cls: type, what: str) -> Any:
    """An object of another namespace's class on its way in. The wire carries no namespace and the two libraries issue the same handle numbers, so a value that is not an instance of `cls` is refused here, before the library could mistake it for one of its own."""
    if not isinstance(value, cls):
        raise TypeError(f"expected {what}, got {type(value).__name__}")
    return ref(value)


_extra_finders: dict[type, dict[str, tuple[str, type]]] = {}


def augment_find(target: type, kind: str, method: str, query: type) -> None:
    """Adds the kind `kind` to the `find` of the core class `target`: its objects are listed by the method `method`, which a library above added, filtered by the record class `query`. A kind the class has of its own is never replaced."""
    _extra_finders.setdefault(target, {})[kind] = (method, query)


def find(service: Any, label: str, finders: dict[str, tuple[str, type]], kind: Any, filter: Any = None) -> list[Any]:
    """`find(kind, filter)` of a service: the objects of one kind that match `filter`: a query, a dict of field values, or a function that keeps an object when it returns a true value."""
    extra: dict[str, tuple[str, type]] = {}
    for cls in reversed(type(service).__mro__):
        extra.update(_extra_finders.get(cls, {}))
    finders = {**extra, **finders}
    entry = finders.get(kind) if isinstance(kind, str) else None
    if entry is None:
        raise TypeError(f"{label}.find: no kind {kind!r}; the kinds that work are {', '.join(finders)}")
    method, query = entry
    if filter is not None and not isinstance(filter, query):
        if callable(filter):
            return [found for found in getattr(service, method)() if filter(found)]
        if not isinstance(filter, dict):
            raise TypeError(f"{label}.find({kind!r}): the filter is a {query.__name__}, a dict of fields or a function, not {type(filter).__name__}")
        known = [field.name for field in dataclasses.fields(query)]
        for key in filter:
            if key not in known:
                raise TypeError(f"{label}.find({kind!r}): no field {key!r} (known: {', '.join(known)})")
        filter = query(**filter)
    return getattr(service, method)(filter)


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
        _remember(handle, self)

    @named(None, "destroy")
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

            try:
                return Listener(self._NS, self._h.listen(self._IDS()[at], run))
            except GnoblinError as error:
                raise _named(error, self, type(self).__name__, event) from None

        if handler is not None:
            return attach(handler)

        def decorator(fn: Callable[..., Any]) -> Callable[..., Any]:
            attach(fn)
            return fn

        return decorator

    def __repr__(self) -> str:
        return f"<{type(self).__name__} {self._h!r}>"


_augmented: set[tuple[type, str]] = set()


def _augmentable(target: type, name: str) -> None:
    """Refuses a member the class has of its own; a library that is loaded again replaces what it added before."""
    if hasattr(target, name) and (target, name) not in _augmented:
        raise TypeError(f"{target.__name__} has a member {name!r} of its own: a library cannot add one of the same name")
    _augmented.add((target, name))


def augment_property(
    target: type, name: str, read: Callable[[Any], Any], write: Callable[[Any, Any], None] | None = None
) -> None:
    """Adds the property `name` to the core class `target`, which reads `read(obj)` for the object it is read on. With `write`, an
    assignment calls `write(obj, value)`; without it the property is read-only."""
    _augmentable(target, name)
    setter = None if write is None else lambda self, value: write(self, value)
    setattr(target, name, property(lambda self: read(self), setter))


def augment_method(target: type, name: str, run: Callable[..., Any]) -> None:
    """Adds the method `name` to the core class `target`, which calls `run(obj, *args, **kwargs)` for the object it is called on."""
    _augmentable(target, name)

    def method(self: Any, *args: Any, **kwargs: Any) -> Any:
        return run(self, *args, **kwargs)

    method.__name__ = name
    setattr(target, name, method)


def assign(obj: Any, label: str, props: Any, options: Any, request: Callable[[Callable[[], None], Any], Any]) -> Any:
    """`set(props, options)` of an object: writes each property `props` names in one transition, which `request(work, options)` commits.
    Every key must be a writable property of the object, or nothing is written."""
    if not isinstance(props, dict):
        raise TypeError(f"{label}.set: expects a dict of properties")
    for key in props:
        found = getattr(type(obj), key, None) if isinstance(key, str) else None
        if not isinstance(found, property) or found.fset is None:
            raise TypeError(f"{label}.set: {key} is not a writable property")

    def work() -> None:
        for key, value in props.items():
            setattr(obj, key, value)

    return request(work, options)
"#;

/// The namespace and the object of the root every shell starts from, and what its stub's class of members is called after the class.
const ROOT_NAMESPACE: &str = "gnoblin.model";
const ROOT_OBJECT: &str = "gnoblin";
const ROOT_SUFFIX: &str = "Root";

fn class_name(object: &ObjectType) -> String {
    values::py_class(&object.name, &object.export)
}

pub(super) fn member_name(name: &str, export: &Option<String>) -> String {
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

/// The docstring of `on`: what it does, and the events of the class with what each is.
fn on_doc(events: &[ScriptEvent]) -> String {
    let mut text = "Calls `handler` each time `event` happens, until the returned listener is stopped. Without a handler it is a decorator.\n\nThe events:".to_owned();
    for event in events {
        text.push_str(&format!("\n\n* `{}`: {}", event.name, event.doc.replace("\n\n", " ")));
    }
    text
}

/// An event of a class: a signal, or a property's change.
struct ScriptEvent {
    /// Whether the catalog declares it as a signal (the change of a property is the other kind).
    signal: bool,
    name: String,
    at: usize,
    decoders: Vec<String>,
    params: Vec<(String, String)>,
    /// What the event is, for the docstring of `on`.
    doc: String,
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
            Type::ObjectRef(name) if name.contains('.') => self.view.foreign(name)?.alias_py(),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                let base = class_name(class);
                if input && class.number.is_some() {
                    format!("{base} | float")
                } else if input && class.tag.is_some() {
                    // A variant may be given as a dict with one key, its tag.
                    format!("{base} | dict[str, Any]")
                } else {
                    base
                }
            }
            Type::List(inner) => format!("list[{}]", self.ty(inner, input)?),
            Type::Callback(name) => {
                let declared = self.view.callback(name)?;
                let params: Vec<String> = declared
                    .params
                    .iter()
                    .map(|field| self.ty(&field.ty, false))
                    .collect::<Result<_, _>>()?;
                let returns = match &declared.returns {
                    // A function may answer nothing, which the library reads as the answer its declaration documents.
                    Some(ty) if declared.optional => format!("{} | None", self.ty(ty, true)?),
                    Some(ty) => self.ty(ty, true)?,
                    None => "None".to_owned(),
                };
                format!("Callable[[{}], {returns}]", params.join(", "))
            }
        })
    }

    /// The type a property or a result is read as: an object or a record may be absent (`T | None`) unless the catalog says the library
    /// `always` has one, the rule of `T | undefined` in TypeScript and of `Option<T>` in Rust.
    fn ty_read(&self, ty: &Type, always: bool) -> Result<String, String> {
        let text = self.ty(ty, false)?;
        Ok(if !always && matches!(ty, Type::ObjectRef(_)) {
            format!("{text} | None")
        } else {
            text
        })
    }

    fn enc(&self, ty: &Type, e: &str) -> Result<String, String> {
        Ok(match ty {
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes => e.to_owned(),
            Type::Enum(name) => format!("_enc_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) if name.contains('.') => {
                let foreign = self.view.foreign(name)?;
                format!(
                    "_B.ref_to({e}, {}, {})",
                    foreign.alias_py(),
                    quote(&format!("a {}", foreign.py))
                )
            }
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
            // An object of another library is read in that library's namespace, as an instance of its class there.
            Type::ObjectRef(name) if name.contains('.') => {
                let foreign = self.view.foreign(name)?;
                let home = quote(&foreign.namespace);
                if absent {
                    format!(
                        "_B.opt({e}, lambda handle: _B.wrap({home}, handle, {}))",
                        foreign.alias_py()
                    )
                } else {
                    format!("_B.wrap({home}, {e}, {})", foreign.alias_py())
                }
            }
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

    /// The decorator that makes a failure of the library in the member that follows a `GnoblinError` naming `class` and `member`. A stub
    /// has none. `bare` is for a member that has no object (a function, a constructor). `spell` is the table of the class
    /// (`_spell_<class>`, [`Py::spell_source`]) that writes the names the library's words carry as the script spells them.
    fn named(&self, c: &mut CodeWriter, class: &str, member: &str, bare: bool, spell: Option<&str>) {
        if !self.stub {
            let tail = if bare { ", True" } else { "" };
            let spell = spell.map(|table| format!(", spell={table}")).unwrap_or_default();
            c.line(format!("@_B.named({class:?}, {member:?}{tail}{spell})"));
        }
    }

    /// `header` and its body: one `...` line in a stub, the body indented in a module. A stub carries the schema's description of the
    /// member, when there is one, as the member's docstring.
    fn def(&self, c: &mut CodeWriter, header: &str, doc: &str, body: &[String]) {
        self.def_described(c, header, "", doc, body);
    }

    fn def_described(&self, c: &mut CodeWriter, header: &str, schema: &str, doc: &str, body: &[String]) {
        if self.stub {
            if schema.is_empty() {
                c.line(format!("{header}: ..."));
            } else {
                c.line(format!("{header}:"));
                c.indent();
                c.pydoc(schema);
                c.dedent();
            }
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

    /// The docstring of a class: the schema's description, or `fallback` when the schema has none.
    fn class_doc(&self, c: &mut CodeWriter, text: &str, fallback: impl FnOnce() -> String) {
        if text.is_empty() {
            c.line(format!("\"\"\"{}\"\"\"", fallback()));
        } else {
            c.pydoc(text);
        }
    }

    /// True for the root service of the object model: `gnoblin` of `gnoblin.model`.
    fn is_root(&self, object: &ObjectType) -> bool {
        object.kind == ObjectKind::Service
            && self.view.namespace.as_str() == ROOT_NAMESPACE
            && object.name == ROOT_OBJECT
    }

    fn enum_source(&self, c: &mut CodeWriter, declared: &EnumDecl) {
        let name = pascal(&declared.name);
        c.line(format!("class {name}(enum.IntEnum):"));
        c.indent();
        let values = declared
            .values
            .iter()
            .map(|value| format!("`{value}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let text = self.view.enum_doc(&declared.name);
        if text.is_empty() {
            c.line(format!("\"\"\"The enum `{}`: {values}.\"\"\"", declared.name));
        } else {
            c.pydoc(&format!("{text}\n\nThe values: {values}."));
        }
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

    /// The classes and the conversion of a record: one `dataclass` and its codec, or for the family of a tagged value, a class and a codec
    /// for each variant, the union of them, and the codec that tells them apart.
    fn record_source(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        if !object.methods.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the record `{}` declares members that are not properties",
                object.name
            ));
        }
        for class in values::value_classes([object]) {
            self.class_source(c, &class)?;
            if self.stub {
                continue;
            }
            match class.of {
                None => self.record_codec(c, object)?,
                Some((family, variant)) => self.variant_codec(c, &class, family, variant)?,
            }
        }
        if is_family(object) {
            let classes: Vec<String> = object
                .variants
                .iter()
                .map(|variant| values::py_class(&variant.name, &variant.export))
                .collect();
            c.line(format!("{} = {}", class_name(object), classes.join(" | ")));
            c.blank();
            if !self.stub {
                self.family_codec(c, object)?;
            }
        }
        Ok(())
    }

    /// A value class as a `dataclass`: the fields in the schema's order, each with its default.
    fn class_source(&self, c: &mut CodeWriter, class: &ValueClass<'_>) -> Result<(), String> {
        let name = class.py();
        let keyword_only = {
            let mut defaulted = false;
            let mut forced = false;
            for property in class.properties {
                if property.default.is_some() || property.optional {
                    defaulted = true;
                } else if defaulted {
                    forced = true;
                }
            }
            forced
        };
        c.line(if keyword_only {
            "@dataclasses.dataclass(kw_only=True)"
        } else {
            "@dataclasses.dataclass"
        });
        c.line(format!("class {name}:"));
        c.indent();
        match class.of {
            None => self.class_doc(c, self.view.class_doc(class.name), || {
                format!("The record `{}`.", class.name)
            }),
            Some((family, variant)) => c.line(format!(
                "\"\"\"The `{}` variant of the record `{}`.\"\"\"",
                variant.tag, family.name
            )),
        }
        for property in class.properties {
            let field = py_field(property);
            let ty = self.ty(&property.ty, false)?;
            let described = self.stub.then(|| self.view.member_doc(class.name, &property.name));
            match absent(property) {
                Absent::Default(default) => {
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
                Absent::None => c.line(format!(
                    "{field}: {ty} | None = {}",
                    if self.stub { "..." } else { "None" }
                )),
                Absent::Required => c.line(format!("{field}: {ty}")),
            }
            if let Some(text) = described {
                c.pydoc(text);
            }
        }
        c.dedent();
        c.blank();
        Ok(())
    }

    /// The expression that makes the library's value of `property` from the script's `given`; a field left at none starts at its default.
    fn field_in(&self, owner: &str, property: &Property, given: &str) -> Result<String, String> {
        Ok(match absent(property) {
            Absent::Default(default) => {
                let value = self.default_expr(&property.ty, default)?;
                self.enc(&property.ty, &format!("({given} if {given} is not None else {value})"))?
            }
            Absent::None => format!("_B.opt({given}, lambda value: {})", self.enc(&property.ty, "value")?),
            Absent::Required => self.enc(
                &property.ty,
                &format!(
                    "_B.need({given}, {})",
                    quote(&format!("{owner}.{}", py_field(property)))
                ),
            )?,
        })
    }

    /// The script value of `property` from the library's `read`; a field the sender left out is none, or its default.
    fn field_out(&self, property: &Property, read: &str) -> Result<String, String> {
        let decoded = self.dec(&property.ty, read, false)?;
        Ok(match absent(property) {
            Absent::Required => decoded,
            Absent::None => format!(
                "_B.opt({read}, lambda value: {})",
                self.dec(&property.ty, "value", false)?
            ),
            Absent::Default(default) => format!(
                "({decoded} if {read} is not None else {})",
                self.default_expr(&property.ty, default)?
            ),
        })
    }

    fn record_codec(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        let name = class_name(object);
        let helper = snake(&object.name);
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        for (index, property) in object.properties.iter().enumerate() {
            let field = py_field(property);
            writes.push(format!(
                "{}: {}",
                quote(&property.name),
                self.field_in(&name, property, &format!("record.{field}"))?
            ));
            reads.push(format!(
                "{field}={}",
                self.field_out(property, &format!("fields[{index}]"))?
            ));
        }
        c.line(format!("def _enc_rec_{helper}(record):"));
        if let Some(field) = &object.number {
            // A bare number stands for the record with only its `number` field.
            let field = object
                .properties
                .iter()
                .find(|property| &property.name == field)
                .ok_or_else(|| format!("`{}` has no number field `{field}`", object.name))?;
            c.line("    if isinstance(record, (int, float)) and not isinstance(record, bool):");
            c.line(format!("        record = {name}({}=record)", py_field(field)));
        }
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

    /// The conversion of one variant to and from the family's record: the record's fields, by the record's names and in its order.
    fn variant_codec(
        &self,
        c: &mut CodeWriter,
        class: &ValueClass<'_>,
        family: &ObjectType,
        variant: &gnoblin_schema::TaggedVariant,
    ) -> Result<(), String> {
        let helper = snake(class.name);
        let py_name = class.py();
        let slots = values::slots(&|record| self.view.class(record), family, variant)?;
        let mut writes = Vec::new();
        let mut setup = Vec::new();
        let mut reads = Vec::new();
        for (index, slot) in slots.iter().enumerate() {
            let key = quote(&slot.field.name);
            match &slot.slot {
                CarrySlot::Tag(tag) => writes.push(format!("{key}: {}", self.enc(&slot.field.ty, &quote(tag))?)),
                CarrySlot::Whole(own) => {
                    writes.push(format!(
                        "{key}: {}",
                        self.field_in(&py_name, own, &format!("record.{}", py_field(own)))?
                    ));
                    reads.push(format!(
                        "{}={}",
                        py_field(own),
                        self.field_out(own, &format!("fields[{index}]"))?
                    ));
                }
                CarrySlot::Part { record, parts } => {
                    let given: Vec<String> = parts
                        .iter()
                        .map(|carried| {
                            let own = format!("record.{}", py_field(carried.own));
                            let value = match absent(carried.own) {
                                Absent::Default(default) => Ok(format!(
                                    "{own} if {own} is not None else {}",
                                    self.default_expr(&carried.own.ty, default)?
                                )),
                                _ => Ok::<String, String>(own),
                            }?;
                            Ok(format!("{}={value}", py_field(carried.inner)))
                        })
                        .collect::<Result<_, String>>()?;
                    writes.push(format!(
                        "{key}: _enc_rec_{}({}({}))",
                        snake(&record.name),
                        class_name(record),
                        given.join(", ")
                    ));
                    let held = format!("part_{}", snake(&slot.field.name));
                    setup.push(format!(
                        "{held} = _dec_rec_{}(_B.need(fields[{index}], {}))",
                        snake(&record.name),
                        quote(&format!("{}.{}", class_name(family), py_field(slot.field)))
                    ));
                    for carried in parts {
                        let value = format!("{held}.{}", py_field(carried.inner));
                        reads.push(match absent(carried.own) {
                            Absent::Default(default) => format!(
                                "{}=({value} if {value} is not None else {})",
                                py_field(carried.own),
                                self.default_expr(&carried.own.ty, default)?
                            ),
                            _ => format!("{}={value}", py_field(carried.own)),
                        });
                    }
                }
                CarrySlot::Empty => writes.push(format!("{key}: None")),
            }
        }
        c.line(format!("def _enc_var_{helper}(record):"));
        c.line(format!("    return {{{}}}", writes.join(", ")));
        c.blank();
        c.line(format!("def _dec_var_{helper}(fields):"));
        for line in &setup {
            c.line(format!("    {line}"));
        }
        c.line(format!("    return {py_name}({})", reads.join(", ")));
        c.blank();
        Ok(())
    }

    /// The conversion of a family: a script's value is an instance of one variant's class, and the library's record says by its tag which.
    fn family_codec(&self, c: &mut CodeWriter, family: &ObjectType) -> Result<(), String> {
        let helper = snake(&family.name);
        let name = class_name(family);
        let tag = family
            .properties
            .iter()
            .position(|property| Some(property.name.as_str()) == family.tag.as_deref())
            .ok_or_else(|| format!("the family `{}` has no `tag` property", family.name))?;
        let Type::Enum(tag_enum) = &family.properties[tag].ty else {
            return Err(format!("the `tag` of the family `{}` is not an enum", family.name));
        };
        let tag_class = pascal(&self.view.enum_decl(tag_enum)?.name);
        let classes: Vec<String> = family
            .variants
            .iter()
            .map(|variant| values::py_class(&variant.name, &variant.export))
            .collect();
        c.line(format!("def _enc_rec_{helper}(record):"));
        for (variant, class) in family.variants.iter().zip(&classes) {
            c.line(format!("    if isinstance(record, {class}):"));
            c.line(format!("        return _enc_var_{}(record)", snake(&variant.name)));
        }
        // A dict with one key, a variant's tag, is that variant: the key's value is the variant's one property, or its options.
        c.line("    if isinstance(record, dict) and len(record) == 1:");
        c.line("        ((key, value),) = record.items()");
        for (variant, class) in family.variants.iter().zip(&classes) {
            let key = snake(&variant.tag);
            c.line(format!("        if key == {}:", quote(&key)));
            let made = match variant.properties.as_slice() {
                [only] => format!("{class}({}=value)", py_field(only)),
                _ => format!("{class}(**value)"),
            };
            c.line(format!("            return _enc_var_{}({made})", snake(&variant.name)));
        }
        c.line(format!(
            "    raise TypeError(f\"expected a {name} (one of {}), got {{type(record).__name__}}\")",
            classes.join(", ")
        ));
        c.blank();
        c.line(format!("def _dec_rec_{helper}(fields):"));
        c.line(format!(
            "    kind = _dec_enum_{}(fields[{tag}])",
            snake(&self.view.enum_decl(tag_enum)?.name)
        ));
        for variant in &family.variants {
            c.line(format!("    if kind == {tag_class}.{}:", enum_member(&variant.tag)));
            c.line(format!("        return _dec_var_{}(fields)", snake(&variant.name)));
        }
        c.line(format!(
            "    raise TypeError(f\"the library sent a {name} of the kind {{kind!r}}, which this SDK has no class for\")"
        ));
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

    /// The names a script spells differently from the catalog, for a failure of a call to say them as the script wrote them: the class's
    /// table `_spell_<class>` has the class, each member of its own with the spelling the class gives it, and the parameters and the
    /// record fields the member takes, where they are spelled otherwise (`_B.named` reads it). The members of a base class that another
    /// library serves are worded by that library.
    fn spell_source(&self, c: &mut CodeWriter, object: &ObjectType, slots: &[ClassMember<'_>]) -> Result<(), String> {
        let mut members = Vec::new();
        for slot in slots.iter().filter(|slot| !slot.foreign) {
            let mut names = BTreeMap::new();
            let (name, spelling) = match &slot.member {
                Member::Property(property) => {
                    self.spelt_fields(&property.ty, &mut names, &mut BTreeSet::new())?;
                    (&property.name, member_name(&property.name, &property.extras.export.py))
                }
                Member::Method(method) => {
                    for param in &method.params {
                        names.insert(param.name.clone(), param_name(param));
                        self.spelt_fields(&param.ty, &mut names, &mut BTreeSet::new())?;
                    }
                    (&method.name, member_name(&method.name, &method.extras.export.py))
                }
                Member::Signal(_) => continue,
            };
            names.retain(|name, spelling| name != spelling.as_str());
            let names = names
                .iter()
                .map(|(name, spelling)| format!("{}: {}", quote(name), quote(spelling)))
                .collect::<Vec<_>>()
                .join(", ");
            members.push(format!("{}: ({}, {{{names}}})", quote(name), quote(&spelling)));
        }
        c.line(format!(
            "_spell_{} = ({}, {{{}}})",
            snake(&object.name),
            quote(&object.name),
            members.join(", ")
        ));
        Ok(())
    }

    /// The fields of the records a value of `ty` holds, with the name a script gives each (`names` maps the catalog's name to it).
    fn spelt_fields(
        &self,
        ty: &Type,
        names: &mut BTreeMap<String, String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        match ty {
            Type::List(inner) => self.spelt_fields(inner, names, seen),
            Type::ObjectRef(name) if !name.contains('.') => {
                let class = self.view.class(name)?;
                if class.kind == ObjectKind::Record && seen.insert(class.name.clone()) {
                    for field in &class.properties {
                        names.insert(field.name.clone(), py_field(field));
                        self.spelt_fields(&field.ty, names, seen)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
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
            self.spell_source(c, object, &slots)?;
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
                        doc: self.view.member_doc(&object.name, &signal.name).to_owned(),
                    });
                }
                Member::Property(property) if !matches!(property.ty, Type::Callback(_)) => events.push(ScriptEvent {
                    signal: false,
                    name: format!("{}_changed", member_name(&property.name, &property.extras.export.py)),
                    at: Self::at(&lookups, 'p', &property.name),
                    decoders: vec![self.decoder(&property.ty, true)?],
                    params: vec![(
                        member_name(&property.name, &None),
                        self.ty_read(&property.ty, property.always || object.augment_of.is_some())?,
                    )],
                    doc: format!("The property `{}` changed.", property.name),
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
        // checker takes a `Camera3D` where a `SceneNode` is wanted, as the library does.
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
        // The root's members are a class of their own in the stub, which `Gnoblin` extends with `get`: `from gnoblin import gnoblin` has them
        // too (the runtime hands them to the root), so what the root has is written once and in the schema's words.
        let root = self.stub && self.is_root(object);
        if root {
            c.line(format!("class {name}{ROOT_SUFFIX}{bases}:"));
            c.indent();
            c.line(format!(
                "\"\"\"The members of the root `{}` of `{}`: the class `{name}` has them, and so has the `gnoblin` object of `gnoblin.api`.\"\"\"",
                object.name, self.view.namespace
            ));
        } else {
            c.line(format!("class {name}{bases}:"));
            c.indent();
            self.class_doc(c, self.view.class_doc(&object.name), || {
                format!("The class `{}` of `{}`.", object.name, self.view.namespace)
            });
        }
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
        if has_instance_members && !root {
            self.constructor_source(c, object, &slots, &lookups)?;
        }
        for slot in &slots {
            match &slot.member {
                Member::Property(property) => {
                    self.property_source(
                        c,
                        object,
                        &snake_name,
                        &lookups,
                        property,
                        object.augment_of.is_some(),
                        &mut claim,
                    )?;
                }
                Member::Method(method) => {
                    self.method_source(c, object, &snake_name, &lookups, method, &mut claim)?;
                }
                Member::Signal(_) => {}
            }
        }
        let finds = finds_of(object);
        if has_instance_members && !finds.is_empty() {
            claim("find")?;
            self.find_source(c, object, &finds)?;
        }
        if has_instance_members && object.assign {
            claim("set")?;
            check_assign(object, None, self.view.objects.values().copied())?;
            self.assign_source(c, &format!("self, {}", quote(&name)))?;
        }
        if self.stub && has_instance_members {
            self.on_stubs(c, &events, !root);
        }
        c.dedent();
        c.blank();
        if root {
            c.line(format!("class {name}({name}{ROOT_SUFFIX}):"));
            c.indent();
            self.class_doc(c, self.view.class_doc(&object.name), || {
                format!("The class `{}` of `{}`.", object.name, self.view.namespace)
            });
            self.constructor_source(c, object, &slots, &lookups)?;
            c.line("def destroy(self) -> None: ...");
            c.dedent();
            c.blank();
        }
        if !self.stub {
            c.line(format!("_B.register(_NS_NAME, {}, {name})", quote(&object.name)));
            c.blank();
        }
        Ok(())
    }

    /// `set(props, options)` of a class: in a stub its signature, in a module the call of `_B.assign` for the receiver and label `given`
    /// (`self, "Workspace"`).
    fn assign_source(&self, c: &mut CodeWriter, given: &str) -> Result<(), String> {
        let options = self.ty(&Type::ObjectRef("transition_options".to_owned()), true)?;
        let transition = self.ty(&Type::ObjectRef("transition".to_owned()), false)?;
        if self.stub {
            c.line(format!(
                "def set(self, props: dict[str, Any], options: {options} | None = None) -> {transition}: ..."
            ));
            return Ok(());
        }
        c.line("def set(self, props, options=None):");
        c.indent();
        c.line("\"\"\"Writes each property `props` names in one transition. Without `options` the change is not animated.\"\"\"");
        c.line(format!(
            "return _B.assign({given}, props, options, {})",
            self.assign_request()?
        ));
        c.dedent();
        Ok(())
    }

    /// What commits a `set`: the root's `request_transition`, which the namespace adds to the core's `Gnoblin` ([`check_assign`]), not
    /// animated without options.
    fn assign_request(&self) -> Result<String, String> {
        let options = class_name(self.view.class("transition_options")?);
        Ok(format!(
            "lambda work, opts: _core.Gnoblin.get().request_transition(work, opts if opts is not None else {options}(duration=0))"
        ))
    }

    /// `find(kind, filter)` of a service: in a module the call of `_B.find` with the kinds the catalog gives the service, in a stub one
    /// overload for each kind and one for a kind a package adds.
    fn find_source(&self, c: &mut CodeWriter, object: &ObjectType, finds: &[(&str, &Method)]) -> Result<(), String> {
        if self.stub {
            for (kind, method) in finds {
                let Some(Type::List(inner)) = &method.returns else {
                    return Err(format!(
                        "`{}`: the finder of `{kind}` does not return a list",
                        object.name
                    ));
                };
                let found = self.ty(inner, false)?;
                let query = self.ty(&method.params[0].ty, true)?;
                c.line("@overload");
                c.line(format!(
                    "def find(self, kind: Literal[{}], filter: {query} | dict[str, object] | Callable[[{found}], object] | None = None) -> list[{found}]: ...",
                    quote(kind)
                ));
            }
            c.line("@overload");
            c.line("def find(self, kind: str, filter: dict[str, object] | Callable[[Any], object] | None = None) -> list[Any]: ...");
            return Ok(());
        }
        let mut kinds = Vec::new();
        for (kind, method) in finds {
            let query = self.ty(&method.params[0].ty, true)?;
            kinds.push(format!(
                "{}: ({}, {query})",
                quote(kind),
                quote(&member_name(&method.name, &method.extras.export.py))
            ));
        }
        c.line("def find(self, kind, filter=None):");
        c.indent();
        c.line(format!(
            "\"\"\"The live objects of one kind that match `filter`: {}. A filter is a query, a dict of field values or a function; an unknown kind or field raises and names what works.\"\"\"",
            finds.iter().map(|(kind, _)| format!("`{kind}`")).collect::<Vec<_>>().join(", ")
        ));
        c.line(format!(
            "return _B.find(self, {}, {{{}}}, kind, filter)",
            quote(&object.name),
            kinds.join(", ")
        ));
        c.dedent();
        Ok(())
    }

    /// The overloads of `on` in a stub: a handler for each event, and the decorator form.
    fn on_stubs(&self, c: &mut CodeWriter, events: &[ScriptEvent], destroy: bool) {
        // The docstring is the first overload's: the events of the class and what each is.
        let mut described = false;
        for event in events {
            let (name, params) = (&event.name, &event.params);
            let types: Vec<&str> = params.iter().map(|(_, ty)| ty.as_str()).collect();
            let handler = format!("Callable[[{}], object]", types.join(", "));
            c.line("@overload");
            if described || !self.view.has_docs() {
                c.line(format!(
                    "def on(self, event: Literal[{}], handler: {handler}) -> Listener: ...",
                    quote(name)
                ));
            } else {
                described = true;
                c.line(format!(
                    "def on(self, event: Literal[{}], handler: {handler}) -> Listener:",
                    quote(name)
                ));
                c.indent();
                c.pydoc(&on_doc(events));
                c.dedent();
            }
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
        if destroy {
            c.line("def destroy(self) -> None: ...");
        }
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
            self.named(c, &class_name(object), "get", true, None);
            self.def(
                c,
                &format!("def get(cls) -> {}", class_name(object)),
                "The service.",
                &[
                    format!("i = _ids_{snake_name}()"),
                    "return _B.wrap(_NS_NAME, _NS.create(i[0], []), cls)".to_owned(),
                ],
            );
            return Ok(());
        }
        let ctor = ctor_properties(object, slots)?;
        let mut params = vec!["self".to_owned()];
        let mut inits = Vec::new();
        for property in &ctor.positional {
            let param = member_name(&property.name, &None);
            params.push(format!("{param}: {}", self.ty(&property.ty, true)?));
            inits.push(format!(
                "(i[{}], {})",
                Self::at(lookups, 'p', &property.name),
                self.enc(&property.ty, &param)?
            ));
        }
        let mut lines = vec![format!("i = _ids_{snake_name}()")];
        if ctor.options.is_empty() {
            lines.push(format!(
                "_B.Object.__init__(self, _NS.create(i[0], [{}]))",
                inits.join(", ")
            ));
        } else {
            // The options are keyword-only, and each that is given is one more property the object is made with.
            params.push("*".to_owned());
            lines.push(format!("inits = [{}]", inits.join(", ")));
            for property in &ctor.options {
                let param = member_name(&property.name, &None);
                params.push(format!("{param}: {} | None = None", self.ty(&property.ty, true)?));
                lines.push(format!(
                    "if {param} is not None: inits.append((i[{}], {}))",
                    Self::at(lookups, 'p', &property.name),
                    self.enc(&property.ty, &param)?
                ));
            }
            lines.push("_B.Object.__init__(self, _NS.create(i[0], inits))".to_owned());
        }
        self.named(
            c,
            &class_name(object),
            "new",
            true,
            Some(&format!("_spell_{snake_name}")),
        );
        self.def(
            c,
            &format!("def __init__({}) -> None", params.join(", ")),
            if ctor.options.is_empty() {
                "Makes the object."
            } else {
                "Makes the object; each keyword sets the property of its name in the same call."
            },
            &lines,
        );
        Ok(())
    }

    fn property_source(
        &self,
        c: &mut CodeWriter,
        object: &ObjectType,
        snake_name: &str,
        lookups: &[MemberLookup],
        property: &Property,
        augment: bool,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let class_owned = class_name(object);
        let class = class_owned.as_str();
        let schema = self.view.member_doc(&object.name, &property.name);
        let ident = member_name(&property.name, &property.extras.export.py);
        let spell = format!("_spell_{snake_name}");
        claim(&ident)?;
        let at = Self::at(lookups, 'p', &property.name);
        let ids = format!("_ids_{snake_name}()[{at}]");
        let callback = matches!(property.ty, Type::Callback(_));
        let write_ty = self.ty(&property.ty, true)?;
        if callback {
            // Written, never read back: a plain attribute in a stub, a write-only property in the module.
            if self.stub {
                c.line(format!("{ident}: {write_ty}"));
                c.pydoc(schema);
            } else {
                self.named(c, class, &ident, false, Some(&spell));
                c.line(format!("def _set_{ident}(self, value):"));
                c.line(format!("    self._h.set({ids}, {})", self.enc(&property.ty, "value")?));
                c.line(format!("{ident} = builtins.property(fset=_set_{ident})"));
            }
            return Ok(());
        }
        // The member an augment adds to a class of another library is read as the object, as in TypeScript and Rust: it is the augmenting
        // library's own, and `T | None` is for what the library the object belongs to may not have.
        let read = self.ty_read(&property.ty, property.always || augment)?;
        c.line("@builtins.property");
        self.named(c, class, &ident, false, Some(&spell));
        // A property the catalog says the library `always` has an object for is read as one: a none answer is the library breaking its word,
        // and is refused where it is read, not handed to the script as `None` under a type that does not allow it.
        let body = if property.always && matches!(property.ty, Type::ObjectRef(_)) {
            let held = format!("_B.always(self._h.get({ids}), {})", quote(&format!("{class}.{ident}")));
            self.dec(&property.ty, &held, false)?
        } else {
            self.dec(&property.ty, &format!("self._h.get({ids})"), true)?
        };
        self.def_described(
            c,
            &format!("def {ident}(self) -> {read}"),
            schema,
            &format!("The property `{}`.", property.name),
            &[format!("return {body}")],
        );
        if !property.readonly {
            c.line(format!("@{ident}.setter"));
            self.named(c, class, &ident, false, Some(&spell));
            self.def(
                c,
                &format!("def {ident}(self, value: {write_ty}) -> None"),
                &format!("Sets the property `{}`.", property.name),
                &[format!("self._h.set({ids}, {})", self.enc(&property.ty, "value")?)],
            );
            if property.animated {
                let timed = format!("set_{ident}_timed");
                claim(&timed)?;
                self.named(c, class, &timed, false, Some(&spell));
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
        object: &ObjectType,
        snake_name: &str,
        lookups: &[MemberLookup],
        method: &Method,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let class_owned = class_name(object);
        let class = class_owned.as_str();
        let schema = self.view.member_doc(&object.name, &method.name);
        let ident = member_name(&method.name, &method.extras.export.py);
        let spell = format!("_spell_{snake_name}");
        claim(&ident)?;
        let (params, args) = self.params(&method.params)?;
        let at = Self::at(lookups, 'm', &method.name);
        let returns = match &method.returns {
            Some(ty) if method.optional => self.ty_read(ty, false)?,
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
            self.named(c, class, &ident, true, Some(&spell));
            self.def_described(c, &format!("def {ident}({params}) -> {returns}"), schema, &doc, &body);
        } else {
            let params = if params.is_empty() {
                "self".to_owned()
            } else {
                format!("self, {params}")
            };
            self.named(c, class, &ident, false, Some(&spell));
            self.def_described(c, &format!("def {ident}({params}) -> {returns}"), schema, &doc, &body);
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
            if self.stub { "(Implementor)" } else { "(_B.Implementor)" }
        ));
        c.indent();
        self.class_doc(c, self.view.class_doc(&object.name), || {
            format!(
                "The interface `{}` of `{}`: a script subclasses it and defines the methods it wants to answer.",
                object.name, self.view.namespace
            )
        });
        for method in &object.methods {
            let ident = member_name(&method.name, &method.extras.export.py);
            let (params, _) = self.params(&method.params)?;
            let params = if params.is_empty() {
                "self".to_owned()
            } else {
                format!("self, {params}")
            };
            let returns = match &method.returns {
                // The script's method may answer nothing, which the library reads as the method's comment says.
                Some(ty) if method.optional => format!("{} | None", self.ty(ty, true)?),
                Some(ty) => self.ty(ty, true)?,
                None => "None".to_owned(),
            };
            if self.stub {
                self.def_described(
                    c,
                    &format!("def {ident}({params}) -> {returns}"),
                    self.view.member_doc(&object.name, &method.name),
                    "",
                    &[],
                );
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
        self.named(c, &name, "implement", true, None);
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

impl Py<'_> {
    /// What installs the members of the class `class`, which holds an augment, on the class `target` of the core's module: each is read or
    /// called through the service of this library, made for the generation that is running when the member is used, so a member binds to
    /// the library of the generation and never to one that has retired.
    fn augment_source(&self, c: &mut CodeWriter, class: &ObjectType, target: &ObjectType) -> Result<(), String> {
        let name = class_name(class);
        let core = format!("_core.{}", class_name(target));
        c.line(format!(
            "# The members `{}` adds to `{}` of `gnoblin.objects.model`: they exist on it once this module is imported, and a shell that never imports it has none.",
            self.view.namespace,
            class_name(target)
        ));
        for property in &class.properties {
            let ident = member_name(&property.name, &property.extras.export.py);
            c.line(format!(
                "_B.augment_property({core}, {}, lambda _obj: {name}.get().{ident})",
                quote(&ident)
            ));
        }
        let getters = class.augment_getters.as_deref();
        for method in &class.methods {
            // A property's setter is installed with its getter, not as a method of its own.
            if is_augment_setter(class, &method.name) {
                continue;
            }
            let ident = member_name(&method.name, &method.extras.export.py);
            let line = match getters {
                Some(getters) if getters.contains(&method.name) => {
                    let write = if class.augment_setters.contains(&method.name) {
                        format!(
                            ", lambda obj, value: {name}.get().{}(obj, value)",
                            member_name(&format!("set_{}", method.name), &None)
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "_B.augment_property({core}, {}, lambda obj: {name}.get().{ident}(obj){write})",
                        quote(&ident)
                    )
                }
                Some(_) => format!(
                    "_B.augment_method({core}, {}, lambda obj, *args, **kwargs: {name}.get().{ident}(obj, *args, **kwargs))",
                    quote(&ident)
                ),
                None => format!(
                    "_B.augment_method({core}, {}, lambda _obj, *args, **kwargs: {name}.get().{ident}(*args, **kwargs))",
                    quote(&ident)
                ),
            };
            c.line(line);
        }
        // A finder the augment declares (`find = "Kind"`) is a kind of the core class's `find`.
        for (kind, method) in augment_finds(class) {
            let Some(Type::ObjectRef(query)) = method.params.first().map(|param| &param.ty) else {
                return Err(format!(
                    "`{}`: the finder of `{kind}` takes no filter record",
                    class.name
                ));
            };
            c.line(format!(
                "_B.augment_find({core}, {}, {}, {})",
                quote(kind),
                quote(&member_name(&method.name, &method.extras.export.py)),
                class_name(self.view.class(query)?)
            ));
        }
        // `set(props, options)` of the core class, over its own writable properties and those the augment adds.
        if class.assign {
            check_assign(class, Some(target), self.view.objects.values().copied())?;
            c.line(format!(
                "_B.augment_method({core}, \"set\", lambda obj, props, options=None: _B.assign(obj, {}, props, options, {}))",
                quote(&class_name(target)),
                self.assign_request()?
            ));
        }
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
        c.line("from typing import Any, Callable, Literal, TypeVar, overload");
        c.blank();
        c.line("from gnoblin.objects import Implementor as Implementor, Listener as Listener, Timing as Timing");
        for foreign in view.foreigns()? {
            c.line(format!(
                "from gnoblin.objects.{} import {} as {}",
                foreign.module,
                foreign.py,
                foreign.alias_py()
            ));
        }
        c.blank();
        c.line("_F = TypeVar(\"_F\", bound=Callable[..., object])");
        c.blank();
    } else {
        c.line("from __future__ import annotations");
        c.blank();
        c.line("import builtins");
        c.line("import dataclasses");
        c.line("import enum");
        c.line("from typing import Any, Callable");
        c.blank();
        c.line("from gnoblin.objects import _base as _B");
        c.line("from gnoblin.objects._base import Timing");
        for foreign in view.foreigns()? {
            c.line(format!(
                "from gnoblin.objects.{} import {} as {}",
                foreign.module,
                foreign.py,
                foreign.alias_py()
            ));
        }
        c.blank();
        c.line(format!("_NS_NAME = {}", quote(view.namespace.as_str())));
        c.line("_NS = _B.open(_NS_NAME)");
    }
    if !stub && !augments_of(view.catalog, &view.objects)?.is_empty() {
        c.line("from gnoblin.objects import model as _core");
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
    if !stub {
        for (class, target) in augments_of(view.catalog, &view.objects)? {
            py.augment_source(&mut c, class, target)?;
        }
    }
    Ok(c.finish())
}

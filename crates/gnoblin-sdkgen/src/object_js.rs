//! The JavaScript and TypeScript classes of a namespace served through the object protocol: `objects.js` and its `objects.d.ts`.
//!
//! The classes call `__gnoblinObjects` (`gnoblin-runtime-js`'s `object_native.rs`) through the small runtime in `object_base.js`. A value
//! crosses as the script's own: a record is a plain object (the native layer sees it as an array of fields, in the catalog's order), an enum
//! is its name, a handle is a class instance and a closure is a function. The three that need a mark when they go in (a record, an enum
//! value, an object) are wrapped by `B.rec`, `B.enm` and `B.ref`, which is the only place the encoding is known.

use std::collections::BTreeSet;

use gnoblin_schema::{CallbackDecl, DefaultValue, EnumDecl, Field, Method, ObjectKind, ObjectType, Property, Type};

use super::object_script::{ScriptView, quote};
use super::objects::{ClassMember, Member, check_foreign, slots_of};
use super::text::{CodeWriter, camel, pascal, snake, ts_ident};
use super::values::{self, Absent, CarrySlot, ValueClass, absent, is_family, js_field};

/// The runtime every generated module imports (`gnoblin-core/generated/object_base`).
pub(super) const BASE_JS: &str = r#"// The runtime the generated object classes share: the native layer, the handle of a library object and the encoding of the values that
// need a mark when they go in.
const native = globalThis.__gnoblinObjects;
if (native === undefined) {
  throw new Error(
    "the object classes need the JavaScript runtime library (libgnoblin-runtime-js): this shell host has no `__gnoblinObjects`",
  );
}

export { native };

/** The slot a library object is held in, on the object. */
export const HANDLE = Symbol("gnoblin.handle");

// A script's reference to a library object is dropped when the script's object is collected. The call is queued by the engine, so it
// runs between the script's own turns.
const finalizer =
  typeof FinalizationRegistry === "function" ? new FinalizationRegistry((slot) => native.release(slot)) : undefined;

const classes = new Map();

/** Opens the namespace the library serves, once for this generation. Throws when the library is not installed. */
export function open(namespace) {
  return native.open(namespace);
}

/** The ids of the class `name` and its members `names` (`p:text`, `m:insert`, `s:changed`), asked of the library the first time they are used. */
export function ids(namespace, name, names) {
  let found;
  return () => (found ??= native.ids(namespace, name, names));
}

/** Makes `instance` hold `slot`, and drop it when the instance is collected. */
export function adopt(instance, slot) {
  instance[HANDLE] = slot;
  if (finalizer !== undefined) {
    finalizer.register(instance, slot);
  }
  return instance;
}

/** Remembers the class a library object of the name `name` is read as. */
export function register(namespace, name, constructor) {
  classes.set(`${namespace}/${name}`, constructor);
}

/** An instance of the class the library says `slot` is, or of `fallback` when the script has no such class. */
export function wrap(namespace, slot, fallback) {
  const constructor = classes.get(`${namespace}/${native.classOf(slot)}`) ?? fallback;
  return adopt(Object.create(constructor.prototype), slot);
}

export const same = (value) => value;
export const opt = (value, convert) => (value === undefined || value === null ? undefined : convert(value));

export function need(value, what) {
  if (value === undefined || value === null) {
    throw new TypeError(`${what} is required`);
  }
  return value;
}

/** Checks the option bag of a value class: an object, and no field the class lacks. */
export function fields(init, label, known) {
  if (init === null || typeof init !== "object" || Array.isArray(init)) {
    throw new TypeError(`${label}: expects an object of fields`);
  }
  for (const key of Object.keys(init)) {
    if (!known.includes(key)) {
      throw new TypeError(`${label}: no field "${key}" (known: ${known.join(", ")})`);
    }
  }
}

/** A record on its way in: its fields, in the catalog's order. */
export const rec = (fields) => ({ $t: 1, $v: fields });

/** A member of an enum on its way in, from its name. */
export function enm(values, value, what) {
  const at = typeof value === "string" ? values.indexOf(value) : -1;
  if (at < 0) {
    throw new TypeError(`${JSON.stringify(value)} is not a value of ${what} (${values.join(", ")})`);
  }
  return { $t: 2, $v: at + 1 };
}

/** Bytes on their way in. */
export const bytes = (value) => ({ $t: 4, $v: Array.from(value) });

/** An object on its way in: a library object, or a script object that implements an interface (`implement` makes the library's side). */
export function ref(value, implement) {
  if (value === null || typeof value !== "object") {
    throw new TypeError(`expected an object, got ${value === null ? "null" : typeof value}`);
  }
  let slot = value[HANDLE];
  if (slot === undefined) {
    if (implement === undefined) {
      throw new TypeError("expected an object the library made");
    }
    slot = adopt(value, implement(value))[HANDLE];
  }
  return { $t: 3, $v: slot };
}

/** A function the library calls: `decoders` read its arguments and `encode` writes what it answers. */
export function callback(fn, decoders, encode) {
  if (typeof fn !== "function") {
    throw new TypeError(`expected a function, got ${typeof fn}`);
  }
  return (member, args) => encode(fn(...args.map((arg, at) => decoders[at](arg))));
}

/** A listener of an object's signal or property. It lasts until it is stopped or the generation ends, not until it is collected. */
export class Listener {
  #namespace;
  #registration;

  constructor(namespace, registration) {
    this.#namespace = namespace;
    this.#registration = registration;
  }

  /** Stops hearing. A second call does nothing. */
  stop() {
    if (this.#registration !== 0) {
      const registration = this.#registration;
      this.#registration = 0;
      native.unlisten(this.#namespace, registration);
    }
  }
}

/** Listens to `event` of `self`: `events` maps an event to its member's place in `ids()` and the decoders of its arguments. */
export function listen(self, namespace, ids, events, event, handler) {
  const spec = events[event];
  if (spec === undefined) {
    throw new TypeError(`no event ${JSON.stringify(event)}; the events are ${Object.keys(events).join(", ")}`);
  }
  if (typeof handler !== "function") {
    throw new TypeError(`expected a function, got ${typeof handler}`);
  }
  const [at, decoders] = spec;
  const registration = native.listen(self[HANDLE], ids()[at], (member, args) => {
    handler(...args.map((arg, k) => decoders[k](arg)));
  });
  return new Listener(namespace, registration);
}

/** Writes a property over a time. */
export function setTimed(self, member, value, timing) {
  native.setTimed(self[HANDLE], member, value, {
    seconds: need(timing?.seconds, "timing.seconds"),
    delay: timing.delay ?? 0,
    tween: timing.tween ?? 0,
    samples: timing.samples ?? [],
  });
}
"#;

/// The declarations of `object_base.js`.
pub(super) const BASE_DTS: &str = r#"// The parts of the object runtime a script sees.

/** A listener of an object's signal or property. It lasts until it is stopped or the generation ends. */
export declare class Listener {
  /** Stops hearing. A second call does nothing. */
  stop(): void;
}

/** How a property is written over a time. */
export interface Timing {
  /** The duration in seconds. */
  seconds: number;
  /** Seconds the start value is held before the change begins. */
  delay?: number;
  /** The `tween` node whose progress drives the write, when there is one. */
  tween?: number;
  /** The curve: progress from 0 to 1, spread evenly over the duration. */
  samples?: number[];
}
"#;

fn class_name(object: &ObjectType) -> String {
    values::js_class(&object.name, &object.export)
}

fn member_name(name: &str, export: &Option<String>) -> String {
    export.clone().unwrap_or_else(|| camel(name))
}

fn param_name(field: &Field) -> String {
    ts_ident(&camel(&field.name))
}

/// The renderer of one namespace.
struct Js<'a> {
    view: &'a ScriptView<'a>,
}

/// One entry of a class's id list: what the runtime is asked to look up.
struct MemberLookup {
    kind: char,
    name: String,
}

impl<'a> Js<'a> {
    /// A plain JavaScript expression for a schema default.
    fn default_expr(&self, default: &DefaultValue) -> String {
        match default {
            DefaultValue::Bool(value) => value.to_string(),
            DefaultValue::Int(value) => value.to_string(),
            DefaultValue::Uint(value) => value.to_string(),
            DefaultValue::Double(value) => value.to_string(),
            DefaultValue::Text(value) => quote(value),
            DefaultValue::List(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| self.default_expr(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// The script value `e` as the value that goes in.
    fn enc(&self, ty: &Type, e: &str) -> Result<String, String> {
        Ok(match ty {
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text => e.to_owned(),
            Type::Bytes => format!("B.bytes({e})"),
            Type::Enum(name) => format!("enc_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                match class.kind {
                    ObjectKind::Record => format!("enc_rec_{}({e})", snake(&class.name)),
                    ObjectKind::Interface => format!("B.ref({e}, implement_{})", snake(&class.name)),
                    _ => format!("B.ref({e})"),
                }
            }
            Type::List(inner) => {
                let item = self.enc(inner, "item")?;
                if item == "item" {
                    e.to_owned()
                } else {
                    format!("{e}.map((item) => {item})")
                }
            }
            Type::Callback(name) => format!("cb_{}({e})", snake(&self.view.callback(name)?.name)),
        })
    }

    /// What the library sent as `e`, as the script value. `absent` says an object or a record may be none (a property's value is, a
    /// call's answer is not).
    fn dec(&self, ty: &Type, e: &str, absent: bool) -> Result<String, String> {
        Ok(match ty {
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes => e.to_owned(),
            Type::Enum(name) => format!("dec_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                let (function, call) = match class.kind {
                    ObjectKind::Record => (
                        format!("dec_rec_{}", snake(&class.name)),
                        format!("dec_rec_{}({e})", snake(&class.name)),
                    ),
                    _ => (
                        format!("(slot) => B.wrap(NS, slot, {})", class_name(class)),
                        format!("B.wrap(NS, {e}, {})", class_name(class)),
                    ),
                };
                if absent {
                    format!("B.opt({e}, {function})")
                } else {
                    call
                }
            }
            Type::List(inner) => {
                let item = self.dec(inner, "item", false)?;
                if item == "item" {
                    e.to_owned()
                } else {
                    format!("{e}.map((item) => {item})")
                }
            }
            Type::Callback(name) => {
                return Err(format!(
                    "`callback:{name}` can be given to the library, not read back from it"
                ));
            }
        })
    }

    /// `dec` as a function, for a table of decoders.
    fn decoder(&self, ty: &Type, absent: bool) -> Result<String, String> {
        let body = self.dec(ty, "value", absent)?;
        Ok(if body == "value" {
            "B.same".to_owned()
        } else {
            format!("(value) => {body}")
        })
    }

    fn enum_source(&self, c: &mut CodeWriter, declared: &EnumDecl) {
        let name = snake(&declared.name);
        let values: Vec<String> = declared.values.iter().map(|value| quote(value)).collect();
        c.line(format!("const enum_{name} = [{}];", values.join(", ")));
        c.line(format!(
            "function enc_enum_{name}(value) {{ return B.enm(enum_{name}, value, {}); }}",
            quote(&declared.name)
        ));
        c.line(format!(
            "function dec_enum_{name}(value) {{ return enum_{name}[value - 1]; }}"
        ));
        c.blank();
    }

    fn callback_source(&self, c: &mut CodeWriter, declared: &CallbackDecl) -> Result<(), String> {
        let name = snake(&declared.name);
        let decoders: Vec<String> = declared
            .params
            .iter()
            .map(|field| self.decoder(&field.ty, false))
            .collect::<Result<_, _>>()?;
        let encode = match &declared.returns {
            Some(ty) => format!("(value) => {}", self.enc(ty, "value")?),
            None => "B.same".to_owned(),
        };
        c.line(format!(
            "function cb_{name}(fn) {{ return B.callback(fn, [{}], {encode}); }}",
            decoders.join(", ")
        ));
        c.blank();
        Ok(())
    }

    /// The classes and the conversion of a record: one value class and its codec, or for the family of a tagged value, a class and a
    /// codec for each variant and the codec that tells them apart.
    fn record_source(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        if !object.methods.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the record `{}` declares members that are not properties",
                object.name
            ));
        }
        for class in values::value_classes([object]) {
            self.class_source(c, &class);
            match class.of {
                None => self.record_codec(c, object)?,
                Some((family, variant)) => self.variant_codec(c, &class, family, variant)?,
            }
        }
        if is_family(object) {
            self.family_codec(c, object)?;
        }
        Ok(())
    }

    /// A value class: the fields in the schema's order, each taken from the option bag or started at its default.
    fn class_source(&self, c: &mut CodeWriter, class: &ValueClass<'_>) {
        let name = class.js();
        let known: Vec<String> = class
            .properties
            .iter()
            .map(|property| quote(&js_field(property)))
            .collect();
        c.line(format!("/** The value class `{}`. */", class.name));
        c.line(format!("export class {name} {{"));
        c.indent();
        c.line("constructor(init = {}) {");
        c.indent();
        c.line(format!("B.fields(init, {}, [{}]);", quote(&name), known.join(", ")));
        for property in class.properties {
            let field = js_field(property);
            let value = match absent(property) {
                Absent::Default(default) => format!("init.{field} ?? {}", self.default_expr(default)),
                Absent::None => format!("init.{field}"),
                Absent::Required => format!("B.need(init.{field}, {})", quote(&format!("{name}.{field}"))),
            };
            c.line(format!("this.{field} = {value};"));
        }
        c.dedent();
        c.line("}");
        c.dedent();
        c.line("}");
        c.blank();
    }

    /// The expression that makes the library's value of `property` from the script's `given`.
    fn field_in(&self, owner: &str, property: &Property, given: &str) -> Result<String, String> {
        Ok(match absent(property) {
            Absent::Default(default) => {
                self.enc(&property.ty, &format!("({given} ?? {})", self.default_expr(default)))?
            }
            Absent::None => format!("B.opt({given}, (value) => {})", self.enc(&property.ty, "value")?),
            Absent::Required => self.enc(
                &property.ty,
                &format!("B.need({given}, {})", quote(&format!("{owner}.{}", js_field(property)))),
            )?,
        })
    }

    /// The script value of `property` from the library's `read`. A field the sender may leave out is read through `B.opt`, and the class's
    /// constructor makes it its default.
    fn field_out(&self, property: &Property, read: &str) -> Result<String, String> {
        let decoded = self.dec(&property.ty, read, false)?;
        Ok(if decoded == read || matches!(absent(property), Absent::Required) {
            decoded
        } else {
            format!("B.opt({read}, (value) => {})", self.dec(&property.ty, "value", false)?)
        })
    }

    fn record_codec(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        let name = snake(&object.name);
        let class = class_name(object);
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        for (index, property) in object.properties.iter().enumerate() {
            let field = js_field(property);
            writes.push(self.field_in(&class, property, &format!("record.{field}"))?);
            reads.push(format!(
                "{field}: {}",
                self.field_out(property, &format!("fields[{index}]"))?
            ));
        }
        c.line(format!("function enc_rec_{name}(record) {{"));
        c.indent();
        c.line("if (record === null || typeof record !== \"object\") {");
        c.line(format!(
            "  throw new TypeError(`expected a {class} object, got ${{record === null ? \"null\" : typeof record}}`);"
        ));
        c.line("}");
        c.line(format!("return B.rec([{}]);", writes.join(", ")));
        c.dedent();
        c.line("}");
        c.line(format!("function dec_rec_{name}(fields) {{"));
        c.line(format!("  return new {class}({{ {} }});", reads.join(", ")));
        c.line("}");
        c.blank();
        Ok(())
    }

    /// The conversion of one variant to and from the family's record: the record's fields in the record's order.
    fn variant_codec(
        &self,
        c: &mut CodeWriter,
        class: &ValueClass<'_>,
        family: &ObjectType,
        variant: &gnoblin_schema::TaggedVariant,
    ) -> Result<(), String> {
        let name = snake(class.name);
        let js_name = class.js();
        let slots = values::slots(&|record| self.view.class(record), family, variant)?;
        let mut writes = Vec::new();
        let mut setup = Vec::new();
        let mut reads = Vec::new();
        for (index, slot) in slots.iter().enumerate() {
            match &slot.slot {
                CarrySlot::Tag(tag) => writes.push(self.enc(&slot.field.ty, &quote(tag))?),
                CarrySlot::Whole(own) => {
                    writes.push(self.field_in(&js_name, own, &format!("record.{}", js_field(own)))?);
                    reads.push(format!(
                        "{}: {}",
                        js_field(own),
                        self.field_out(own, &format!("fields[{index}]"))?
                    ));
                }
                CarrySlot::Part { record, parts } => {
                    let record_class = class_name(record);
                    let given: Vec<String> = parts
                        .iter()
                        .map(|carried| {
                            let own = format!("record.{}", js_field(carried.own));
                            let value = match absent(carried.own) {
                                Absent::Default(default) => format!("{own} ?? {}", self.default_expr(default)),
                                _ => own,
                            };
                            format!("{}: {value}", js_field(carried.inner))
                        })
                        .collect();
                    writes.push(format!(
                        "enc_rec_{}(new {record_class}({{ {} }}))",
                        snake(&record.name),
                        given.join(", ")
                    ));
                    let held = format!("part_{}", snake(&slot.field.name));
                    setup.push(format!(
                        "const {held} = dec_rec_{}(B.need(fields[{index}], {}));",
                        snake(&record.name),
                        quote(&format!("{}.{}", class_name(family), js_field(slot.field)))
                    ));
                    for carried in parts {
                        reads.push(format!("{}: {held}.{}", js_field(carried.own), js_field(carried.inner)));
                    }
                }
                CarrySlot::Empty => writes.push("undefined".to_owned()),
            }
        }
        c.line(format!("function enc_var_{name}(record) {{"));
        c.line(format!("  return B.rec([{}]);", writes.join(", ")));
        c.line("}");
        c.line(format!("function dec_var_{name}(fields) {{"));
        c.indent();
        for line in &setup {
            c.line(line);
        }
        c.line(format!("return new {js_name}({{ {} }});", reads.join(", ")));
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    /// The conversion of a family: a script's value is one of its variants' classes, and the library's record says by its tag which.
    fn family_codec(&self, c: &mut CodeWriter, family: &ObjectType) -> Result<(), String> {
        let name = snake(&family.name);
        let class = class_name(family);
        let tag = family
            .properties
            .iter()
            .position(|property| Some(property.name.as_str()) == family.tag.as_deref())
            .ok_or_else(|| format!("the family `{}` has no `tag` property", family.name))?;
        let classes: Vec<String> = family
            .variants
            .iter()
            .map(|variant| values::js_class(&variant.name, &variant.export))
            .collect();
        c.line(format!("function enc_rec_{name}(record) {{"));
        c.indent();
        for (variant, js_name) in family.variants.iter().zip(&classes) {
            c.line(format!(
                "if (record instanceof {js_name}) return enc_var_{}(record);",
                snake(&variant.name)
            ));
        }
        c.line(format!(
            "throw new TypeError(`expected a {class} (one of {}), got ${{record === null ? \"null\" : typeof record === \"object\" ? record.constructor.name : typeof record}}`);",
            classes.join(", ")
        ));
        c.dedent();
        c.line("}");
        c.line(format!("function dec_rec_{name}(fields) {{"));
        c.indent();
        c.line(format!(
            "const kind = {};",
            self.dec(&family.properties[tag].ty, &format!("fields[{tag}]"), false)?
        ));
        c.line("switch (kind) {");
        for variant in &family.variants {
            c.line(format!(
                "  case {}: return dec_var_{}(fields);",
                quote(&variant.tag),
                snake(&variant.name)
            ));
        }
        c.line(format!(
            "  default: throw new TypeError(`the library sent a {class} of the kind ${{kind}}, which this SDK has no class for`);"
        ));
        c.line("}");
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    /// The names a class asks the library for, in the order `ids()` answers (the class first).
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

    /// The place of the member `name` of kind `kind` in a class's `ids()` (the class is 0).
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
            "const ids_{} = B.ids(NS, {}, [{}]);",
            snake(&object.name),
            quote(&object.name),
            names.join(", ")
        ));
    }

    /// The events of a class: its signals, and `<property>Changed` for each property.
    fn events(slots: &[ClassMember<'_>]) -> Vec<(String, MemberLookup, Vec<Field>, Option<Property>)> {
        let mut events = Vec::new();
        for slot in slots {
            match &slot.member {
                Member::Signal(signal) => events.push((
                    member_name(&signal.name, &signal.extras.export.js),
                    MemberLookup {
                        kind: 's',
                        name: signal.name.clone(),
                    },
                    signal.fields.clone(),
                    None,
                )),
                Member::Property(property) if !matches!(property.ty, Type::Callback(_)) => events.push((
                    format!("{}Changed", member_name(&property.name, &property.extras.export.js)),
                    MemberLookup {
                        kind: 'p',
                        name: property.name.clone(),
                    },
                    Vec::new(),
                    Some((*property).clone()),
                )),
                _ => {}
            }
        }
        // A signal the catalog names `title_changed` is the event of that name: the property's own change is not told twice under it.
        let signals: BTreeSet<String> = events
            .iter()
            .filter(|(_, lookup, _, _)| lookup.kind == 's')
            .map(|(name, ..)| name.clone())
            .collect();
        events.retain(|(name, lookup, _, _)| lookup.kind == 's' || !signals.contains(name));
        events
    }

    fn params_source(&self, fields: &[Field]) -> Result<(String, Vec<String>), String> {
        let names: Vec<String> = fields.iter().map(param_name).collect();
        let encoded = fields
            .iter()
            .zip(&names)
            .map(|(field, name)| {
                if field.optional {
                    Ok(format!("B.opt({name}, (value) => {})", self.enc(&field.ty, "value")?))
                } else {
                    self.enc(&field.ty, name)
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((names.join(", "), encoded))
    }

    fn handle_source(&self, c: &mut CodeWriter, object: &'a ObjectType) -> Result<(), String> {
        let name = class_name(object);
        let snake_name = snake(&object.name);
        let slots = slots_of(self.view.catalog, &self.view.objects, object);
        for slot in &slots {
            check_foreign(&object.name, slot)?;
        }
        let lookups = Self::lookups(&slots);
        self.ids_source(c, object, &lookups);
        let events = Self::events(&slots);
        if !events.is_empty() {
            c.line(format!("const events_{snake_name} = {{"));
            c.indent();
            for (event, lookup, fields, property) in &events {
                let decoders: Vec<String> = match property {
                    Some(property) => vec![self.decoder(&property.ty, true)?],
                    None => fields
                        .iter()
                        .map(|field| self.decoder(&field.ty, false))
                        .collect::<Result<_, _>>()?,
                };
                c.line(format!(
                    "{event}: [{}, [{}]],",
                    Self::at(&lookups, lookup.kind, &lookup.name),
                    decoders.join(", ")
                ));
            }
            c.dedent();
            c.line("};");
        }
        let has_instance_members = object.kind == ObjectKind::Handle
            || slots.iter().any(|slot| match &slot.member {
                Member::Method(method) => !method.function,
                _ => true,
            });
        let mut taken: BTreeSet<String> = ["constructor", "on", "destroy"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        if object.kind == ObjectKind::Service && has_instance_members {
            taken.insert("static get".to_owned());
        }
        let mut claim = |ident: &str, statik: bool| -> Result<(), String> {
            if taken.insert(format!("{}{ident}", if statik { "static " } else { "" })) {
                Ok(())
            } else {
                Err(format!(
                    "`{}`: two members would both be `{ident}` in JavaScript",
                    object.name
                ))
            }
        };

        c.line(format!(
            "/** The class `{}` of `{}`. */",
            object.name, self.view.namespace
        ));
        c.line(format!("export class {name} {{"));
        c.indent();
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
        if has_instance_members {
            if !events.is_empty() {
                c.line("/** Calls `handler` each time `event` happens, until the listener is stopped. */");
                c.block(
                    "on(event, handler) {",
                    |c| {
                        c.line(format!(
                            "return B.listen(this, NS, ids_{snake_name}, events_{snake_name}, event, handler);"
                        ))
                    },
                    "}",
                );
                if object.name == "gnoblin" {
                    // The root object also takes its signals as named subscriptions, as the TypeScript `gnoblin` did
                    // (`gnoblin.onClientCreated(handler)`): the same as `on`, and it returns the same listener.
                    for (event, lookup, _, _) in events.iter().filter(|(_, lookup, _, _)| lookup.kind == 's') {
                        let method = format!("on{}", pascal(&lookup.name));
                        c.line(format!(
                            "/** Calls `handler` each time `{event}` happens: the same as `on({}, handler)`. */",
                            quote(event)
                        ));
                        c.block(
                            format!("{method}(handler) {{"),
                            |c| c.line(format!("return this.on({}, handler);", quote(event))),
                            "}",
                        );
                    }
                }
            }
            c.line("/** Ends the object and its children, whoever else holds it. */");
            c.block("destroy() {", |c| c.line("N.destroy(this[B.HANDLE]);"), "}");
        }
        c.dedent();
        c.line("}");
        c.line(format!("B.register(NS, {}, {name});", quote(&object.name)));
        c.blank();
        Ok(())
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
            c.line("/** The service. */");
            c.block(
                "static get() {",
                |c| {
                    c.line(format!("const i = ids_{snake_name}();"));
                    c.line(format!(
                        "return B.adopt(Object.create({}.prototype), N.create(NS, i[0], []));",
                        class_name(object)
                    ));
                },
                "}",
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
        c.line("/** Makes the object. */");
        match &object.ctor {
            Some(names) => {
                let mut params = Vec::new();
                let mut inits = Vec::new();
                for wanted in names {
                    let property = property_named(wanted)?;
                    let param = ts_ident(&camel(&property.name));
                    inits.push(format!(
                        "[i[{}], {}]",
                        Self::at(lookups, 'p', &property.name),
                        self.enc(&property.ty, &param)?
                    ));
                    params.push(param);
                }
                c.block(
                    format!("constructor({}) {{", params.join(", ")),
                    |c| {
                        c.line(format!("const i = ids_{snake_name}();"));
                        c.line(format!("B.adopt(this, N.create(NS, i[0], [{}]));", inits.join(", ")));
                    },
                    "}",
                );
            }
            None => {
                let writable: Vec<&Property> = slots
                    .iter()
                    .filter(|slot| !slot.foreign)
                    .filter_map(|slot| match &slot.member {
                        Member::Property(property) if !property.readonly => Some(*property),
                        _ => None,
                    })
                    .collect();
                let mut lines = Vec::new();
                for property in writable {
                    let field = member_name(&property.name, &property.extras.export.js);
                    lines.push(format!(
                        "if (options.{field} !== undefined) inits.push([i[{}], {}]);",
                        Self::at(lookups, 'p', &property.name),
                        self.enc(&property.ty, &format!("options.{field}"))?
                    ));
                }
                c.block(
                    "constructor(options = {}) {",
                    |c| {
                        c.line(format!("const i = ids_{snake_name}();"));
                        c.line("const inits = [];");
                        for line in &lines {
                            c.line(line);
                        }
                        c.line("B.adopt(this, N.create(NS, i[0], inits));");
                    },
                    "}",
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
        claim: &mut impl FnMut(&str, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&property.name, &property.extras.export.js);
        let at = Self::at(lookups, 'p', &property.name);
        let ids = format!("ids_{snake_name}()[{at}]");
        if !matches!(property.ty, Type::Callback(_)) {
            claim(&ident, false)?;
            let read = self.dec(&property.ty, &format!("N.get(this[B.HANDLE], {ids})"), true)?;
            c.line(format!("/** The property `{}`. */", property.name));
            c.block(format!("get {ident}() {{"), |c| c.line(format!("return {read};")), "}");
        } else {
            claim(&ident, false)?;
        }
        if !property.readonly {
            let write = self.enc(&property.ty, "value")?;
            c.block(
                format!("set {ident}(value) {{"),
                |c| c.line(format!("N.set(this[B.HANDLE], {ids}, {write});")),
                "}",
            );
            if property.animated {
                let timed = format!("set{}Timed", pascal(&ident));
                claim(&timed, false)?;
                c.line(format!("/** Sets the property `{}` over a time. */", property.name));
                c.block(
                    format!("{timed}(value, timing) {{"),
                    |c| c.line(format!("B.setTimed(this, {ids}, {write}, timing);")),
                    "}",
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
        claim: &mut impl FnMut(&str, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&method.name, &method.extras.export.js);
        claim(&ident, method.function)?;
        let (params, args) = self.params_source(&method.params)?;
        let at = Self::at(lookups, 'm', &method.name);
        let call = if method.function {
            format!("N.callStatic(NS, i[0], i[{at}], [{}])", args.join(", "))
        } else {
            format!("N.call(this[B.HANDLE], i[{at}], [{}])", args.join(", "))
        };
        let body = match &method.returns {
            Some(ty) => format!("return {};", self.dec(ty, &call, method.optional)?),
            None => format!("{call};"),
        };
        c.line(format!(
            "/** The {} `{}`. */",
            if method.function { "function" } else { "method" },
            method.name
        ));
        c.block(
            format!("{}{ident}({params}) {{", if method.function { "static " } else { "" }),
            |c| {
                c.line(format!("const i = ids_{snake_name}();"));
                c.line(body);
            },
            "}",
        );
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
        self.ids_source(c, object, &lookups);
        c.line(format!(
            "/** The interface `{}` of `{}`: a script extends it and defines the methods it wants to answer. */",
            object.name, self.view.namespace
        ));
        c.line(format!("export class {name} {{}}"));
        c.line(format!("function implement_{snake_name}(object) {{"));
        c.indent();
        c.line(format!("const i = ids_{snake_name}();"));
        c.line("const implemented = [];");
        let mut arms = Vec::new();
        for method in &object.methods {
            let ident = member_name(&method.name, &method.extras.export.js);
            let at = Self::at(&lookups, 'm', &method.name);
            c.line(format!(
                "if (typeof object.{ident} === \"function\") implemented.push(i[{at}]);"
            ));
            let reads: Vec<String> = method
                .params
                .iter()
                .enumerate()
                .map(|(index, field)| self.dec(&field.ty, &format!("args[{index}]"), false))
                .collect::<Result<_, _>>()?;
            let call = format!("object.{ident}({})", reads.join(", "));
            let answer = match &method.returns {
                Some(ty) => self.enc(ty, &call)?,
                None => call,
            };
            arms.push(format!("if (member === i[{at}]) return {answer};"));
        }
        c.line("return N.implement(NS, i[0], implemented, (member, args) => {");
        c.indent();
        for arm in &arms {
            c.line(arm);
        }
        c.line("throw new Error(\"the library called a method this interface does not have\");");
        c.dedent();
        c.line("});");
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }
}

/// The classes of `view`'s namespace as a JavaScript module.
pub(super) fn render_js(view: &ScriptView<'_>) -> Result<String, String> {
    let js = Js { view };
    let mut c = CodeWriter::new("  ");
    c.line(format!(
        "// The classes of `{}`, over the object protocol.",
        view.namespace
    ));
    c.line("import * as B from \"gnoblin-core/generated/object_base\";");
    c.blank();
    c.line(format!("const NS = B.open({});", quote(view.namespace.as_str())));
    c.line("const N = B.native;");
    c.blank();
    for declared in view.enums.values() {
        js.enum_source(&mut c, declared);
    }
    for declared in view.callbacks.values() {
        js.callback_source(&mut c, declared)?;
    }
    for object in view.objects.values() {
        match object.kind {
            ObjectKind::Record => js.record_source(&mut c, object)?,
            ObjectKind::Handle | ObjectKind::Service => js.handle_source(&mut c, object)?,
            ObjectKind::Interface => js.interface_source(&mut c, object)?,
        }
    }
    if let Some(object) = view
        .objects
        .values()
        .find(|object| object.name == "gnoblin" && object.kind == ObjectKind::Service)
    {
        c.line(
            "/** The service `gnoblin`, made once: the root of the object model, which a shell reads as `gnoblin`. */",
        );
        c.line(format!("export const gnoblin = {}.get();", class_name(object)));
    }
    Ok(c.finish())
}

// ---- the declarations ------------------------------------------------------------------------------------------------------

impl Js<'_> {
    /// The TypeScript type of `ty`. `input` is the form a parameter takes: a record may leave its defaulted fields out.
    fn ts(&self, ty: &Type, input: bool) -> Result<String, String> {
        Ok(match ty {
            Type::Bool => "boolean".to_owned(),
            Type::Int | Type::Uint | Type::Double => "number".to_owned(),
            Type::Text => "string".to_owned(),
            Type::Bytes => "Uint8Array".to_owned(),
            Type::Enum(name) => pascal(&self.view.enum_decl(name)?.name),
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                let base = class_name(class);
                if input && class.kind == ObjectKind::Record && has_init(class) {
                    format!("{base}Init")
                } else {
                    base
                }
            }
            Type::List(inner) => {
                let item = self.ts(inner, input)?;
                if item.contains(' ') {
                    format!("({item})[]")
                } else {
                    format!("{item}[]")
                }
            }
            Type::Callback(name) => {
                let declared = self.view.callback(name)?;
                let params: Vec<String> = declared
                    .params
                    .iter()
                    .map(|field| {
                        Ok(format!(
                            "{}{}: {}",
                            param_name(field),
                            if field.optional { "?" } else { "" },
                            self.ts(&field.ty, false)?
                        ))
                    })
                    .collect::<Result<_, String>>()?;
                let returns = match &declared.returns {
                    Some(ty) => self.ts(ty, true)?,
                    None => "void".to_owned(),
                };
                format!("({}) => {returns}", params.join(", "))
            }
        })
    }

    /// The type a property is read as: an object or a record may be absent.
    fn ts_read(&self, ty: &Type) -> Result<String, String> {
        let text = self.ts(ty, false)?;
        Ok(if matches!(ty, Type::ObjectRef(_)) {
            format!("{text} | undefined")
        } else {
            text
        })
    }

    fn record_dts(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        for class in values::value_classes([object]) {
            self.class_dts(c, &class)?;
        }
        if is_family(object) {
            let classes: Vec<String> = object
                .variants
                .iter()
                .map(|variant| values::js_class(&variant.name, &variant.export))
                .collect();
            c.line(format!(
                "/** The record `{}`: any one of its variants, told apart by its class. */",
                object.name
            ));
            c.line(format!("export type {} = {};", class_name(object), classes.join(" | ")));
            c.blank();
        }
        Ok(())
    }

    /// A value class: its fields as the class holds them, and the option bag its constructor takes, where a field that has a default (or
    /// is optional) may be left out.
    fn class_dts(&self, c: &mut CodeWriter, class: &ValueClass<'_>) -> Result<(), String> {
        let name = class.js();
        c.line(format!("/** The value class `{}`. */", class.name));
        c.line(format!("export class {name} {{"));
        c.indent();
        for property in class.properties {
            let field = js_field(property);
            let ty = self.ts(&property.ty, false)?;
            if property.optional {
                c.line(format!("{field}?: {ty} | undefined;"));
            } else {
                c.line(format!("{field}: {ty};"));
            }
        }
        let all_optional = class
            .properties
            .iter()
            .all(|property| property.default.is_some() || property.optional);
        c.line(format!(
            "constructor(init{}: {name}Init);",
            if all_optional { "?" } else { "" }
        ));
        c.dedent();
        c.line("}");
        c.line(format!(
            "/** What a caller gives for a `{name}`: the fields that have a default may be left out. */"
        ));
        c.line(format!("export interface {name}Init {{"));
        c.indent();
        for property in class.properties {
            let field = js_field(property);
            let ty = self.ts(&property.ty, true)?;
            if property.default.is_some() || property.optional {
                c.line(format!(
                    "{field}?: {ty}{};",
                    if property.optional { " | undefined" } else { "" }
                ));
            } else {
                c.line(format!("{field}: {ty};"));
            }
        }
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    fn params_dts(&self, fields: &[Field]) -> Result<String, String> {
        let params: Vec<String> = fields
            .iter()
            .map(|field| {
                Ok(format!(
                    "{}{}: {}",
                    param_name(field),
                    if field.optional { "?" } else { "" },
                    self.ts(&field.ty, true)?
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(params.join(", "))
    }

    fn handle_dts(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        let name = class_name(object);
        let slots = slots_of(self.view.catalog, &self.view.objects, object);
        for slot in &slots {
            check_foreign(&object.name, slot)?;
        }
        let events = Self::events(&slots);
        let has_instance_members = object.kind == ObjectKind::Handle
            || slots.iter().any(|slot| match &slot.member {
                Member::Method(method) => !method.function,
                _ => true,
            });
        if !events.is_empty() {
            c.line(format!(
                "/** The events of `{name}`: its signals, and `<property>Changed` for each property. */"
            ));
            c.line(format!("export interface {name}Events {{"));
            c.indent();
            for (event, _, fields, property) in &events {
                let args: Vec<String> = match property {
                    Some(property) => vec![format!(
                        "{}: {}",
                        ts_ident(&camel(&property.name)),
                        self.ts_read(&property.ty)?
                    )],
                    None => fields
                        .iter()
                        .map(|field| {
                            Ok(format!(
                                "{}{}: {}",
                                param_name(field),
                                if field.optional { "?" } else { "" },
                                self.ts(&field.ty, false)?
                            ))
                        })
                        .collect::<Result<_, String>>()?,
                };
                c.line(format!("{event}: [{}];", args.join(", ")));
            }
            c.dedent();
            c.line("}");
        }
        c.line(format!(
            "/** The class `{}` of `{}`. */",
            object.name, self.view.namespace
        ));
        c.line(format!("export declare class {name} {{"));
        c.indent();
        if has_instance_members {
            match (&object.kind, &object.ctor) {
                (ObjectKind::Service, _) => c.line(format!("static get(): {name};")),
                (_, Some(names)) => {
                    let mut params = Vec::new();
                    for wanted in names {
                        let property = slots
                            .iter()
                            .find_map(|slot| match &slot.member {
                                Member::Property(property) if &property.name == wanted => Some(*property),
                                _ => None,
                            })
                            .ok_or_else(|| {
                                format!(
                                    "`{}`: the constructor names `{wanted}`, which is not a property",
                                    object.name
                                )
                            })?;
                        params.push(format!(
                            "{}: {}",
                            ts_ident(&camel(&property.name)),
                            self.ts(&property.ty, true)?
                        ));
                    }
                    c.line(format!("constructor({});", params.join(", ")));
                }
                (_, None) => {
                    let mut fields = Vec::new();
                    for slot in slots.iter().filter(|slot| !slot.foreign) {
                        if let Member::Property(property) = &slot.member
                            && !property.readonly
                        {
                            fields.push(format!(
                                "{}?: {}",
                                member_name(&property.name, &property.extras.export.js),
                                self.ts(&property.ty, true)?
                            ));
                        }
                    }
                    c.line(format!("constructor(options?: {{ {} }});", fields.join("; ")));
                }
            }
        } else {
            c.line("private constructor();");
        }
        for slot in &slots {
            match &slot.member {
                Member::Property(property) => self.property_dts(c, property)?,
                Member::Method(method) => {
                    let ident = member_name(&method.name, &method.extras.export.js);
                    let returns = match &method.returns {
                        Some(ty) if method.optional => self.ts_read(ty)?,
                        Some(ty) => self.ts(ty, false)?,
                        None => "void".to_owned(),
                    };
                    c.line(format!(
                        "{}{ident}({}): {returns};",
                        if method.function { "static " } else { "" },
                        self.params_dts(&method.params)?
                    ));
                }
                Member::Signal(_) => {}
            }
        }
        if has_instance_members {
            if !events.is_empty() {
                c.line(format!(
                    "on<K extends keyof {name}Events>(event: K, handler: (...args: {name}Events[K]) => void): Listener;"
                ));
                if object.name == "gnoblin" {
                    for (event, lookup, _, _) in events.iter().filter(|(_, lookup, _, _)| lookup.kind == 's') {
                        c.line(format!(
                            "on{}(handler: (...args: {name}Events[{}]) => void): Listener;",
                            pascal(&lookup.name),
                            quote(event)
                        ));
                    }
                }
            }
            c.line("destroy(): void;");
        }
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    fn property_dts(&self, c: &mut CodeWriter, property: &Property) -> Result<(), String> {
        let ident = member_name(&property.name, &property.extras.export.js);
        let callback = matches!(property.ty, Type::Callback(_));
        let read = if callback {
            String::new()
        } else {
            self.ts_read(&property.ty)?
        };
        let write = self.ts(&property.ty, true)?;
        if property.readonly {
            c.line(format!("readonly {ident}: {read};"));
            return Ok(());
        }
        if callback {
            c.line(format!("set {ident}(value: {write});"));
        } else if read == write {
            c.line(format!("{ident}: {read};"));
        } else {
            c.line(format!("get {ident}(): {read};"));
            c.line(format!("set {ident}(value: {write});"));
        }
        if property.animated {
            c.line(format!(
                "set{}Timed(value: {write}, timing: Timing): void;",
                pascal(&ident)
            ));
        }
        Ok(())
    }

    fn interface_dts(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        let name = class_name(object);
        c.line(format!(
            "/** The interface `{}` of `{}`: a script extends it and defines the methods it wants to answer. */",
            object.name, self.view.namespace
        ));
        c.line(format!("export declare abstract class {name} {{"));
        c.indent();
        for method in &object.methods {
            let ident = member_name(&method.name, &method.extras.export.js);
            let returns = match &method.returns {
                Some(ty) => self.ts(ty, true)?,
                None => "void".to_owned(),
            };
            if method.required {
                c.line(format!(
                    "abstract {ident}({}): {returns};",
                    self.params_dts(&method.params)?
                ));
            } else {
                c.line(format!("{ident}?({}): {returns};", self.params_dts(&method.params)?));
            }
        }
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }
}

/// Whether a record has a field with a default, so a parameter takes its `Init` form (an optional field is optional in both).
fn has_init(object: &ObjectType) -> bool {
    !is_family(object) && object.properties.iter().any(|property| property.default.is_some())
}

/// The declarations of `view`'s namespace.
pub(super) fn render_dts(view: &ScriptView<'_>) -> Result<String, String> {
    let js = Js { view };
    let mut c = CodeWriter::new("  ");
    c.line(format!(
        "// The classes of `{}`, over the object protocol.",
        view.namespace
    ));
    c.line("import type { Listener, Timing } from \"gnoblin-core/generated/object_base\";");
    c.blank();
    for declared in view.enums.values() {
        let values: Vec<String> = declared.values.iter().map(|value| quote(value)).collect();
        c.line(format!("/** The enum `{}`. */", declared.name));
        c.line(format!(
            "export type {} = {};",
            pascal(&declared.name),
            values.join(" | ")
        ));
    }
    c.blank();
    for object in view.objects.values() {
        match object.kind {
            ObjectKind::Record => js.record_dts(&mut c, object)?,
            ObjectKind::Handle | ObjectKind::Service => js.handle_dts(&mut c, object)?,
            ObjectKind::Interface => js.interface_dts(&mut c, object)?,
        }
    }
    if let Some(object) = view
        .objects
        .values()
        .find(|object| object.name == "gnoblin" && object.kind == ObjectKind::Service)
    {
        c.line(
            "/** The service `gnoblin`, made once: the root of the object model, which a shell reads as `gnoblin`. */",
        );
        c.line(format!("export declare const gnoblin: {};", class_name(object)));
    }
    Ok(c.finish())
}

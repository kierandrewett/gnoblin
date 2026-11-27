//! The JavaScript and TypeScript classes of a namespace served through the object protocol: `objects.js` and its `objects.d.ts`.
//!
//! The classes call `__gnoblinObjects` (`gnoblin-runtime-js`'s `object_native.rs`) through the small runtime in `object_base.js`. A value
//! crosses as the script's own: a record is a plain object (the native layer sees it as an array of fields, in the catalog's order), an enum
//! is its name, a handle is a class instance and a closure is a function. The three that need a mark when they go in (a record, an enum
//! value, an object) are wrapped by `B.rec`, `B.enm` and `B.ref`, which is the only place the encoding is known.

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_schema::{CallbackDecl, DefaultValue, EnumDecl, Field, Method, ObjectKind, ObjectType, Property, Type};

use super::object_script::{ScriptView, quote};
use super::objects::{
    ClassMember, Member, assign_fields, augment_finds, augments_of, check_assign, check_foreign, ctor_properties,
    finds_of, is_augment_setter, options_type_name, slots_of,
};
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

// A script's reference to a library object ends when the script's object is collected. The object holds a native guard of its slot
// (`native.guard`), a value of the engine's own whose finaliser runs as the object is freed: it files the slot for release and runs no
// script. The engine lets go of the slot at the next frame, so a script object that is made and dropped queues no promise job, and a
// shell that does this on every event still comes to rest. (A `FinalizationRegistry` is not an option: the engine runs each of its
// callbacks as a promise job, and a job left at the end of a frame is a frame due at once.)
//
// One library object is one script object: `live` maps the identity of each object the script holds (`native.key`) to its script object,
// so an object handed over twice is the same script object both times and `===`, `indexOf` and a `Map` key work on it. The second
// delivery's reference is released at once, since the script object already holds one. The entry of an object that was collected is
// dropped when its identity is next handed over, or when the map has doubled since it was last swept.
const live = new Map();
const GUARD = Symbol("gnoblin.guard");
let sweepAt = 256;

/** Drops the entries of `live` whose script object was collected. */
function sweep() {
  for (const [key, ref] of live) {
    if (ref.deref() === undefined) {
      live.delete(key);
    }
  }
  sweepAt = Math.max(256, live.size * 2);
}

/** The script object that already holds the object of `slot`, which then lets `slot` go; or none, with the key to file it under. */
function held(slot) {
  const key = native.key(slot);
  const found = live.get(key)?.deref();
  if (found !== undefined) {
    native.release(slot);
  } else {
    live.delete(key);
  }
  return { found, key };
}

/** Makes `instance` hold `slot` filed under `key`, and drop it when the instance is collected. */
function hold(instance, slot, key) {
  if (live.size >= sweepAt) {
    sweep();
  }
  live.set(key, new WeakRef(instance));
  instance[HANDLE] = slot;
  instance[GUARD] = native.guard(slot);
  return instance;
}

const classes = new Map();

/**
 * What a failed call on a library object throws: `code` says why (`not_found`, `read_only`, ...) and `target` names the object and
 * member, as in `Client#7.x`. The message begins with the target. A bad argument is not one of these: it is a `TypeError` or a
 * `RangeError`, because it is the script's own mistake and no state is involved.
 *
 * It is one class with the `GnoblinError` of the TypeScript library (`gnoblin-core/errors`), which a shell that imports both has: the
 * module that loads first makes it under a key of the symbol registry and the other takes that one, so `instanceof` holds whichever
 * module the shell imported it from.
 */
export const GnoblinError = (globalThis[Symbol.for("gnoblin.GnoblinError")] ??= class GnoblinError extends Error {
  constructor(code, target, message) {
    super(`${target}: ${message}`);
    this.name = "GnoblinError";
    this.code = code;
    this.target = target;
  }
});

let naming = false;

/** The `id` of `self` as text, when it has one that can be read: a failed read of it names nothing and is not another failure. */
function identity(self) {
  if (naming || self === undefined || self === null) {
    return undefined;
  }
  naming = true;
  try {
    const id = self.id;
    return typeof id === "string" || typeof id === "number" ? String(id) : undefined;
  } catch {
    return undefined;
  } finally {
    naming = false;
  }
}

// The library words a refusal with the names of the catalog: a member is `class.member` and a parameter or a field of a record is its
// own name, all in snake_case. A script spells them as the generated class does, so the call site that knows both (its `spell` table,
// `[class, { member: [spelling, { name: spelling }] }]`) writes the script's spelling in their place.
const LABEL =
  /`([a-z][a-z0-9_]*)\.([a-z][a-z0-9_]*)`((?:\[\d+\]|\.[a-z][a-z0-9_]*)*)(?:: `([a-z][a-z0-9_]*)`((?:\[\d+\]|\.[a-z][a-z0-9_]*)*))?/g;
const TARGET = /^([A-Za-z0-9_]+)(#[^.:\s]*)?\.([a-z][a-z0-9_]*)(?=:|\()/;

/** What `map` holds under `key` as its own entry, or none. */
function own(map, key) {
  return map !== undefined && Object.prototype.hasOwnProperty.call(map, key) ? map[key] : undefined;
}

/** `path` (`.field` and `[index]` steps) with each field spelled as `names` spells it. */
function respellPath(path, names) {
  return path.replace(/\.([a-z][a-z0-9_]*)/g, (_, field) => `.${own(names, field) ?? field}`);
}

/**
 * `message` with the catalog's names of the members of the class `spell` describes written as the class `className` spells them: the
 * label of the member (`class.member`, then a record's fields or a list's indexes, then the parameter a refusal names) and the target
 * a library puts first in its own words (`Class#id.member:`). A name `spell` does not know is left as it is.
 */
function respell(message, className, spell) {
  const [schema, members] = spell;
  const labelled = message.replace(LABEL, (whole, owner, name, path, parameter, parameterPath) => {
    const entry = owner === schema ? own(members, name) : undefined;
    if (entry === undefined) {
      return whole;
    }
    const [spelling, names] = entry;
    const label = `\`${className}.${spelling}\`${respellPath(path, names)}`;
    return parameter === undefined ? label : `${label}: \`${own(names, parameter) ?? parameter}\`${respellPath(parameterPath, names)}`;
  });
  return labelled.replace(TARGET, (whole, owner, id, name) => {
    const entry = owner === className ? own(members, name) : undefined;
    return entry === undefined ? whole : `${owner}${id ?? ""}.${entry[0]}`;
  });
}

/**
 * The error to throw for what a call of `member` of `className` threw. The native layer throws an `Error` with the library's words, its
 * `code` as text and the `abi` integer for a failure that has a code; the call site, which knows the class, the object and the member,
 * makes it a `GnoblinError` with the target `Class#id.member` (`Class.member` when there is no object or it has no id). Anything else
 * is returned as it is: a `TypeError` for a bad argument, or an error the script's own code threw. Whichever it is, the names of the
 * catalog in its words are written as the class spells them when the call site gives its `spell` table.
 */
export function raise(error, self, className, member, spell) {
  if (!(error instanceof Error) || typeof error.code !== "string" || typeof error.abi !== "number") {
    if (spell !== undefined && (error instanceof TypeError || error instanceof RangeError)) {
      const message = respell(error.message, className, spell);
      if (message !== error.message) {
        error.message = message;
      }
    }
    return error;
  }
  const id = identity(self);
  const target = id === undefined ? `${className}.${member}` : `${className}#${id}.${member}`;
  let message = spell === undefined ? error.message : respell(error.message, className, spell);
  for (const lead of [`${target}: `, `${target}(): `]) {
    if (message.startsWith(lead)) {
      message = message.slice(lead.length);
      break;
    }
  }
  return new GnoblinError(error.code, target, message);
}

/** Opens the namespace the library serves, once for this generation. Throws when the library is not installed. */
export function open(namespace) {
  return native.open(namespace);
}

/** The ids of the class `name` and its members `names` (`p:text`, `m:insert`, `s:changed`), asked of the library the first time they are used. */
export function ids(namespace, name, names) {
  let found;
  return () => (found ??= native.ids(namespace, name, names));
}

/**
 * Makes `instance` hold `slot`, and drop it when the instance is collected. When a script object holds that library object already, that
 * one is answered and `slot` is let go.
 */
export function adopt(instance, slot) {
  const { found, key } = held(slot);
  return found ?? hold(instance, slot, key);
}

/** Remembers the class a library object of the name `name` is read as. */
export function register(namespace, name, constructor) {
  classes.set(`${namespace}/${name}`, constructor);
}

/** An instance of the class the library says `slot` is, or of `fallback` when the script has no such class. */
export function wrap(namespace, slot, fallback) {
  const { found, key } = held(slot);
  if (found !== undefined) {
    return found;
  }
  let name;
  try {
    name = native.classOf(slot);
  } catch (error) {
    throw raise(error, undefined, fallback.name, "classOf");
  }
  const constructor = classes.get(`${namespace}/${name}`) ?? fallback;
  return hold(Object.create(constructor.prototype), slot, key);
}

export const same = (value) => value;
export const opt = (value, convert) => (value === undefined || value === null ? undefined : convert(value));

export function need(value, what) {
  if (value === undefined || value === null) {
    throw new TypeError(`${what} is required`);
  }
  return value;
}

/** Whether a script object answers the method `name` of the interface `label`: it does with a function, and an object that holds anything else there is refused. */
export function defines(object, name, label) {
  const member = object[name];
  if (typeof member === "function") {
    return true;
  }
  if (member !== undefined && member !== null) {
    throw new TypeError(`${label}.${name}: expected a function, got ${typeof member}`);
  }
  return false;
}

/**
 * Checks the option bag of a value class: an object, and no field the class lacks. `refused` maps a name that is not a field to the reason
 * the class refuses it, which is said in place of the list of fields.
 */
export function fields(init, label, known, refused) {
  if (init === null || typeof init !== "object" || Array.isArray(init)) {
    throw new TypeError(`${label}: expects an object of fields`);
  }
  for (const key of Object.keys(init)) {
    if (!known.includes(key)) {
      if (refused !== undefined && Object.hasOwn(refused, key)) {
        throw new TypeError(`${label}.${key} is not a field: ${refused[key]}`);
      }
      throw new TypeError(`${label}: no field "${key}" (known: ${known.join(", ")})`);
    }
  }
}

/**
 * `find(kind, filter)` of a service: the objects of one kind that match `filter`, a table of field values or a predicate. `finders`
 * maps each kind the service knows to the method that lists it and a function that answers the class of its filter, so a kind it does
 * not know is refused with the kinds that work, and so is a table with a field the class lacks.
 */
export function find(self, label, finders, kind, filter) {
  const entry = typeof kind === "string" && Object.hasOwn(finders, kind) ? finders[kind] : undefined;
  if (entry === undefined) {
    throw new TypeError(
      `${label}.find: no kind ${JSON.stringify(kind)}; the kinds that work are ${Object.keys(finders).join(", ")}`,
    );
  }
  const [method, queryClass] = entry;
  if (typeof filter === "function") {
    return self[method]().filter((found) => filter(found));
  }
  if (filter !== undefined && filter !== null && typeof filter === "object" && !(filter instanceof queryClass())) {
    const Query = queryClass();
    filter = new Query(filter);
  }
  return self[method](filter);
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

/**
 * An object of another namespace's class on its way in. The wire carries no namespace and the two libraries issue the same handle
 * numbers, so a value that is not an instance of `Class` is refused here, before the library could mistake it for one of its own.
 */
export function refTo(value, Class, what) {
  if (!(value instanceof Class)) {
    const got = value === null ? "null" : typeof value === "object" ? (value.constructor?.name ?? "an object") : typeof value;
    throw new TypeError(`expected ${what}, got ${got}`);
  }
  return ref(value);
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
      try {
        native.unlisten(this.#namespace, registration);
      } catch (error) {
        throw raise(error, undefined, "Listener", "stop");
      }
    }
  }
}

/** Listens to `event` of `self`, an object of `className`: `events` maps an event to its member's place in `ids()` and the decoders of its arguments. */
export function listen(self, namespace, ids, events, event, handler, className) {
  const spec = events[event];
  if (spec === undefined) {
    throw new TypeError(`no event ${JSON.stringify(event)}; the events are ${Object.keys(events).join(", ")}`);
  }
  if (typeof handler !== "function") {
    throw new TypeError(`expected a function, got ${typeof handler}`);
  }
  const [at, decoders] = spec;
  let registration;
  try {
    registration = native.listen(self[HANDLE], ids()[at], (member, args) => {
      handler(...args.map((arg, k) => decoders[k](arg)));
    });
  } catch (error) {
    throw raise(error, self, className, event);
  }
  return new Listener(namespace, registration);
}

/** Writes the property `name` of `self`, an object of `className`, over a time. `spell` is the table of the class (`raise`). */
export function setTimed(self, member, value, timing, className, name, spell) {
  try {
    native.setTimed(self[HANDLE], member, value, {
      seconds: need(timing?.seconds, "timing.seconds"),
      delay: timing.delay ?? 0,
      tween: timing.tween ?? 0,
      samples: timing.samples ?? [],
    });
  } catch (error) {
    throw raise(error, self, className, name, spell);
  }
}

/** The members already added to a class, so that loading a library again replaces its own and refuses another's. */
const augmented = new WeakSet();

function augmentable(target, name) {
  if (name in target.prototype) {
    const own = Object.getOwnPropertyDescriptor(target.prototype, name);
    if (own === undefined || !augmented.has(own.get ?? own.value)) {
      throw new TypeError(`${target.name} has a member "${name}" of its own: a library cannot add one of the same name`);
    }
  }
}

/**
 * Adds the property `name` to the core class `target`, which reads `read(object)` for the object it is read on. With `write`, an
 * assignment calls `write(object, value)`; without it the property is read-only.
 */
export function augmentGet(target, name, read, write) {
  augmentable(target, name);
  const get = function () {
    return read(this);
  };
  augmented.add(get);
  const set =
    write === undefined
      ? undefined
      : function (value) {
          write(this, value);
        };
  Object.defineProperty(target.prototype, name, { configurable: true, enumerable: false, get, set });
}

/** Adds the method `name` to the core class `target`, which calls `run(object, ...args)` for the object it is called on. */
export function augmentCall(target, name, run) {
  augmentable(target, name);
  const value = function (...args) {
    return run(this, ...args);
  };
  augmented.add(value);
  Object.defineProperty(target.prototype, name, { configurable: true, enumerable: false, writable: true, value });
}

/**
 * `set(props, options)` of an object: writes each property `props` names in one transition, which `request(work, options)` commits.
 * Every key must be a writable property of the object, or nothing is written. Without `options` the change is not animated.
 */
export function assign(self, label, props, options, request) {
  if (props === null || typeof props !== "object" || Array.isArray(props)) {
    throw new TypeError(`${label}.set: expects an object of properties`);
  }
  for (const key of Object.keys(props)) {
    let found;
    for (let at = self; at !== null && found === undefined; at = Object.getPrototypeOf(at)) {
      found = Object.getOwnPropertyDescriptor(at, key);
    }
    if (found === undefined || typeof found.set !== "function") {
      throw new TypeError(`${label}.set: ${key} is not a writable property`);
    }
  }
  return request(() => {
    for (const [key, value] of Object.entries(props)) {
      self[key] = value;
    }
  }, options ?? { duration: 0 });
}

/** Where a service class keeps the kinds its `find` knows: `[method, () => QueryClass]` by kind name. */
export const FINDERS = Symbol("finders");

/** The kinds libraries above added to a class's `find`, so loading a library again replaces its own and refuses another's. */
const augmentedKinds = new WeakSet();

/** Adds the kind `kind` to the `find` of the core class `target`: its objects are listed by the method `method`, which a library above
 * added, filtered by the record class `query()` returns. */
export function augmentFind(target, kind, method, query) {
  const table = target[FINDERS];
  if (table === undefined) {
    throw new TypeError(`${target.name} has no find: a library cannot add the kind "${kind}" to it`);
  }
  if (Object.hasOwn(table, kind) && !augmentedKinds.has(table[kind])) {
    throw new TypeError(`${target.name}.find has a kind "${kind}" of its own: a library cannot add one of the same name`);
  }
  const entry = [method, query];
  augmentedKinds.add(entry);
  table[kind] = entry;
}
"#;

/// The declarations of `object_base.js`.
pub(super) const BASE_DTS: &str = r#"// The parts of the object runtime a script sees.

/** A listener of an object's signal or property. It lasts until it is stopped or the generation ends. */
export declare class Listener {
  /** Stops hearing. A second call does nothing. */
  stop(): void;
}

/** Why a call failed. */
export type ErrorCode =
  | "invalid_argument"
  | "not_found"
  | "already_exists"
  | "invalid_state"
  | "read_only"
  | "unsupported"
  | "denied"
  | "busy"
  | "cancelled"
  | "timed_out"
  | "unavailable"
  | "internal"
  | "config_rejected";

/**
 * What a failed call on a library object throws. `code` says why and `target` names the object and member, as in `Client#7.x`; the
 * message begins with the target. A bad argument throws a `TypeError` or a `RangeError` instead.
 */
export declare class GnoblinError extends Error {
  /** Stable reason code for this failure. */
  readonly code: ErrorCode;
  /** The object and member the failed call named. */
  readonly target: string;
  constructor(code: ErrorCode, target: string, message: string);
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

pub(super) fn member_name(name: &str, export: &Option<String>) -> String {
    export.clone().unwrap_or_else(|| camel(name))
}

/// What commits a `set(props, options)`: the root's `requestTransition`, which the namespace that declares `assign` adds to the core's
/// `Gnoblin` ([`check_assign`]).
const ASSIGN_REQUEST: &str = "(work, options) => Core.Gnoblin.get().requestTransition(work, options)";

fn param_name(field: &Field) -> String {
    ts_ident(&camel(&field.name))
}

/// The statements `body` writes, inside a `try` whose `catch` throws what `B.raise` makes of the failure: a `GnoblinError` naming the
/// member of `class` and the object `object` (`this`, or `undefined` for a call that has none), or the error itself when it is not a
/// failure of the library (a `TypeError` for a bad argument). `spell` is the table of the class (`spell_<class>`, [`Js::spell_source`]) that
/// lets `B.raise` write the names the library's words carry as the script spells them.
fn guarded(
    c: &mut CodeWriter,
    class: &str,
    member: &str,
    object: &str,
    spell: Option<&str>,
    body: impl FnOnce(&mut CodeWriter),
) {
    c.line("try {");
    c.indent();
    body(c);
    c.dedent();
    c.line("} catch (error) {");
    c.indent();
    let spell = spell.map(|table| format!(", {table}")).unwrap_or_default();
    c.line(format!(
        "throw B.raise(error, {object}, {}, {}{spell});",
        quote(class),
        quote(member)
    ));
    c.dedent();
    c.line("}");
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
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Any => e.to_owned(),
            Type::Bytes => format!("B.bytes({e})"),
            Type::Enum(name) => format!("enc_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            Type::ObjectRef(name) if name.contains('.') => {
                let foreign = self.view.foreign(name)?;
                format!(
                    "B.refTo({e}, {}, {})",
                    foreign.alias,
                    quote(&format!("a {}", foreign.js))
                )
            }
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
            Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes | Type::Any => e.to_owned(),
            Type::Enum(name) => format!("dec_enum_{}({e})", snake(&self.view.enum_decl(name)?.name)),
            // An object of another library is read in that library's namespace, as an instance of its class there.
            Type::ObjectRef(name) if name.contains('.') => {
                let foreign = self.view.foreign(name)?;
                let home = quote(&foreign.namespace);
                if absent {
                    format!("B.opt({e}, (slot) => B.wrap({home}, slot, {}))", foreign.alias)
                } else {
                    format!("B.wrap({home}, {e}, {})", foreign.alias)
                }
            }
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
        c.line(format!(
            "B.fields(init, {}, [{}]{});",
            quote(&name),
            known.join(", "),
            Self::refused_arg(class.refuses)
        ));
        for property in class.properties {
            let field = js_field(property);
            let value = match absent(property) {
                Absent::Default(default) => format!("init.{field} ?? {}", self.default_expr(default)),
                Absent::None => format!("init.{field}"),
                Absent::Required => format!("B.need(init.{field}, {})", quote(&format!("{name}.{field}"))),
            };
            c.line(format!("this.{field} = {value};"));
        }
        if class.immutable {
            c.line("Object.freeze(this);");
        }
        c.dedent();
        c.line("}");
        c.dedent();
        c.line("}");
        c.blank();
    }

    /// The reasons a class refuses a name that is not a field, as the trailing argument of `B.fields`: nothing for a class that has none.
    fn refused_arg(refuses: &[(String, String)]) -> String {
        if refuses.is_empty() {
            return String::new();
        }
        let reasons: Vec<String> = refuses
            .iter()
            .map(|(name, reason)| format!("{}: {}", quote(&camel(name)), quote(reason)))
            .collect();
        format!(", {{ {} }}", reasons.join(", "))
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
        if let Some(field) = number_field(object)? {
            // A bare number stands for the record with only its `number` field.
            c.line(format!(
                "if (typeof record === \"number\") record = new {class}({{ {}: record }});",
                js_field(field)
            ));
        }
        c.line("if (record === null || typeof record !== \"object\") {");
        c.line(format!(
            "  throw new TypeError(`expected a {class} object, got ${{record === null ? \"null\" : typeof record}}`);"
        ));
        c.line("}");
        // A bag a script wrote is checked as the class's constructor checks it: a field the class lacks is refused, not dropped.
        let known: Vec<String> = object
            .properties
            .iter()
            .map(|property| quote(&js_field(property)))
            .collect();
        c.line(format!(
            "if (!(record instanceof {class})) B.fields(record, {}, [{}]{});",
            quote(&class),
            known.join(", "),
            Self::refused_arg(&object.refuses)
        ));
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
        // A plain object with one key, a variant's tag, is that variant: the key's value is the variant's one property, or its options.
        c.line("if (record !== null && typeof record === \"object\" && Object.getPrototypeOf(record) === Object.prototype) {");
        c.line("  const keys = Object.keys(record);");
        c.line("  if (keys.length === 1) switch (keys[0]) {");
        for (variant, js_name) in family.variants.iter().zip(&classes) {
            let key = literal_key(variant);
            let init = match variant.properties.as_slice() {
                [only] => format!("{{ {}: record.{key} }}", js_field(only)),
                _ => format!("record.{key}"),
            };
            c.line(format!(
                "    case {}: return enc_var_{}(new {js_name}({init}));",
                quote(&key),
                snake(&variant.name)
            ));
        }
        c.line("  }");
        c.line("}");
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

    /// The names a script spells differently from the catalog, for a failure of a call to say them as the script wrote them.
    ///
    /// The library words a refusal with the catalog's names (`class.member`, and the parameter or field it refuses), which are snake_case,
    /// and the script's are camelCase. The class has a table of its own members, `spell_<class>`, that `B.raise` reads: the class and
    /// each member of its own with the spelling the class gives it, and the parameters and the record fields the member takes, where
    /// they are spelled otherwise. The members of a base class that another library serves are worded by that library.
    fn spell_source(&self, c: &mut CodeWriter, object: &ObjectType, slots: &[ClassMember<'_>]) -> Result<(), String> {
        let mut members = Vec::new();
        for slot in slots.iter().filter(|slot| !slot.foreign) {
            let mut names = BTreeMap::new();
            let (name, spelling) = match &slot.member {
                Member::Property(property) => {
                    self.spelt_fields(&property.ty, &mut names, &mut BTreeSet::new())?;
                    (&property.name, member_name(&property.name, &property.extras.export.js))
                }
                Member::Method(method) => {
                    for param in &method.params {
                        names.insert(param.name.clone(), param_name(param));
                        self.spelt_fields(&param.ty, &mut names, &mut BTreeSet::new())?;
                    }
                    (&method.name, member_name(&method.name, &method.extras.export.js))
                }
                Member::Signal(_) => continue,
            };
            names.retain(|name, spelling| name != spelling.as_str());
            let names = if names.is_empty() {
                String::new()
            } else {
                format!(
                    ", {{ {} }}",
                    names
                        .iter()
                        .map(|(name, spelling)| format!("{}: {}", quote(name), quote(spelling)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            members.push(format!("{}: [{}{names}]", quote(name), quote(&spelling)));
        }
        c.line(format!(
            "const spell_{} = [{}, {{ {} }}];",
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
                        names.insert(field.name.clone(), js_field(field));
                        self.spelt_fields(&field.ty, names, seen)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
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
        self.spell_source(c, object, &slots)?;
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

        let finds = finds_of(object);
        if has_instance_members && !finds.is_empty() {
            claim("find", false)?;
            let mut kinds = Vec::new();
            for (kind, method) in &finds {
                let Some(Type::ObjectRef(query)) = method.params.first().map(|param| &param.ty) else {
                    return Err(format!(
                        "`{}`: the finder of `{kind}` takes no filter record",
                        object.name
                    ));
                };
                kinds.push(format!(
                    "{kind}: [{}, () => {}]",
                    quote(&member_name(&method.name, &method.extras.export.js)),
                    class_name(self.view.class(query)?)
                ));
            }
            c.line(format!("const finders_{snake_name} = {{ {} }};", kinds.join(", ")));
        }
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
                Member::Property(property) => {
                    self.property_source(c, &name, &snake_name, &lookups, property, &mut claim)?
                }
                Member::Method(method) => self.method_source(c, &name, &snake_name, &lookups, method, &mut claim)?,
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
                            "return B.listen(this, NS, ids_{snake_name}, events_{snake_name}, event, handler, {});",
                            quote(&name)
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
            if !finds.is_empty() {
                c.line("/**");
                c.line(" * The live objects of one kind that match `filter`, a table of field values or a function that keeps an object when it");
                c.line(format!(
                    " * returns a truthy value: {}. An unknown kind or field throws and names what works.",
                    finds
                        .iter()
                        .map(|(kind, _)| format!("`{kind}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                c.line(" */");
                c.block(
                    "find(kind, filter) {",
                    |c| {
                        c.line(format!(
                            "return B.find(this, {}, finders_{snake_name}, kind, filter);",
                            quote(&object.name)
                        ))
                    },
                    "}",
                );
            }
            if object.assign {
                check_assign(object, None, self.view.objects.values().copied())?;
                c.line("/** Writes each property `props` names in one transition. Without `options` the change is not animated. */");
                c.block(
                    "set(props, options) {",
                    |c| {
                        c.line(format!(
                            "return B.assign(this, {}, props, options, {ASSIGN_REQUEST});",
                            quote(&name)
                        ))
                    },
                    "}",
                );
            }
            c.line("/** Ends the object and its children, whoever else holds it. */");
            c.block(
                "destroy() {",
                |c| {
                    guarded(c, &name, "destroy", "this", None, |c| {
                        c.line("N.destroy(this[B.HANDLE]);")
                    })
                },
                "}",
            );
        }
        c.dedent();
        c.line("}");
        c.line(format!("B.register(NS, {}, {name});", quote(&object.name)));
        if has_instance_members && !finds.is_empty() {
            // A library above may add a kind to this `find` (`B.augmentFind`).
            c.line(format!("{name}[B.FINDERS] = finders_{snake_name};"));
        }
        c.blank();
        Ok(())
    }

    /// Where a shell reads the service `object`, for the words of a refusal: the property of `gnoblin` that holds it, `gnoblin` for the
    /// root itself, and the class's own `get` for a service that nothing holds.
    fn service_source(&self, object: &ObjectType) -> String {
        if object.name == "gnoblin" {
            return "`gnoblin`".to_owned();
        }
        let held = self
            .view
            .objects
            .values()
            .filter(|holder| holder.name == "gnoblin" || holder.augment_of.as_deref() == Some("gnoblin"))
            .flat_map(|holder| holder.properties.iter())
            .find(|property| matches!(&property.ty, Type::ObjectRef(name) if *name == object.name));
        match held {
            Some(property) => format!("`gnoblin.{}`", member_name(&property.name, &property.extras.export.js)),
            None => format!("`{}.get()`", class_name(object)),
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
        let spell = format!("spell_{snake_name}");
        if object.kind == ObjectKind::Service {
            // A service is one object of its generation, which `get` and the root that holds it answer: a script that makes its own has
            // an object that is no service's, so `new` is refused and says where the service is.
            let message = format!(
                "`{}` is a service: a shell does not make one, it reads {}",
                class_name(object),
                self.service_source(object)
            );
            c.block(
                "constructor() {",
                |c| c.line(format!("throw new TypeError({});", quote(&message))),
                "}",
            );
            c.line("/** The service. */");
            c.block(
                "static get() {",
                |c| {
                    c.line(format!("const i = ids_{snake_name}();"));
                    guarded(c, &class_name(object), "get", "undefined", None, |c| {
                        c.line(format!(
                            "return B.adopt(Object.create({}.prototype), N.create(NS, i[0], []));",
                            class_name(object)
                        ));
                    });
                },
                "}",
            );
            return Ok(());
        }
        let ctor = ctor_properties(object, slots)?;
        let mut params = Vec::new();
        let mut inits = Vec::new();
        for property in &ctor.positional {
            let param = ts_ident(&camel(&property.name));
            inits.push(format!(
                "[i[{}], {}]",
                Self::at(lookups, 'p', &property.name),
                self.enc(&property.ty, &param)?
            ));
            params.push(param);
        }
        // A class with no positional property takes the options record alone, and any other takes it after them when it has options.
        if object.ctor.is_none() || !ctor.options.is_empty() {
            if params.iter().any(|param| param == "options") {
                return Err(format!(
                    "`{}`: a constructor parameter is called `options`, which is the name of the options record",
                    object.name
                ));
            }
            params.push("options = {}".to_owned());
        }
        let mut lines = vec![format!("const i = ids_{snake_name}();")];
        if params.iter().any(|param| param == "options = {}") {
            // The options are the properties the constructor sets; a key that is none of them is refused, not dropped.
            let known: Vec<String> = ctor
                .options
                .iter()
                .map(|property| quote(&member_name(&property.name, &property.extras.export.js)))
                .collect();
            lines.push(format!(
                "B.fields(options, {}, [{}]);",
                quote(&class_name(object)),
                known.join(", ")
            ));
        }
        if ctor.options.is_empty() {
            lines.push(format!("B.adopt(this, N.create(NS, i[0], [{}]));", inits.join(", ")));
        } else {
            lines.push(format!("const inits = [{}];", inits.join(", ")));
            for property in &ctor.options {
                let field = member_name(&property.name, &property.extras.export.js);
                lines.push(format!(
                    "if (options.{field} !== undefined) inits.push([i[{}], {}]);",
                    Self::at(lookups, 'p', &property.name),
                    self.enc(&property.ty, &format!("options.{field}"))?
                ));
            }
            lines.push("B.adopt(this, N.create(NS, i[0], inits));".to_owned());
        }
        c.line(if ctor.options.is_empty() {
            "/** Makes the object. */"
        } else {
            "/** Makes the object; each field of `options` sets the property of its name in the same call. */"
        });
        c.block(
            format!("constructor({}) {{", params.join(", ")),
            |c| {
                c.line(&lines[0]);
                guarded(c, &class_name(object), "new", "undefined", Some(&spell), |c| {
                    for line in &lines[1..] {
                        c.line(line);
                    }
                });
            },
            "}",
        );
        Ok(())
    }

    fn property_source(
        &self,
        c: &mut CodeWriter,
        class: &str,
        snake_name: &str,
        lookups: &[MemberLookup],
        property: &Property,
        claim: &mut impl FnMut(&str, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&property.name, &property.extras.export.js);
        let spell = format!("spell_{snake_name}");
        let at = Self::at(lookups, 'p', &property.name);
        let ids = format!("ids_{snake_name}()[{at}]");
        if !matches!(property.ty, Type::Callback(_)) {
            claim(&ident, false)?;
            let read = self.dec(&property.ty, &format!("N.get(this[B.HANDLE], {ids})"), true)?;
            c.line(format!("/** The property `{}`. */", property.name));
            c.block(
                format!("get {ident}() {{"),
                |c| {
                    guarded(c, class, &ident, "this", Some(&spell), |c| {
                        c.line(format!("return {read};"))
                    })
                },
                "}",
            );
        } else {
            claim(&ident, false)?;
        }
        if !property.readonly {
            let write = self.enc(&property.ty, "value")?;
            c.block(
                format!("set {ident}(value) {{"),
                |c| {
                    guarded(c, class, &ident, "this", Some(&spell), |c| {
                        c.line(format!("N.set(this[B.HANDLE], {ids}, {write});"))
                    })
                },
                "}",
            );
            if property.animated {
                let timed = format!("set{}Timed", pascal(&ident));
                claim(&timed, false)?;
                c.line(format!("/** Sets the property `{}` over a time. */", property.name));
                c.block(
                    format!("{timed}(value, timing) {{"),
                    |c| {
                        c.line(format!(
                            "B.setTimed(this, {ids}, {write}, timing, {}, {}, {spell});",
                            quote(class),
                            quote(&timed)
                        ))
                    },
                    "}",
                );
            }
        }
        Ok(())
    }

    fn method_source(
        &self,
        c: &mut CodeWriter,
        class: &str,
        snake_name: &str,
        lookups: &[MemberLookup],
        method: &Method,
        claim: &mut impl FnMut(&str, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_name(&method.name, &method.extras.export.js);
        let spell = format!("spell_{snake_name}");
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
                guarded(
                    c,
                    class,
                    &ident,
                    if method.function { "undefined" } else { "this" },
                    Some(&spell),
                    |c| c.line(body),
                );
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
                "if (B.defines(object, {}, {})) implemented.push(i[{at}]);",
                quote(&ident),
                quote(&name)
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
        guarded(c, &name, "implement", "undefined", None, |c| {
            c.line("return N.implement(NS, i[0], implemented, (member, args) => {");
            c.indent();
            for arm in &arms {
                c.line(arm);
            }
            c.line("throw new Error(\"the library called a method this interface does not have\");");
            c.dedent();
            c.line("});");
        });
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
    c.line("export { GnoblinError } from \"gnoblin-core/generated/object_base\";");
    for foreign in view.foreigns()? {
        c.line(format!(
            "import {{ {} as {} }} from \"{}/generated/objects\";",
            foreign.js, foreign.alias, foreign.package
        ));
    }
    if !augments_of(view.catalog, &view.objects)?.is_empty() {
        c.line("import * as Core from \"gnoblin-core/generated/objects\";");
    }
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
    for (class, target) in augments_of(view.catalog, &view.objects)? {
        js.augment_source(&mut c, class, target)?;
    }
    Ok(c.finish())
}

impl Js<'_> {
    /// What installs the members of the class `class`, which holds an augment, on the class `target` of the core's module: each is read or
    /// called through the service of this library, made for the generation that is running when the member is used, so a member binds to
    /// the library of the generation and never to one that has retired. A member of a `receiver = true` augment hands the service the
    /// object it is used on.
    fn augment_source(&self, c: &mut CodeWriter, class: &ObjectType, target: &ObjectType) -> Result<(), String> {
        let name = class_name(class);
        let core = format!("Core.{}", class_name(target));
        c.line(format!(
            "// The members `{}` adds to `{}` of `gnoblin-core`: they exist on it once this module is loaded, and a shell that never loads it has none.",
            self.view.namespace,
            class_name(target)
        ));
        for property in &class.properties {
            let ident = member_name(&property.name, &property.extras.export.js);
            c.line(format!(
                "B.augmentGet({core}, {}, () => {name}.get().{ident});",
                quote(&ident)
            ));
        }
        let getters = class.augment_getters.as_deref();
        for method in &class.methods {
            // A property's setter is installed with its getter, not as a method of its own.
            if is_augment_setter(class, &method.name) {
                continue;
            }
            let ident = member_name(&method.name, &method.extras.export.js);
            let line = match getters {
                Some(getters) if getters.contains(&method.name) => {
                    let write = if class.augment_setters.contains(&method.name) {
                        format!(
                            ", (self, value) => {name}.get().{}(self, value)",
                            member_name(&format!("set_{}", method.name), &None)
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "B.augmentGet({core}, {}, (self) => {name}.get().{ident}(self){write});",
                        quote(&ident)
                    )
                }
                Some(_) => format!(
                    "B.augmentCall({core}, {}, (self, ...args) => {name}.get().{ident}(self, ...args));",
                    quote(&ident)
                ),
                None => format!(
                    "B.augmentCall({core}, {}, (_self, ...args) => {name}.get().{ident}(...args));",
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
                "B.augmentFind({core}, {}, {}, () => {});",
                quote(kind),
                quote(&member_name(&method.name, &method.extras.export.js)),
                class_name(self.view.class(query)?)
            ));
        }
        // `set(props, options)` of the core class, over its own writable properties and those the augment adds.
        if class.assign {
            check_assign(class, Some(target), self.view.objects.values().copied())?;
            c.line(format!(
                "B.augmentCall({core}, \"set\", (self, props, options) => B.assign(self, {}, props, options, {ASSIGN_REQUEST}));",
                quote(&class_name(target))
            ));
        }
        c.blank();
        Ok(())
    }

    /// The declarations of those members, merged into the core's class of the same name.
    fn augment_dts(&self, c: &mut CodeWriter, class: &ObjectType, target: &ObjectType) -> Result<(), String> {
        c.line("declare module \"gnoblin-core/generated/objects\" {");
        c.indent();
        c.line(format!(
            "/** What `{}` adds to the core's `{}`: it is on the class once the module of this library is imported. */",
            self.view.namespace,
            class_name(target)
        ));
        c.line(format!("interface {} {{", class_name(target)));
        c.indent();
        for property in &class.properties {
            c.jsdoc(&self.view.member_doc(&class.name, &property.name));
            let ident = member_name(&property.name, &property.extras.export.js);
            let line = format!("readonly {ident}: {};", self.ts(&property.ty, false)?);
            self.view.declared(&class.name, &property.name, &line);
            c.line(line);
        }
        let getters = class.augment_getters.as_deref();
        for method in &class.methods {
            if is_augment_setter(class, &method.name) {
                continue;
            }
            c.jsdoc(&self.view.member_doc(&class.name, &method.name));
            let ident = member_name(&method.name, &method.extras.export.js);
            let returns = match &method.returns {
                Some(ty) if method.optional => self.ts_read(ty)?,
                Some(ty) => self.ts(ty, false)?,
                None => "void".to_owned(),
            };
            // A receiver augment's first parameter is the object the member is used on, which the declaration's `this` is.
            let params = if getters.is_some() {
                &method.params[1..]
            } else {
                &method.params[..]
            };
            if getters.is_some_and(|getters| getters.contains(&method.name)) {
                if class.augment_setters.contains(&method.name) {
                    // A write takes what a parameter of the type takes, which for a record is the plain object of its fields.
                    let ty = method
                        .returns
                        .as_ref()
                        .ok_or_else(|| format!("`{}`: the getter `{}` returns nothing", class.name, method.name))?;
                    let input = self.ts(ty, true)?;
                    let input = if method.optional {
                        format!("{input} | null | undefined")
                    } else {
                        input
                    };
                    for line in [
                        format!("get {ident}(): {returns};"),
                        format!("set {ident}(value: {input});"),
                    ] {
                        self.view.declared(&class.name, &method.name, &line);
                        c.line(line);
                    }
                } else {
                    let line = format!("readonly {ident}: {returns};");
                    self.view.declared(&class.name, &method.name, &line);
                    c.line(line);
                }
            } else {
                let line = format!("{ident}({}): {returns};", self.params_dts(params)?);
                self.view.declared(&class.name, &method.name, &line);
                c.line(line);
            }
        }
        for (kind, method) in augment_finds(class) {
            let Some(Type::List(inner)) = &method.returns else {
                return Err(format!(
                    "`{}`: the finder of `{kind}` does not return a list",
                    class.name
                ));
            };
            let found = self.ts(inner, false)?;
            let query = self.ts(&method.params[0].ty, true)?;
            c.line(format!(
                "find(kind: {}, filter?: {query} | ((found: {found}) => unknown)): {found}[];",
                quote(kind)
            ));
        }
        if class.assign {
            let mut names: Vec<String> = assign_fields(class, Some(target))
                .iter()
                .map(|property| member_name(&property.name, &property.extras.export.js))
                .collect();
            names.extend(
                class
                    .methods
                    .iter()
                    .filter(|method| class.augment_setters.contains(&method.name))
                    .map(|method| member_name(&method.name, &method.extras.export.js)),
            );
            self.assign_dts(c, &class_name(target), &names)?;
        }
        c.dedent();
        c.line("}");
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }
}

// ---- the declarations ------------------------------------------------------------------------------------------------------

impl Js<'_> {
    /// The declaration of `set(props, options)` on the class `class`, which takes the writable properties `names`.
    fn assign_dts(&self, c: &mut CodeWriter, class: &str, names: &[String]) -> Result<(), String> {
        let picked = if names.is_empty() {
            "never".to_owned()
        } else {
            names.iter().map(|name| quote(name)).collect::<Vec<_>>().join(" | ")
        };
        let options = self.ts(&Type::ObjectRef("transition_options".to_owned()), true)?;
        let transition = self.ts(&Type::ObjectRef("transition".to_owned()), false)?;
        c.line("/** Writes each property `props` names in one transition. Without `options` the change is not animated. */");
        c.line(format!(
            "set(props: Partial<Pick<{class}, {picked}>>, options?: {options}): {transition};"
        ));
        Ok(())
    }

    /// The TypeScript type of `ty`. `input` is the form a parameter takes: a record may leave its defaulted fields out.
    fn ts(&self, ty: &Type, input: bool) -> Result<String, String> {
        Ok(match ty {
            Type::Bool => "boolean".to_owned(),
            Type::Int | Type::Uint | Type::Double => "number".to_owned(),
            Type::Text => "string".to_owned(),
            Type::Bytes => "Uint8Array".to_owned(),
            Type::Any => "unknown".to_owned(),
            Type::Enum(name) => pascal(&self.view.enum_decl(name)?.name),
            Type::ObjectRef(name) if name.contains('.') => self.view.foreign(name)?.alias,
            Type::ObjectRef(name) => {
                let class = self.view.class(name)?;
                let base = class_name(class);
                if input && class.kind == ObjectKind::Record && is_family(class) {
                    format!("{base} | {base}Literal")
                } else if input && class.kind == ObjectKind::Record && has_init(class) {
                    if class.number.is_some() {
                        format!("{base}Init | number")
                    } else {
                        format!("{base}Init")
                    }
                } else if input && class.number.is_some() {
                    format!("{base} | number")
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
                    // A function may answer nothing, which the library reads as the answer its declaration documents.
                    Some(ty) if declared.optional => format!("{} | void", self.ts(ty, true)?),
                    Some(ty) => self.ts(ty, true)?,
                    None => "void".to_owned(),
                };
                format!("({}) => {returns}", params.join(", "))
            }
        })
    }

    /// The type `property` is read as: an object may be absent unless the catalog says the library `always` has one.
    fn ts_read_property(&self, property: &Property) -> Result<String, String> {
        if property.always {
            self.ts(&property.ty, false)
        } else {
            self.ts_read(&property.ty)
        }
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

    /// A `/** */` comment of `text`, or of `fallback` when the schema describes nothing.
    fn doc_or(&self, c: &mut CodeWriter, text: &str, fallback: impl FnOnce() -> String) {
        if text.is_empty() {
            c.line(format!("/** {} */", fallback()));
        } else {
            c.jsdoc(text);
        }
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
            self.doc_or(c, self.view.class_doc(&object.name), || {
                format!(
                    "The record `{}`: any one of its variants, told apart by its class.",
                    object.name
                )
            });
            c.line(format!("export type {} = {};", class_name(object), classes.join(" | ")));
            let literals: Vec<String> = object
                .variants
                .iter()
                .map(|variant| {
                    let key = literal_key(variant);
                    Ok(match variant.properties.as_slice() {
                        [only] => format!("{{ {key}: {} }}", self.ts(&only.ty, true)?),
                        _ => format!("{{ {key}: {}Init }}", values::js_class(&variant.name, &variant.export)),
                    })
                })
                .collect::<Result<_, String>>()?;
            c.line(format!(
                "/** A variant of `{}` written as an object with one key, its tag. */",
                object.name
            ));
            c.line(format!(
                "export type {}Literal = {};",
                class_name(object),
                literals.join(" | ")
            ));
            c.blank();
        }
        Ok(())
    }

    /// A value class: its fields as the class holds them, and the option bag its constructor takes, where a field that has a default (or
    /// is optional) may be left out.
    fn class_dts(&self, c: &mut CodeWriter, class: &ValueClass<'_>) -> Result<(), String> {
        let name = class.js();
        self.doc_or(c, self.view.class_doc(class.name), || {
            format!("The value class `{}`.", class.name)
        });
        c.line(format!("export class {name} {{"));
        c.indent();
        for property in class.properties {
            c.jsdoc(&self.view.member_doc(class.name, &property.name));
            let field = js_field(property);
            let ty = self.ts(&property.ty, false)?;
            let readonly = if class.immutable { "readonly " } else { "" };
            let line = if property.optional {
                format!("{readonly}{field}?: {ty} | undefined;")
            } else {
                format!("{readonly}{field}: {ty};")
            };
            self.view.declared(class.name, &property.name, &line);
            c.line(line);
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
            c.jsdoc(&self.view.member_doc(class.name, &property.name));
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

    /// The name of the interface that holds the options of the constructor of `object`.
    fn options_type(&self, object: &ObjectType) -> Result<String, String> {
        options_type_name(&class_name(object), |name| {
            self.view.objects.values().any(|other| {
                class_name(other) == name
                    || other
                        .variants
                        .iter()
                        .any(|variant| values::js_class(&variant.name, &variant.export) == name)
            }) || self.view.enums.values().any(|declared| pascal(&declared.name) == name)
        })
    }

    /// The interface of the options of the constructor of `object`, when it has any: a field for each property the record carries, each
    /// optional.
    fn options_dts(&self, c: &mut CodeWriter, object: &ObjectType, slots: &[ClassMember<'_>]) -> Result<(), String> {
        let ctor = ctor_properties(object, slots)?;
        if ctor.options.is_empty() {
            return Ok(());
        }
        c.line(format!(
            "/** The options of `new {}(...)`: each field sets the property of its name in the call that makes the object. */",
            class_name(object)
        ));
        c.line(format!("export interface {} {{", self.options_type(object)?));
        c.indent();
        for property in &ctor.options {
            self.doc_or(c, &self.view.member_doc(&object.name, &property.name), || {
                format!("The property `{}`.", property.name)
            });
            c.line(format!(
                "{}?: {};",
                member_name(&property.name, &property.extras.export.js),
                self.ts(&property.ty, true)?
            ));
        }
        c.dedent();
        c.line("}");
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
        let finds = finds_of(object);
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
            for (event, lookup, fields, property) in &events {
                match property {
                    Some(property) => c.jsdoc(&format!(
                        "The property `{}` changed. {}",
                        property.name,
                        self.view.description(&object.name, &property.name)
                    )),
                    None => c.jsdoc(&self.view.member_doc(&object.name, &lookup.name)),
                }
                let args: Vec<String> = match property {
                    Some(property) => vec![format!(
                        "{}: {}",
                        ts_ident(&camel(&property.name)),
                        self.ts_read_property(property)?
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
                let line = format!("{event}: [{}];", args.join(", "));
                if property.is_none() {
                    self.view.declared(&object.name, &lookup.name, &line);
                } else {
                    self.view
                        .declared(&object.name, &format!("{}#changed", lookup.name), &line);
                }
                c.line(line);
            }
            c.dedent();
            c.line("}");
        }
        if has_instance_members && object.kind != ObjectKind::Service {
            self.options_dts(c, object, &slots)?;
        }
        self.doc_or(c, self.view.class_doc(&object.name), || {
            format!("The class `{}` of `{}`.", object.name, self.view.namespace)
        });
        c.line(format!("export declare class {name} {{"));
        c.indent();
        if object.kind == ObjectKind::Service {
            c.line("private constructor();");
        }
        if has_instance_members {
            if object.kind == ObjectKind::Service {
                c.line(format!("static get(): {name};"));
            } else {
                let ctor = ctor_properties(object, &slots)?;
                let mut params = Vec::new();
                for property in &ctor.positional {
                    params.push(format!(
                        "{}: {}",
                        ts_ident(&camel(&property.name)),
                        self.ts(&property.ty, true)?
                    ));
                }
                if ctor.options.is_empty() {
                    if object.ctor.is_none() {
                        params.push("options?: {}".to_owned());
                    }
                } else {
                    params.push(format!("options?: {}", self.options_type(object)?));
                }
                let line = format!("constructor({});", params.join(", "));
                self.view.declared(&object.name, "#new", &line);
                c.line(line);
            }
        } else {
            c.line("private constructor();");
        }
        for slot in &slots {
            match &slot.member {
                Member::Property(property) => self.property_dts(c, &object.name, property)?,
                Member::Method(method) => {
                    c.jsdoc(&self.view.member_doc(&object.name, &method.name));
                    let ident = member_name(&method.name, &method.extras.export.js);
                    let returns = match &method.returns {
                        Some(ty) if method.optional => self.ts_read(ty)?,
                        Some(ty) => self.ts(ty, false)?,
                        None => "void".to_owned(),
                    };
                    let line = format!(
                        "{}{ident}({}): {returns};",
                        if method.function { "static " } else { "" },
                        self.params_dts(&method.params)?
                    );
                    self.view.declared(&object.name, &method.name, &line);
                    c.line(line);
                }
                Member::Signal(_) => {}
            }
        }
        if has_instance_members {
            if !events.is_empty() {
                let line = format!(
                    "on<K extends keyof {name}Events>(event: K, handler: (...args: {name}Events[K]) => void): Listener;"
                );
                self.view.declared(&object.name, "#on", &line);
                c.line(line);
                if object.name == "gnoblin" {
                    for (event, lookup, _, _) in events.iter().filter(|(_, lookup, _, _)| lookup.kind == 's') {
                        let line = format!(
                            "on{}(handler: (...args: {name}Events[{}]) => void): Listener;",
                            pascal(&lookup.name),
                            quote(event)
                        );
                        self.view.declared(&object.name, &lookup.name, &line);
                        c.line(line);
                    }
                }
            }
            for (kind, method) in &finds {
                let Some(Type::List(inner)) = &method.returns else {
                    return Err(format!(
                        "`{}`: the finder of `{kind}` does not return a list",
                        object.name
                    ));
                };
                let found = self.ts(inner, false)?;
                let query = self.ts(&method.params[0].ty, true)?;
                c.line(format!(
                    "find(kind: {}, filter?: {query} | ((found: {found}) => unknown)): {found}[];",
                    quote(kind)
                ));
            }
            if !finds.is_empty() {
                c.line("find(kind: string, filter?: Record<string, unknown> | ((found: never) => unknown)): object[];");
            }
            if object.assign {
                let names: Vec<String> = assign_fields(object, None)
                    .iter()
                    .map(|property| member_name(&property.name, &property.extras.export.js))
                    .collect();
                self.assign_dts(c, &name, &names)?;
            }
            c.line("destroy(): void;");
        }
        c.dedent();
        c.line("}");
        c.blank();
        Ok(())
    }

    fn property_dts(&self, c: &mut CodeWriter, class: &str, property: &Property) -> Result<(), String> {
        c.jsdoc(&self.view.member_doc(class, &property.name));
        let ident = member_name(&property.name, &property.extras.export.js);
        let callback = matches!(property.ty, Type::Callback(_));
        let read = if callback {
            String::new()
        } else {
            self.ts_read_property(property)?
        };
        let write = self.ts(&property.ty, true)?;
        let mut lines: Vec<String> = Vec::new();
        if property.readonly {
            lines.push(format!("readonly {ident}: {read};"));
        } else {
            if callback {
                lines.push(format!("set {ident}(value: {write});"));
            } else if read == write {
                lines.push(format!("{ident}: {read};"));
            } else {
                lines.push(format!("get {ident}(): {read};"));
                lines.push(format!("set {ident}(value: {write});"));
            }
            if property.animated {
                lines.push(format!(
                    "set{}Timed(value: {write}, timing: Timing): void;",
                    pascal(&ident)
                ));
            }
        }
        for line in lines {
            self.view.declared(class, &property.name, &line);
            c.line(line);
        }
        Ok(())
    }

    fn interface_dts(&self, c: &mut CodeWriter, object: &ObjectType) -> Result<(), String> {
        let name = class_name(object);
        self.doc_or(c, self.view.class_doc(&object.name), || {
            format!(
                "The interface `{}` of `{}`: a script extends it and defines the methods it wants to answer.",
                object.name, self.view.namespace
            )
        });
        c.line(format!("export declare abstract class {name} {{"));
        c.indent();
        for method in &object.methods {
            c.jsdoc(&self.view.member_doc(&object.name, &method.name));
            let ident = member_name(&method.name, &method.extras.export.js);
            let returns = match &method.returns {
                // The script's method may answer nothing, which the library reads as the method's comment says.
                Some(ty) if method.optional => format!("{} | void", self.ts(ty, true)?),
                Some(ty) => self.ts(ty, true)?,
                None => "void".to_owned(),
            };
            let line = if method.required {
                format!("abstract {ident}({}): {returns};", self.params_dts(&method.params)?)
            } else {
                format!("{ident}?({}): {returns};", self.params_dts(&method.params)?)
            };
            self.view.declared(&object.name, &method.name, &line);
            c.line(line);
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

/// The property a record's `number` names, which a bare number stands for.
fn number_field(object: &ObjectType) -> Result<Option<&Property>, String> {
    object
        .number
        .as_ref()
        .map(|field| {
            object
                .properties
                .iter()
                .find(|property| &property.name == field)
                .ok_or_else(|| format!("`{}` has no number field `{field}`", object.name))
        })
        .transpose()
}

/// The key a variant is written under as an object with one key: its tag, as a member name (`blur`, `dropShadow`).
fn literal_key(variant: &gnoblin_schema::TaggedVariant) -> String {
    member_name(&variant.tag, &None)
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
    c.line("export { GnoblinError } from \"gnoblin-core/generated/object_base\";");
    c.line("export type { ErrorCode } from \"gnoblin-core/generated/object_base\";");
    for foreign in view.foreigns()? {
        c.line(format!(
            "import type {{ {} as {} }} from \"{}/generated/objects\";",
            foreign.js, foreign.alias, foreign.package
        ));
    }
    c.blank();
    for declared in view.enums.values() {
        let values: Vec<String> = declared.values.iter().map(|value| quote(value)).collect();
        for value in &declared.values {
            view.declared(&declared.name, value, quote(value));
        }
        js.doc_or(&mut c, view.enum_doc(&declared.name), || {
            format!("The enum `{}`.", declared.name)
        });
        if declared
            .values
            .iter()
            .any(|value| !view.enum_value_doc(&declared.name, value).is_empty())
        {
            c.line(format!("export type {} =", pascal(&declared.name)));
            c.indent();
            for (index, value) in declared.values.iter().enumerate() {
                c.jsdoc(view.enum_value_doc(&declared.name, value));
                let end = if index + 1 == declared.values.len() { ";" } else { "" };
                c.line(format!("| {}{end}", quote(value)));
            }
            c.dedent();
        } else {
            c.line(format!(
                "export type {} = {};",
                pascal(&declared.name),
                values.join(" | ")
            ));
        }
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
    for (class, target) in augments_of(view.catalog, &view.objects)? {
        js.augment_dts(&mut c, class, target)?;
    }
    Ok(c.finish())
}

/// How the declarations spell the types of a member, for the reference model: the property as it is read, a parameter or a field as it is
/// given, a result as it is returned.
pub(super) mod spelling {
    use gnoblin_schema::{Field, Method, Property};

    use super::Js;
    use crate::object_script::ScriptView;

    pub(crate) fn property(view: &ScriptView<'_>, property: &Property) -> Result<String, String> {
        Js { view }.ts_read_property(property)
    }

    pub(crate) fn input(view: &ScriptView<'_>, field: &Field) -> Result<String, String> {
        Js { view }.ts(&field.ty, true)
    }

    /// The type of the function a callback declares.
    pub(crate) fn callback(view: &ScriptView<'_>, name: &str) -> Result<String, String> {
        Js { view }.ts(&gnoblin_schema::Type::Callback(name.to_owned()), false)
    }

    pub(crate) fn returns(view: &ScriptView<'_>, method: &Method) -> Result<String, String> {
        let js = Js { view };
        Ok(match &method.returns {
            Some(ty) if method.optional => js.ts_read(ty)?,
            Some(ty) => js.ts(ty, false)?,
            None => "void".to_owned(),
        })
    }
}

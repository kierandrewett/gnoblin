// The style model of design/js-research/owner-spec.md section 6, over plain ES classes: `Style`, `ClientStyle`, the
// list elements (`Border`, `Shadow`, the filters) and the cascade that turns the layers a client sits under into one
// resolved style. Nothing here touches the compositor: `gnoblin/host` binds the scopes (`gnoblin.clients.style`, a
// workspace's, a `where()` selection's, one client's) and writes what `mapStyle` returns to the node. Read
// `docs/js-style.md` first for the user's view of it.
//
// A scope holds only what was set on it: a property that was never assigned, or was unset, is `undefined` and is
// inherited. `null` is a value (`background = null`). A list assigned to a scope replaces the inherited list whole.
//
// Changes are seen two ways, and no Proxy is involved. A field of an element or a scalar of a style is an accessor
// that checks the value, throws with the property path when it is bad, leaves the old value, and bumps a counter the
// host reads each frame. An array a script changes in place (`push`, `splice`, `style.shadows[0] = {...}`) cannot be
// intercepted, so `syncStyleLists` compares each array with the contents it had last time, once per frame, checks
// the new contents then, and puts the old ones back with a reported error when they are bad.
//
// Reading a list that a scope does not override returns a copy of the inherited list whose elements know they are
// copies. The first write to one of them (or the first in-place change of the array, seen by the frame comparison)
// makes that copy the scope's own list, so the write never reaches the scope the list came from.

// ---- Values and limits ----

/** A colour as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. */
export type Color = string;

/** One radius per corner, in pixels. */
export interface CornerRadii {
    topLeft: number;
    topRight: number;
    bottomRight: number;
    bottomLeft: number;
}

/** How many entries a style list holds, and how many effects a window's node carries. */
export const MAX_STYLE_LIST = 8;

/** The renderer's own limits (crates/gnoblin-render/src/effect.rs): a blur or shadow radius, a spread, a grade. */
const MAX_RADIUS = 100;
const MAX_GRADE = 2;

/** The text form of a colour: 3, 4, 6 or 8 hex digits after the `#`. */
const COLOUR = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

/** Bumped by every accepted change of any style object. The host re-resolves a client when it moved. */
let epoch = 0;
/** Above zero while elements are built, which are not changes. */
let building = 0;

/**
 * A number that goes up by one for every accepted change of any style object. The host compares it between frames
 * and re-resolves a client's style when it moved.
 */
export function styleEpoch(): number {
    return epoch;
}

// ---- Checkers: one per kind of value, each returns the checked value or throws with the property path ----

/** Throws a RangeError whose text starts with the property path, such as `Client#42.style.opacity`. */
function fail(path: string, message: string): never {
    throw new RangeError(`${path}: ${message}`);
}

/** Checks `value` for the property at `path`, and returns it (or its frozen, normalised form). */
type Checker<T = unknown> = (value: unknown, path: string) => T;

/** A checker for a finite number from `min` to `max`, both included. */
function finiteIn(min: number, max: number): Checker<number> {
    return (value, path) => {
        if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) {
            if (min === -Infinity && max === Infinity) {
                return fail(path, "must be a finite number");
            }
            return fail(
                path,
                max === Infinity ? `must be finite and >= ${min}` : `must be finite and between ${min} and ${max}`,
            );
        }
        return value;
    };
}

const FINITE = finiteIn(-Infinity, Infinity);
const NON_NEGATIVE = finiteIn(0, Infinity);
const RADIUS = finiteIn(0, MAX_RADIUS);
const SPREAD = finiteIn(-MAX_RADIUS, MAX_RADIUS);
const GRADE = finiteIn(0, MAX_GRADE);
const UNIT = finiteIn(0, 1);

const colour: Checker<Color> = (value, path) => {
    if (typeof value !== "string" || !COLOUR.test(value)) {
        return fail(path, "must be a #rgb, #rgba, #rrggbb or #rrggbbaa colour");
    }
    return value;
};

const flag: Checker<boolean> = (value, path) => {
    if (typeof value !== "boolean") {
        return fail(path, "must be true or false");
    }
    return value;
};

const RADII_KEYS: readonly (keyof CornerRadii)[] = ["topLeft", "topRight", "bottomRight", "bottomLeft"];

const cornerRadius: Checker<number | Readonly<CornerRadii>> = (value, path) => {
    if (typeof value === "number") {
        return NON_NEGATIVE(value, path);
    }
    if (value !== null && typeof value === "object" && !Array.isArray(value)) {
        const given = value as Record<string, unknown>;
        for (const key of Object.keys(given)) {
            if (!(RADII_KEYS as readonly string[]).includes(key)) {
                throw new TypeError(`${path}: no corner "${key}" (known: ${RADII_KEYS.join(", ")})`);
            }
        }
        const radii = {} as CornerRadii;
        for (const key of RADII_KEYS) {
            radii[key] = NON_NEGATIVE(given[key], `${path}.${key}`);
        }
        return Object.freeze(radii);
    }
    return fail(path, "must be a finite number >= 0 or { topLeft, topRight, bottomRight, bottomLeft }");
};

const background: Checker<Color | null> = (value, path) => (value === null ? null : colour(value, path));

// ---- List elements ----

/** Where a list element sits, for its error paths, and what runs before one of its fields is written. */
interface Home {
    list: StyleElement[] | null;
    path(element: StyleElement): string;
    beforeWrite?: () => void;
}

/** The home of each element that sits in a list. An element outside every list has none, and its path is its class. */
const homes = new WeakMap<StyleElement, Home>();

/** One accepted write of an element's field: the checked value, with the copy-on-write and liveness hooks run. */
function write<T>(element: StyleElement, name: string, value: unknown, check: Checker<T>): T {
    const home = homes.get(element);
    const checked = check(value, `${home ? home.path(element) : element.constructor.name}.${name}`);
    home?.beforeWrite?.();
    if (building === 0) {
        epoch += 1;
    }
    return checked;
}

/** Sets each field of the literal `init` through the element's checked setters; an unknown field is a TypeError. */
function assignFields(target: object, init: unknown, fields: readonly string[], label: string): void {
    if (init === undefined) {
        return;
    }
    if (init === null || typeof init !== "object" || Array.isArray(init)) {
        throw new TypeError(`${label}: expects an object of fields`);
    }
    for (const key of Object.keys(init)) {
        if (!fields.includes(key)) {
            throw new TypeError(`${label}: no field "${key}" (known: ${fields.join(", ")})`);
        }
        (target as Record<string, unknown>)[key] = (init as Record<string, unknown>)[key];
    }
}

/** What every list element shares: it can be copied and written out as plain data. */
export abstract class StyleElement {
    abstract toJSON(): Record<string, unknown>;
}

/** The fields of a `Border`, in the order `toJSON` writes them. */
const BORDER_FIELDS = ["width", "color", "offset", "inset"] as const;

/** A `Border` as a literal: any field may be left out. */
export interface BorderInit {
    width?: number;
    color?: Color;
    offset?: number;
    inset?: boolean;
}

/**
 * One stroke around the window. Several make concentric rings, `offset` apart. `width` and `offset` are in pixels;
 * `offset` moves the ring out from the frame (negative moves it in).
 */
export class Border extends StyleElement {
    #width = 1;
    #color: Color = "#ffffff";
    #offset = 0;
    #inset = false;

    constructor(init?: BorderInit) {
        super();
        assignFields(this, init, BORDER_FIELDS, "Border");
    }

    get width(): number {
        return this.#width;
    }

    set width(value: number) {
        this.#width = write(this, "width", value, NON_NEGATIVE);
    }

    get color(): Color {
        return this.#color;
    }

    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    get offset(): number {
        return this.#offset;
    }

    set offset(value: number) {
        this.#offset = write(this, "offset", value, FINITE);
    }

    get inset(): boolean {
        return this.#inset;
    }

    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

    toJSON(): { width: number; color: Color; offset: number; inset: boolean } {
        return { width: this.#width, color: this.#color, offset: this.#offset, inset: this.#inset };
    }
}

/** The fields of a `Shadow`, in the order `toJSON` writes them. */
const SHADOW_FIELDS = ["x", "y", "blur", "spread", "color", "inset"] as const;

/** A `Shadow` as a literal: any field may be left out. Offsets, blur and spread are in pixels. */
export interface ShadowInit {
    x?: number;
    y?: number;
    blur?: number;
    spread?: number;
    color?: Color;
    inset?: boolean;
}

/**
 * A shadow behind the window, or inside it with `inset`. `x` and `y` move it, `blur` softens its edge and `spread`
 * grows it, all in pixels.
 */
export class Shadow extends StyleElement {
    #x = 0;
    #y = 0;
    #blur = 12;
    #spread = 0;
    #color: Color = "#00000080";
    #inset = false;

    constructor(init?: ShadowInit) {
        super();
        assignFields(this, init, SHADOW_FIELDS, "Shadow");
    }

    get x(): number {
        return this.#x;
    }

    set x(value: number) {
        this.#x = write(this, "x", value, FINITE);
    }

    get y(): number {
        return this.#y;
    }

    set y(value: number) {
        this.#y = write(this, "y", value, FINITE);
    }

    get blur(): number {
        return this.#blur;
    }

    set blur(value: number) {
        this.#blur = write(this, "blur", value, RADIUS);
    }

    get spread(): number {
        return this.#spread;
    }

    set spread(value: number) {
        this.#spread = write(this, "spread", value, SPREAD);
    }

    get color(): Color {
        return this.#color;
    }

    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    get inset(): boolean {
        return this.#inset;
    }

    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

    toJSON(): { x: number; y: number; blur: number; spread: number; color: Color; inset: boolean } {
        return {
            x: this.#x,
            y: this.#y,
            blur: this.#blur,
            spread: this.#spread,
            color: this.#color,
            inset: this.#inset,
        };
    }
}

/** A `BlurFilter` as a literal. `radius` is in pixels. */
export interface BlurFilterInit {
    radius?: number;
}

/** A Gaussian-style blur. Of a `backdropFilter` it blurs what is behind the window. */
export class BlurFilter extends StyleElement {
    #radius = 12;

    constructor(init?: BlurFilterInit) {
        super();
        assignFields(this, init, ["radius"], "BlurFilter");
    }

    get radius(): number {
        return this.#radius;
    }

    set radius(value: number) {
        this.#radius = write(this, "radius", value, RADIUS);
    }

    toJSON(): { kind: "blur"; radius: number } {
        return { kind: "blur", radius: this.#radius };
    }
}

/** A `SaturateFilter`, `BrightnessFilter` or `ContrastFilter` as a literal: `amount` is 1 for no change. */
export interface AmountInit {
    amount?: number;
}

/** The colour saturation, 1 unchanged and 0 grey, up to 2. */
export class SaturateFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "SaturateFilter");
    }

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    toJSON(): { kind: "saturate"; amount: number } {
        return { kind: "saturate", amount: this.#amount };
    }
}

/** The brightness gain, 1 unchanged, up to 2. */
export class BrightnessFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "BrightnessFilter");
    }

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    toJSON(): { kind: "brightness"; amount: number } {
        return { kind: "brightness", amount: this.#amount };
    }
}

/** The contrast gain about mid grey, 1 unchanged, up to 2. */
export class ContrastFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "ContrastFilter");
    }

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    toJSON(): { kind: "contrast"; amount: number } {
        return { kind: "contrast", amount: this.#amount };
    }
}

/** Any filter element of a `filter` or `backdropFilter` list. */
export type Filter = BlurFilter | SaturateFilter | BrightnessFilter | ContrastFilter;

/** The shorthand a filter list accepts: `{ blur: 18 }` is a `BlurFilter` of radius 18, and so on. */
export type FilterInit =
    Filter | { blur: number } | { saturate: number } | { brightness: number } | { contrast: number };

/** A border element or the literal that makes one, as a `borders` list takes it. */
export type BorderLike = Border | BorderInit;
/** A shadow element or the literal that makes one, as a `shadows` list takes it. */
export type ShadowLike = Shadow | ShadowInit;

/** The one-key literals a filter list accepts, each with the element it makes and the checker of its number. */
const FILTER_SHORTHAND = {
    blur: { make: (n: number) => new BlurFilter({ radius: n }), check: RADIUS },
    saturate: { make: (n: number) => new SaturateFilter({ amount: n }), check: GRADE },
    brightness: { make: (n: number) => new BrightnessFilter({ amount: n }), check: GRADE },
    contrast: { make: (n: number) => new ContrastFilter({ amount: n }), check: GRADE },
} as const;

/** A list element class, as `entryOf` constructs it. */
type ElementClass = new (init?: never) => StyleElement;

/** A copy of `element` that no list owns. Built under `building`, so making it is not a change. */
function cloneElement(element: StyleElement): StyleElement {
    const Class = element.constructor as new (init: unknown) => StyleElement;
    building += 1;
    try {
        // A filter writes its `kind` out for the reader of `toJSON`; the constructor takes only its fields.
        const { kind: _kind, ...fields } = element.toJSON();
        return new Class(fields);
    } finally {
        building -= 1;
    }
}

/** Whether `value` is one of the four filter classes. */
function isFilter(value: unknown): value is Filter {
    return (
        value instanceof BlurFilter ||
        value instanceof SaturateFilter ||
        value instanceof BrightnessFilter ||
        value instanceof ContrastFilter
    );
}

/** The element a list entry stands for: an element of that class (copied when another list owns it) or a literal. */
function entryOf(
    value: unknown,
    path: string,
    target: StyleElement[],
    Class: ElementClass,
    fields: readonly string[],
): StyleElement {
    if (value instanceof Class) {
        const home = homes.get(value);
        return home?.list && home.list !== target ? cloneElement(value) : value;
    }
    if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof StyleElement) {
        throw new TypeError(`${path}: must be a ${Class.name} or an object of its fields`);
    }
    building += 1;
    try {
        const element = new Class();
        homes.set(element, { list: null, path: () => path });
        assignFields(element, value, fields, path);
        return element;
    } finally {
        building -= 1;
    }
}

/** The element a `filter` or `backdropFilter` entry stands for: a filter element or a one-key literal. */
function filterEntry(value: unknown, path: string, target: StyleElement[]): StyleElement {
    if (isFilter(value)) {
        const home = homes.get(value);
        return home?.list && home.list !== target ? cloneElement(value) : value;
    }
    if (value !== null && typeof value === "object" && !Array.isArray(value) && !(value instanceof StyleElement)) {
        const keys = Object.keys(value);
        const key = keys[0] as keyof typeof FILTER_SHORTHAND;
        if (keys.length === 1 && Object.hasOwn(FILTER_SHORTHAND, key)) {
            const entry = FILTER_SHORTHAND[key];
            const amount = entry.check((value as Record<string, unknown>)[key], `${path}.${key}`);
            building += 1;
            try {
                return entry.make(amount);
            } finally {
                building -= 1;
            }
        }
    }
    throw new TypeError(
        `${path}: must be a filter: { blur: 18 }, { saturate: 1.2 }, { brightness: 1.1 }, { contrast: 1.1 } or a ` +
            "BlurFilter, SaturateFilter, BrightnessFilter or ContrastFilter",
    );
}

// ---- Stores: what each scope holds and how its lists are copied on write ----

/** The properties that are lists. */
type ListName = "borders" | "shadows" | "filter" | "backdropFilter";
/** The properties that are one value. */
type ScalarName = "opacity" | "cornerRadius" | "background";
/** Any style property, as `Style.unset` takes it. */
export type StyleProperty = ListName | ScalarName;

const LIST_ENTRY: Readonly<Record<ListName, (value: unknown, path: string, target: StyleElement[]) => StyleElement>> = {
    borders: (value, path, target) => entryOf(value, path, target, Border as ElementClass, BORDER_FIELDS),
    shadows: (value, path, target) => entryOf(value, path, target, Shadow as ElementClass, SHADOW_FIELDS),
    filter: filterEntry,
    backdropFilter: filterEntry,
};

/** The checker of each scalar property. */
const SCALAR_CHECK: Readonly<Record<ScalarName, Checker>> = {
    opacity: UNIT,
    cornerRadius,
    background,
};

/** The properties every `Style` has, and the ones a `ClientStyle` adds. */
const STYLE_FIELDS: readonly string[] = ["opacity", "filter", "backdropFilter"];
const CLIENT_FIELDS: readonly string[] = [...STYLE_FIELDS, "cornerRadius", "borders", "shadows", "background"];

/** What a scope tells its style: where it is, what it inherits, and whether it may still be written. */
export interface StyleScope {
    /** `Client#42.style`, `gnoblin.clients.style`: the front of every error path. */
    path(): string;
    /** The list the layers below this scope resolve to, as live elements. */
    below(name: ListName): readonly StyleElement[];
    /** Throws when the scope is gone (a destroyed client). */
    guard?(): void;
}

/** A copy of an inherited list handed to a script, valid while the epoch has not moved. */
interface View {
    epoch: number;
    list: StyleElement[];
    /** The entries `list` had when it was handed out. */
    seen: StyleElement[];
}

/** What one style object holds. It lives in a WeakMap so that a script cannot reach it through the object. */
interface Store {
    label: string;
    scope: StyleScope | null;
    scalars: Map<ScalarName, unknown>;
    lists: Map<ListName, StyleElement[]>;
    /** The entries each override list had when last checked. */
    seen: Map<ListName, StyleElement[]>;
    views: Map<ListName, View>;
}

const stores = new WeakMap<Style, Store>();

/** The store of `style`; a TypeError when `style` is not one of ours. */
function storeOf(style: Style): Store {
    const store = stores.get(style);
    if (store === undefined) {
        throw new TypeError("not a style object");
    }
    return store;
}

/** The error path of property `name` of the style held by `store`. */
function pathOf(store: Store, name: string): string {
    return `${store.scope?.path() ?? store.label}.${name}`;
}

/** Whether two lists hold the same element objects in the same order. */
function sameEntries(a: readonly StyleElement[], b: readonly StyleElement[]): boolean {
    return a.length === b.length && a.every((element, at) => element === b[at]);
}

/** Makes `list` the entries of override list `name`, with every element knowing where it sits. */
function own(store: Store, name: ListName, list: StyleElement[]): void {
    for (const element of list) {
        homes.set(element, {
            list,
            path: (e) => `${pathOf(store, name)}[${list.indexOf(e)}]`,
            beforeWrite: () => store.scope?.guard?.(),
        });
    }
    store.lists.set(name, list);
    store.seen.set(name, [...list]);
    store.views.delete(name);
}

/** The first write to a copy of an inherited list: the copy becomes the scope's own list. */
function commitView(store: Store, name: ListName, view: View): void {
    store.scope?.guard?.();
    if (store.lists.has(name) || store.views.get(name) !== view) {
        return;
    }
    own(store, name, view.list);
    epoch += 1;
}

/** `value` as a list of elements for property `name`, each entry checked; the first bad entry throws with its path. */
function checkedList(store: Store, name: ListName, value: unknown, target: StyleElement[]): StyleElement[] {
    const path = pathOf(store, name);
    if (!Array.isArray(value)) {
        throw new TypeError(`${path}: must be an array`);
    }
    if (value.length > MAX_STYLE_LIST) {
        return fail(path, `holds at most ${MAX_STYLE_LIST} entries, got ${value.length}`);
    }
    const entries: StyleElement[] = [];
    for (let at = 0; at < value.length; at += 1) {
        entries.push(LIST_ENTRY[name](value[at], `${path}[${at}]`, target));
    }
    return entries;
}

/** The setter of a list property: check every entry, then make the new list the scope's own. */
function setList(style: Style, name: ListName, value: unknown): void {
    const store = storeOf(style);
    store.scope?.guard?.();
    if (value === undefined) {
        unsetOne(store, name);
        return;
    }
    const target: StyleElement[] = [];
    const entries = checkedList(store, name, value, target);
    target.push(...entries);
    own(store, name, target);
    epoch += 1;
}

/**
 * The getter of a list property. A scope that overrides the list returns its own. Otherwise the script gets a copy
 * of the inherited list (cached for one epoch) whose first write makes the copy the scope's own.
 */
function getList(style: Style, name: ListName): StyleElement[] {
    const store = storeOf(style);
    const overriding = store.lists.get(name);
    if (overriding !== undefined) {
        return overriding;
    }
    const held = store.views.get(name);
    if (held !== undefined && held.epoch === epoch) {
        return held.list;
    }
    const list: StyleElement[] = [];
    const view: View = { epoch, list, seen: list };
    building += 1;
    try {
        for (const inherited of store.scope?.below(name) ?? []) {
            const copy = cloneElement(inherited);
            homes.set(copy, {
                list,
                path: (e) => `${pathOf(store, name)}[${list.indexOf(e)}]`,
                beforeWrite: () => commitView(store, name, view),
            });
            list.push(copy);
        }
    } finally {
        building -= 1;
    }
    view.seen = [...list];
    store.views.set(name, view);
    return list;
}

/** The setter of a scalar property; `undefined` takes it back to inheriting. */
function setScalar(style: Style, name: ScalarName, value: unknown): void {
    const store = storeOf(style);
    store.scope?.guard?.();
    if (value === undefined) {
        unsetOne(store, name);
        return;
    }
    store.scalars.set(name, SCALAR_CHECK[name](value, pathOf(store, name)));
    epoch += 1;
}

/** Takes one property back to inheriting, in whichever store it sits. */
function unsetOne(store: Store, name: StyleProperty): void {
    store.scope?.guard?.();
    store.scalars.delete(name as ScalarName);
    store.lists.delete(name as ListName);
    store.seen.delete(name as ListName);
    store.views.delete(name as ListName);
    epoch += 1;
}

/**
 * The defaults one scope sets, and the ones it leaves to the scopes below it. `opacity`, `filter` and
 * `backdropFilter` are on every style; `ClientStyle` adds the rest. Assigning `undefined` is the same as `unset`.
 */
export class Style {
    constructor(init?: object) {
        stores.set(this, {
            label: this.constructor.name,
            scope: null,
            scalars: new Map(),
            lists: new Map(),
            seen: new Map(),
            views: new Map(),
        });
        assignFields(this, init, this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS, this.constructor.name);
    }

    /** From 0 (invisible) to 1. Written to the window node's `opacity`. */
    get opacity(): number | undefined {
        return storeOf(this).scalars.get("opacity") as number | undefined;
    }

    set opacity(value: number | undefined) {
        setScalar(this, "opacity", value);
    }

    /** The chain applied to the window's own content. No renderer draws it yet. */
    get filter(): Filter[] {
        return getList(this, "filter") as Filter[];
    }

    set filter(value: FilterInit[] | undefined) {
        setList(this, "filter", value);
    }

    /** The chain applied to what is behind the window. A blur in it is the one the compositor draws. */
    get backdropFilter(): Filter[] {
        return getList(this, "backdropFilter") as Filter[];
    }

    set backdropFilter(value: FilterInit[] | undefined) {
        setList(this, "backdropFilter", value);
    }

    /** Takes the named properties back to inheriting. An unknown name is a `TypeError`. */
    unset(...names: StyleProperty[]): void {
        const store = storeOf(this);
        const known = this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS;
        for (const name of names) {
            if (!known.includes(name)) {
                throw new TypeError(
                    `${pathOf(store, "unset")}: no property "${String(name)}" (known: ${known.join(", ")})`,
                );
            }
        }
        for (const name of names) {
            unsetOne(store, name);
        }
    }

    /** Takes every property back to inheriting. */
    reset(): void {
        this.unset(
            ...(this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS).map((name) => name as StyleProperty),
        );
    }

    /** What this scope sets, as plain data: a property it inherits is absent. */
    toJSON(): Record<string, unknown> {
        const store = storeOf(this);
        const out: Record<string, unknown> = {};
        for (const [name, value] of store.scalars) {
            out[name] = value;
        }
        for (const [name, list] of store.lists) {
            out[name] = list.map((element) => element.toJSON());
        }
        return out;
    }
}

/** A `ClientStyle` as a literal: any property may be left out. */
export interface ClientStyleInit {
    opacity?: number;
    cornerRadius?: number | CornerRadii;
    background?: Color | null;
    borders?: BorderLike[];
    shadows?: ShadowLike[];
    filter?: FilterInit[];
    backdropFilter?: FilterInit[];
}

/** The style of a client, or the defaults a scope of clients gives. */
export class ClientStyle extends Style {
    constructor(init?: ClientStyleInit) {
        super(init);
    }

    /** A number, or one radius per corner. Not drawn by any renderer yet. */
    get cornerRadius(): number | Readonly<CornerRadii> | undefined {
        return storeOf(this).scalars.get("cornerRadius") as number | Readonly<CornerRadii> | undefined;
    }

    set cornerRadius(value: number | CornerRadii | undefined) {
        setScalar(this, "cornerRadius", value);
    }

    /** A colour or `null` (no background). Not drawn by any renderer yet. */
    get background(): Color | null | undefined {
        return storeOf(this).scalars.get("background") as Color | null | undefined;
    }

    set background(value: Color | null | undefined) {
        setScalar(this, "background", value);
    }

    /** Not drawn by any renderer yet. */
    get borders(): Border[] {
        return getList(this, "borders") as Border[];
    }

    set borders(value: BorderLike[] | undefined) {
        setList(this, "borders", value);
    }

    /** Behind the window. The first entry is on top. */
    get shadows(): Shadow[] {
        return getList(this, "shadows") as Shadow[];
    }

    set shadows(value: ShadowLike[] | undefined) {
        setList(this, "shadows", value);
    }
}

// ---- for the host: scopes, the frame comparison, the cascade ------------------------------------------------------

/** Ties a style to the scope it belongs to, so its error paths and its inherited lists are the scope's. */
export function bindStyle(style: Style, scope: StyleScope): void {
    storeOf(style).scope = scope;
}

/** A style that checks `init` and is thrown away: the validation of a literal, errors under `label`. */
export function checkStyleInit(init: unknown, label: string): ClientStyle {
    const style = new ClientStyle();
    storeOf(style).label = label;
    assignFields(style, init, CLIENT_FIELDS, label);
    return style;
}

/** Makes `target` hold what `source` (a style or a literal) sets, and nothing else; on a bad literal changes nothing. */
export function replaceStyle(target: ClientStyle, source: unknown): void {
    const staged = source instanceof Style ? source : checkStyleInit(source, storeOf(target).scope?.path() ?? "style");
    const from = storeOf(staged);
    target.reset();
    for (const [name, value] of from.scalars) {
        setScalar(target, name, value);
    }
    for (const [name, list] of from.lists) {
        setList(target, name, list);
    }
}

/**
 * Compares every array of `style` that a script may have changed in place with what it held last time. Entries
 * that were added get checked and made elements; a bad change is undone and returned as an error, and a change of
 * a copy of an inherited list makes the copy the scope's own. Cheap when nothing moved.
 */
export function syncStyleLists(style: Style): Error[] {
    const store = storeOf(style);
    const errors: Error[] = [];
    for (const [name, list] of store.lists) {
        const seen = store.seen.get(name) ?? [];
        if (!sameEntries(list, seen)) {
            recheck(store, name, list, seen, errors, () => own(store, name, list));
        }
    }
    for (const [name, view] of store.views) {
        if (!sameEntries(view.list, view.seen)) {
            recheck(store, name, view.list, view.seen, errors, () => commitView(store, name, view));
        }
    }
    return errors;
}

/** Checks the list that changed in place; on success `accept` runs, on failure the old entries come back. */
function recheck(
    store: Store,
    name: ListName,
    list: StyleElement[],
    seen: StyleElement[],
    errors: Error[],
    accept: () => void,
): void {
    try {
        store.scope?.guard?.();
        const entries = checkedList(store, name, [...list], list);
        list.splice(0, list.length, ...entries);
        accept();
        epoch += 1;
    } catch (error) {
        list.splice(0, list.length, ...seen);
        errors.push(error instanceof Error ? error : new Error(String(error)));
    }
}

/** One layer of the cascade, with the name that `computedStyle` reports for the values it supplies. */
export interface StyleLayer {
    origin: string;
    style: Style;
}

/** The elements of `name` the highest layer that overrides it holds, or none. */
export function layeredList(layers: readonly StyleLayer[], name: ListName): readonly StyleElement[] {
    for (let at = layers.length - 1; at >= 0; at -= 1) {
        const list = storeOf(layers[at].style).lists.get(name);
        if (list !== undefined) {
            return list;
        }
    }
    return [];
}

/** A resolved border, as plain frozen data. */
export interface BorderValue {
    readonly width: number;
    readonly color: Color;
    readonly offset: number;
    readonly inset: boolean;
}

/** A resolved shadow, as plain frozen data. */
export interface ShadowValue {
    readonly x: number;
    readonly y: number;
    readonly blur: number;
    readonly spread: number;
    readonly color: Color;
    readonly inset: boolean;
}

/** A resolved filter, as plain frozen data. */
export type FilterValue =
    | { readonly kind: "blur"; readonly radius: number }
    | { readonly kind: "saturate"; readonly amount: number }
    | { readonly kind: "brightness"; readonly amount: number }
    | { readonly kind: "contrast"; readonly amount: number };

/** A client's resolved style: frozen data, and which layer each property came from. */
export interface ComputedStyle {
    readonly opacity: number;
    readonly cornerRadius: number | Readonly<CornerRadii>;
    readonly background: Color | null;
    readonly borders: readonly BorderValue[];
    readonly shadows: readonly ShadowValue[];
    readonly filter: readonly FilterValue[];
    readonly backdropFilter: readonly FilterValue[];
    /** `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`, for every property. */
    readonly origin: Readonly<Record<StyleProperty, string>>;
}

/** What a property resolves to when no layer sets it. */
const DEFAULT_SCALARS: Readonly<Record<ScalarName, unknown>> = { opacity: 1, cornerRadius: 0, background: null };

/** The style the layers resolve to, lowest layer first, per property: the highest layer that sets it wins. */
export function resolveStyle(layers: readonly StyleLayer[]): ComputedStyle {
    const origin = {} as Record<StyleProperty, string>;
    const resolved: Record<string, unknown> = {};
    for (const name of Object.keys(SCALAR_CHECK) as ScalarName[]) {
        origin[name] = "default";
        resolved[name] = DEFAULT_SCALARS[name];
        for (let at = layers.length - 1; at >= 0; at -= 1) {
            const scalars = storeOf(layers[at].style).scalars;
            if (scalars.has(name)) {
                resolved[name] = scalars.get(name);
                origin[name] = layers[at].origin;
                break;
            }
        }
    }
    for (const name of Object.keys(LIST_ENTRY) as ListName[]) {
        origin[name] = "default";
        resolved[name] = Object.freeze([]);
        for (let at = layers.length - 1; at >= 0; at -= 1) {
            const list = storeOf(layers[at].style).lists.get(name);
            if (list !== undefined) {
                resolved[name] = Object.freeze(list.map((element) => Object.freeze(element.toJSON())));
                origin[name] = layers[at].origin;
                break;
            }
        }
    }
    return Object.freeze({ ...resolved, origin: Object.freeze(origin) }) as ComputedStyle;
}

// ---- the node's effects -------------------------------------------------------------------------------------------

/** One `effect` of the tree's node `effects` list, with the schema's property names (effect.toml). */
export interface EffectWire {
    kind: "blur" | "shadow" | "border";
    downscale: number;
    passes: number;
    algorithm: string;
    noise: number;
    brightness: number;
    contrast: number;
    saturation: number;
    tint: string;
    offset_x: number;
    offset_y: number;
    blur_radius: number;
    spread: number;
    colour: string;
    width: number;
    offset: number;
    inset: boolean;
    frame: boolean;
}

/** The compositor's fixed backdrop blur radius (crates/gnoblin-compositor/src/ipc/tree_effects.rs, `BLUR_RADIUS`). */
export const BACKDROP_BLUR_RADIUS = 12;

/** The wire's documented defaults for the parameters a kind does not use. */
const BLANK: Omit<EffectWire, "kind"> = {
    downscale: 2,
    passes: 3,
    algorithm: "kawase",
    noise: 0,
    brightness: 1,
    contrast: 1,
    saturation: 1,
    tint: "",
    offset_x: 0,
    offset_y: 0,
    blur_radius: 12,
    spread: 0,
    colour: "#00000080",
    width: 0,
    offset: 0,
    inset: false,
    frame: false,
};

/** A colour as the wire spells it: `#rrggbbaa`, lower case, with a full alpha added to a six-digit colour. */
function wireColour(text: Color): string {
    let digits = text.slice(1).toLowerCase();
    if (digits.length <= 4) {
        digits = [...digits].map((digit) => digit + digit).join("");
    }
    return `#${digits.length === 6 ? `${digits}ff` : digits}`;
}

/** What a resolved style becomes on the node, and what of it nothing draws. */
export interface StyleMapping {
    /**
     * In paint order, first to last: the backdrop blur, the shadows from the last listed to the first, then the borders
     * in list order.
     */
    effects: EffectWire[];
    /** The node's `corner_radius`, in logical pixels: the one radius the style asks for, or 0. */
    cornerRadius: number;
    /** The properties of this style the tree cannot express, one text each, `<property>: <reason>`. */
    unapplied: string[];
}

/** The radius a style's `cornerRadius` makes on the node, and the reason when the node cannot say it exactly. */
function nodeRadius(radius: ComputedStyle["cornerRadius"]): { radius: number; note: string | null } {
    if (typeof radius === "number") {
        return { radius, note: null };
    }
    const corners = [radius.topLeft, radius.topRight, radius.bottomRight, radius.bottomLeft];
    const largest = Math.max(...corners);
    return {
        radius: largest,
        note: corners.every((corner) => corner === largest)
            ? null
            : `cornerRadius: a window has one radius for every corner, so the largest of the four (${largest}) is drawn`,
    };
}

/**
 * What a style becomes on the node: a node `corner_radius`, and the `effects` list. Outer `shadows` are `shadow`
 * effects that follow the window's whole frame (the titlebar included), `borders` are `border` effects, and the
 * `backdropFilter` is one backdrop blur whose saturation, brightness and contrast are the products of the chain's.
 * `background`, `filter` and inset shadows stay in `computedStyle`; each comes back as an `unapplied` text when the
 * style has a non-default value for it. Whether the active renderer draws a kind is the compositor's to say: what it
 * leaves out is in the node's `skipped` list (`Client#styleWarnings`), not here.
 */
export function mapStyle(computed: ComputedStyle): StyleMapping {
    const effects: EffectWire[] = [];
    const unapplied: string[] = [];
    const blurs = computed.backdropFilter.filter((f): f is Extract<FilterValue, { kind: "blur" }> => f.kind === "blur");
    const grade = (kind: FilterValue["kind"]): number =>
        computed.backdropFilter.reduce((acc, f) => (f.kind === kind && "amount" in f ? acc * f.amount : acc), 1);
    const [saturation, brightness, contrast] = [grade("saturate"), grade("brightness"), grade("contrast")];
    if (blurs.some((f) => f.radius > 0)) {
        if (blurs.some((f) => f.radius > 0 && f.radius !== BACKDROP_BLUR_RADIUS)) {
            unapplied.push(
                `backdropFilter: the compositor blurs at a fixed radius of ${BACKDROP_BLUR_RADIUS}, so a BlurFilter's radius only switches the blur on`,
            );
        }
        const clamped = (n: number): number => Math.min(MAX_GRADE, n);
        if ([saturation, brightness, contrast].some((n) => n > MAX_GRADE)) {
            unapplied.push(
                `backdropFilter: a combined saturate, brightness or contrast above ${MAX_GRADE} is drawn as ${MAX_GRADE}`,
            );
        }
        effects.push({
            ...BLANK,
            kind: "blur",
            saturation: clamped(saturation),
            brightness: clamped(brightness),
            contrast: clamped(contrast),
        });
    } else if ([saturation, brightness, contrast].some((n) => n !== 1)) {
        unapplied.push(
            "backdropFilter: saturate, brightness and contrast are drawn with the backdrop blur, and there is none in the chain",
        );
    }
    for (const shadow of [...computed.shadows].reverse()) {
        if (shadow.inset) {
            continue;
        }
        effects.push({
            ...BLANK,
            kind: "shadow",
            offset_x: shadow.x,
            offset_y: shadow.y,
            blur_radius: shadow.blur,
            spread: shadow.spread,
            colour: wireColour(shadow.color),
            // A window's shadow is the whole frame's, titlebar and all, not the content's alone.
            frame: true,
        });
    }
    if (computed.shadows.some((shadow) => shadow.inset)) {
        unapplied.push("shadows: an inset shadow has no effect kind");
    }
    for (const border of computed.borders) {
        effects.push({
            ...BLANK,
            kind: "border",
            width: border.width,
            offset: border.offset,
            inset: border.inset,
            colour: wireColour(border.color),
        });
    }
    const { radius, note } = nodeRadius(computed.cornerRadius);
    if (note !== null) {
        unapplied.push(note);
    }
    if (computed.background !== null) {
        unapplied.push("background: the tree has no background for a window");
    }
    if (computed.filter.length > 0) {
        unapplied.push("filter: no effect kind filters a window's own content");
    }
    return { effects, cornerRadius: radius, unapplied };
}

/**
 * Takes the state of the styles `current()` yields (the scalars, the lists and each list element's fields) and returns
 * what puts it back. A style that `current()` yields only later (made after the snapshot) is emptied. A failed
 * `requestTransition` uses it so that the style writes of its callback leave nothing behind. A list handed out before is
 * not the list that comes back: the script reads it again.
 */
export function snapshotStyles(current: () => Iterable<Style>): () => void {
    interface Saved {
        scalars: Map<ScalarName, unknown>;
        lists: Map<ListName, { element: StyleElement; fields: Record<string, unknown> }[]>;
    }
    const saved = new Map<Style, Saved>();
    for (const style of current()) {
        const store = storeOf(style);
        saved.set(style, {
            scalars: new Map(store.scalars),
            lists: new Map(
                [...store.lists].map(([name, list]) => [
                    name,
                    list.map((element) => ({ element, fields: element.toJSON() })),
                ]),
            ),
        });
    }
    return () => {
        for (const style of current()) {
            const store = storeOf(style);
            const before = saved.get(style);
            store.scalars = new Map(before?.scalars);
            store.lists = new Map();
            store.seen = new Map();
            store.views = new Map();
            for (const [name, entries] of before?.lists ?? []) {
                own(
                    store,
                    name,
                    entries.map((entry) => entry.element),
                );
                for (const { element, fields } of entries) {
                    // A filter writes its `kind` out for the reader of `toJSON`; it is not a field to write back.
                    const { kind: _kind, ...values } = fields;
                    building += 1;
                    try {
                        Object.assign(element, values);
                    } finally {
                        building -= 1;
                    }
                }
            }
        }
        epoch += 1;
    };
}

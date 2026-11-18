// The base of the stock layouts that the library computes: `libgnoblin-std` holds the algorithm, this class holds what a script sees.
//
// A stock layout of this package is the library's built-in layout of its kind (`gnoblin-std/generated`'s `Builtin`, made by
// `builtin_new`) behind the `Layout` class a script extends and a workspace drives. The script's options are plain own fields, as
// they always were, and the workspace compares them between frames; the first compute makes the library's layout with their values,
// and a later one tells it what changed. The clients are the script's own objects: the library knows them by numbers this class
// gives them and keeps, so a layout that keeps a tree of them (a dwindle) remembers which is which.
//
// Nothing here decides where a client goes, what an option may be, or what an action does: the library's table of layout kinds
// (`gnoblin-std-abi`'s `LAYOUT_KINDS`) is the one declaration, and the library refuses a value it does not accept. What this class
// adds is the spelling: an option is `masterCount` here and `master_count` there, and a member of an enumeration is the word a script
// has always written (`"left"`) for the library's number.

import { Builtin } from "gnoblin-std/generated/classes";
import { PlacementFlags, QueryFlags, ValueKind } from "gnoblin-std/generated/constants";
import type { Option, Placement as LibraryPlacement, Split } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import type { Point } from "gnoblin-core/bindings/types";
import type { LayoutSplit } from "gnoblin-std/bindings/tiling";
import { Layout } from "gnoblin-std/layout";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";

/** What the library's actions take of a workspace: the nodes it holds, and which of them has the keyboard. */
export interface ActionSource {
    /** The nodes the layout places, in the workspace's order. A node with a `focused` flag that is true has the keyboard. */
    readonly clients: Iterable<LayoutNode>;
}

/** The size of a node with its frame, for an action that keeps a window's own size. */
export interface OwnSize {
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
}

/** What a script's option is called in the library, and the words of its enumeration when it has one. */
export interface OptionSpec {
    /** The option's name in the library's table of layout kinds (`master_count`). */
    readonly native: string;
    /** The members of an enumeration option, by the word a script writes (`left`) in upper case (`LEFT`). */
    readonly members?: Readonly<Record<string, number>>;
    /** The library's value for the script's (`"auto"` columns are 0). */
    readonly toNative?: (value: unknown) => unknown;
    /** The script's value for the library's. */
    readonly fromNative?: (value: unknown) => unknown;
    /** True for an option only the script changes (a function the library is told the meaning of): it is not read back. */
    readonly readOnly?: boolean;
}

/** The options of a layout, by the name of the script's own field. */
export type OptionSpecs = Readonly<Record<string, OptionSpec>>;

/** The numbers the library knows the script's clients by: one for each object, kept as long as the object lives. */
const known = new WeakMap<object, number>();
let issued = 0;

/** The number of `node`, given on first use. */
export function numberOf(node: object): number {
    let found = known.get(node);
    if (found === undefined) {
        issued += 1;
        found = issued;
        known.set(node, found);
    }
    return found;
}

/** One argument of an action as the library takes it: absent when `value` is `undefined` or `null`, a member of `members` by its word. */
export function argOf(name: string, value: unknown, members?: Readonly<Record<string, number>>): Init<Option> {
    if (value === undefined || value === null) {
        return { kind: ValueKind.NONE, name };
    }
    if (typeof value === "boolean") {
        return { kind: ValueKind.BOOL, flag: value ? 1 : 0, name };
    }
    if (typeof value === "string" && members !== undefined) {
        const member = members[value.toUpperCase()];
        return member === undefined ? { kind: ValueKind.TEXT, text: value, name } : { kind: ValueKind.ENUM, number: member, name };
    }
    if (typeof value === "number") {
        return { kind: ValueKind.NUMBER, number: value, name };
    }
    if (typeof value === "string") {
        return { kind: ValueKind.TEXT, text: value, name };
    }
    return { kind: ValueKind.TEXT, text: String(value), name };
}

/** A node as an argument of an action: the number the library knows it by. */
export function nodeArgOf(name: string, node: LayoutNode): Init<Option> {
    return { kind: ValueKind.NODE, node: numberOf(node), name };
}

/** The word of a member of an enumeration, or the number when the library knows none. */
function wordOf(spec: OptionSpec, value: number): string | number {
    const found = Object.entries(spec.members ?? {}).find(([, member]) => member === value);
    return found === undefined ? value : found[0].toLowerCase();
}

/** One option as the library takes it: `undefined` and `null` clear an option that may be absent. */
function optionOf(spec: OptionSpec, owner: string, field: string, given: unknown): Init<Option> {
    const name = spec.native;
    const value = spec.toNative === undefined ? given : spec.toNative(given);
    if (value === undefined || value === null) {
        return { kind: ValueKind.NONE, name };
    }
    if (typeof value === "boolean") {
        return { kind: ValueKind.BOOL, flag: value ? 1 : 0, name };
    }
    if (spec.members !== undefined) {
        const member = typeof value === "string" ? spec.members[value.toUpperCase()] : undefined;
        if (member === undefined) {
            const words = Object.keys(spec.members).map((word) => word.toLowerCase());
            throw new TypeError(`${owner} ${field} must be one of ${words.join(", ")}, got ${String(value)}`);
        }
        return { kind: ValueKind.ENUM, number: member, name };
    }
    if (typeof value === "number") {
        return { kind: ValueKind.NUMBER, number: value, name };
    }
    if (typeof value === "string") {
        return { kind: ValueKind.TEXT, text: value, name };
    }
    throw new TypeError(`${owner} ${field} must be a number, a string, a boolean or absent, got ${String(value)}`);
}

/** What an action answers, as a script reads it. */
function answerOf(answer: Option): unknown {
    switch (answer.kind) {
        case ValueKind.NONE:
            return undefined;
        case ValueKind.BOOL:
            return answer.flag !== 0;
        case ValueKind.NODE:
            return answer.node;
        case ValueKind.TEXT:
            return answer.text;
        default:
            return answer.number;
    }
}

/** A divider the library found, as a drag moves it: the ratio is the library's, written through. */
class NativeSplit implements LayoutSplit {
    readonly axis: "x" | "y";
    readonly span: number;
    readonly direction: 1 | -1;
    readonly min: number;
    readonly max: number;
    readonly cross: LayoutSplit | null;
    readonly #layout: NativeLayout;
    readonly #token: number;
    readonly #second: boolean;
    #ratio: number;

    constructor(layout: NativeLayout, split: Split) {
        this.#layout = layout;
        this.#token = split.token;
        this.axis = split.axis === 0 ? "x" : "y";
        this.span = split.span;
        this.direction = split.direction < 0 ? -1 : 1;
        this.min = split.min;
        this.max = split.max;
        this.#ratio = split.ratio;
        this.#second = (split.flags & 1) !== 0;
        this.cross = this.#second
            ? new NativeCross(layout, {
                  axis: split.crossAxis === 0 ? "x" : "y",
                  span: split.crossSpan,
                  direction: split.crossDirection < 0 ? -1 : 1,
                  min: split.crossMin,
                  max: split.crossMax,
                  ratio: split.crossRatio,
                  write: (ratio) => this.#write(Number.NaN, ratio),
                  reset: () => this.reset(),
              })
            : null;
    }

    get ratio(): number {
        return this.#ratio;
    }

    set ratio(value: number) {
        this.#ratio = value;
        this.#write(value, Number.NaN);
    }

    reset(): void {
        this.#layout.resetSplit(this.#token);
    }

    #write(ratio: number, cross: number): void {
        this.#layout.setSplit(this.#token, ratio, cross);
    }
}

/** The second divider of a drag: it moves with the first, through the same token. */
class NativeCross implements LayoutSplit {
    readonly axis: "x" | "y";
    readonly span: number;
    readonly direction: 1 | -1;
    readonly min: number;
    readonly max: number;
    readonly cross = null;
    readonly #write: (ratio: number) => void;
    readonly #reset: () => void;
    #ratio: number;

    constructor(
        _layout: NativeLayout,
        init: {
            axis: "x" | "y";
            span: number;
            direction: 1 | -1;
            min: number;
            max: number;
            ratio: number;
            write: (ratio: number) => void;
            reset: () => void;
        },
    ) {
        this.axis = init.axis;
        this.span = init.span;
        this.direction = init.direction;
        this.min = init.min;
        this.max = init.max;
        this.#ratio = init.ratio;
        this.#write = init.write;
        this.#reset = init.reset;
    }

    get ratio(): number {
        return this.#ratio;
    }

    set ratio(value: number) {
        this.#ratio = value;
        this.#write(value);
    }

    reset(): void {
        this.#reset();
    }
}

/**
 * A stock layout the library computes. A subclass names its kind and its options and keeps them as own fields; everything else is
 * the library's.
 */
export abstract class NativeLayout extends Layout {
    /** The library's layout, made at the first compute. */
    #builtin: Builtin | null = null;
    /** What the library was last told for each option, as text. */
    readonly #told = new Map<string, string>();
    /** The clients of the last query, by the number the library knows them by. */
    readonly #nodes = new Map<number, LayoutNode>();

    /** The `STD_BUILTIN_*` kind of the library's layout. */
    protected abstract get kind(): number;

    /** The options, by the name of the field that holds each. */
    protected abstract get options(): OptionSpecs;

    /** The client of `context.children` that the library is to treat as the focused one. */
    protected focusedChild(context: LayoutContext): LayoutNode | undefined {
        const { focused, children } = context;
        return focused !== null && focused !== undefined && children.includes(focused) ? focused : undefined;
    }

    /** The seat's pointer in the layout's coordinates, for a layout that reads it. */
    protected pointerOf(): Point | null {
        return null;
    }

    /** The own size of `node` with its frame, for an action that keeps a window's size; null when the node has none. */
    protected ownSizeOf(_node: LayoutNode): OwnSize | null {
        return null;
    }

    /** Called when the library changed something a workspace cannot see in the fields: a divider moved, an action changed the tree. */
    protected changed(): void {}

    /** The numbers of `nodes`, which the library knows them by from now on, and the focus among them. */
    #numbered(nodes: readonly LayoutNode[], focused: LayoutNode | undefined): { children: number[]; focused: number } {
        this.#nodes.clear();
        const children = nodes.map((node) => {
            const number = numberOf(node);
            this.#nodes.set(number, node);
            return number;
        });
        return { children, focused: focused === undefined ? 0 : numberOf(focused) };
    }

    /** What the library is asked about: the clients, the area, the gaps, the focus and what the layout reads of the pointer. */
    protected query(context: LayoutContext, focused: LayoutNode | undefined) {
        const numbers = this.#numbered(context.children, focused);
        const pointer = this.pointerOf();
        return {
            flags: pointer === null ? 0 : QueryFlags.POINTER,
            children: numbers.children,
            area: context.area,
            gapInner: context.gap?.inner ?? 0,
            gapOuter: context.gap?.outer ?? 0,
            focused: numbers.focused,
            pointerX: pointer?.x ?? 0,
            pointerY: pointer?.y ?? 0,
        };
    }

    /** `value` as the library takes it. */
    #nativeOf(spec: OptionSpec, value: unknown): unknown {
        return spec.toNative === undefined ? value : spec.toNative(value);
    }

    /** The library's layout with the options the fields hold now. */
    protected native(): Builtin {
        const owner = this.constructor.name;
        const specs = this.options;
        const fields = this as unknown as Record<string, unknown>;
        const text = (spec: OptionSpec, field: string): string => JSON.stringify(this.#nativeOf(spec, fields[field]) ?? null);
        if (this.#builtin === null) {
            const list = Object.entries(specs).map(([field, spec]) => optionOf(spec, owner, field, fields[field]));
            const made = Builtin.new(this.kind, list);
            if (made === null) {
                throw new Error(`${owner}: the library made no layout`);
            }
            this.#builtin = made;
            this.#told.clear();
            for (const [field, spec] of Object.entries(specs)) {
                this.#told.set(spec.native, text(spec, field));
            }
            return made;
        }
        for (const [field, spec] of Object.entries(specs)) {
            const now = text(spec, field);
            if (this.#told.get(spec.native) !== now) {
                this.#builtin.set(optionOf(spec, owner, field, fields[field]));
                this.#told.set(spec.native, now);
            }
        }
        return this.#builtin;
    }

    /** Reads the options back into the fields: the library moved one (a divider dragged, an action that sets a ratio). */
    protected pull(): void {
        const builtin = this.#builtin;
        if (builtin === null) {
            return;
        }
        const fields = this as unknown as Record<string, unknown>;
        for (const [field, spec] of Object.entries(this.options)) {
            if (spec.readOnly === true) {
                continue;
            }
            const answer = builtin.get(spec.native);
            const value = spec.members === undefined ? answerOf(answer) : wordOf(spec, answer.number);
            const plain = answer.kind === ValueKind.NONE ? undefined : value;
            const next = spec.fromNative === undefined ? plain : spec.fromNative(plain);
            if (fields[field] !== next) {
                fields[field] = next;
            }
            this.#told.set(spec.native, JSON.stringify(this.#nativeOf(spec, fields[field]) ?? null));
        }
    }

    /** Where `context.children` go, as the library computes it. */
    compute(context: LayoutContext): Placement[] {
        if (context.children.length === 0) {
            this.native();
            return [];
        }
        const builtin = this.native();
        const query = this.query(context, this.focusedChild(context));
        return builtin.compute(query).map((placement) => this.placement(placement));
    }

    /** A placement of the library as the script's `Placement`. */
    protected placement(raw: LibraryPlacement): Placement {
        const node = this.#nodes.get(raw.node) as LayoutNode;
        const placement: Placement = { node };
        if ((raw.flags & PlacementFlags.BOUNDS) !== 0) {
            placement.bounds = { ...raw.bounds };
        }
        if ((raw.flags & PlacementFlags.TILED) !== 0) {
            placement.tiled = true;
        }
        placement.visible = (raw.flags & PlacementFlags.HIDDEN) === 0;
        if (raw.z !== 0) {
            placement.z = raw.z;
        }
        const rotated = raw.rotationX !== 0 || raw.rotationY !== 0 || raw.rotationZ !== 0;
        if (rotated) {
            placement.rotation = [raw.rotationX, raw.rotationY, raw.rotationZ];
        }
        if (raw.scaleX !== 1 || raw.scaleY !== 1 || raw.scaleZ !== 1) {
            placement.scale = [raw.scaleX, raw.scaleY, raw.scaleZ];
        }
        if ((raw.flags & PlacementFlags.OFFSET) !== 0) {
            placement.transform = { position: { x: raw.offsetX, y: raw.offsetY, z: raw.z } };
        }
        return placement;
    }

    /**
     * Runs the library's action `name` with `values` against the nodes of `source` and the one that has the keyboard. A node the
     * library hands the keyboard to is given it. The options are read back, and an answer that is not `false` says the action
     * changed something (`changes` false for one that only reads or sets what no workspace compares).
     */
    protected act(name: string, values: readonly Init<Option>[], source: ActionSource, changes = true): unknown {
        const builtin = this.native();
        const nodes = [...source.clients];
        const focused = nodes.find((node) => (node as { focused?: unknown }).focused === true);
        const numbers = this.#numbered(nodes, focused);
        const size = focused === undefined ? null : this.ownSizeOf(focused);
        const [hand, answer] = builtin.act(name, [...values], {
            flags: size === null ? 0 : QueryFlags.FOCUSED_SIZE,
            children: numbers.children,
            area: { x: 0, y: 0, width: 0, height: 0 },
            gapInner: 0,
            gapOuter: 0,
            focused: numbers.focused,
            pointerX: 0,
            pointerY: 0,
            focusedWidth: size?.width ?? 0,
            focusedHeight: size?.height ?? 0,
        });
        this.#give(hand);
        this.pull();
        const value = answerOf(answer);
        if (changes && value !== false && value !== undefined) {
            this.changed();
        }
        return value;
    }

    /** Gives the keyboard to the node the library named, when it is one of the last query's and can take it. */
    #give(number: number | null): void {
        const node = number === null || number === 0 ? undefined : this.#nodes.get(number);
        (node as { focus?: () => void } | undefined)?.focus?.();
    }

    /** The divider a drag that pressed `client` at `point` would move: the library's, or null while it has none. */
    findSplit(client: LayoutNode, point: Point): LayoutSplit | null {
        if (this.#builtin === null || !this.#nodes.has(numberOf(client))) {
            return null;
        }
        const split = this.#builtin.findSplit(numberOf(client), point.x, point.y);
        return split === null ? null : new NativeSplit(this, split);
    }

    /** Moves the dividers of the drag `token`, and reads the options back. */
    setSplit(token: number, ratio: number, cross: number): void {
        this.#builtin?.setSplit(token, ratio, cross);
        this.pull();
        this.changed();
    }

    /** Puts the dividers of the drag `token` back, and reads the options back. */
    resetSplit(token: number): void {
        this.#builtin?.resetSplit(token);
        this.pull();
        this.changed();
    }

    /** The two clients traded places in the workspace's order. */
    onClientsSwapped(a: LayoutNode, b: LayoutNode): void {
        this.#builtin?.clientsSwapped(numberOf(a), numberOf(b));
    }

    /** A client left the workspace: a layout that keeps the keyboard in its own order (a strip's columns) hands it on. */
    onClientRemoved(client: LayoutNode, context: LayoutContext): void {
        if (this.#builtin === null) {
            return;
        }
        const query = this.query(context, this.focusedChild(context));
        this.#give(this.#builtin.clientRemoved(query, numberOf(client)));
    }
}

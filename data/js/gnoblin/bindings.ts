// Bindings: the trigger grammar, the binding classes and the collection a layout keeps them in.
//
// A trigger is the string a user writes for an input: `Super+Return`, `Super+MouseLeft+Drag`, `Swipe3Left`.
// `parseTrigger` is the one place its grammar lives. It turns the spelling into a canonical one, so two
// spellings of one trigger are one trigger (`Ctrl+Alt+Delete` and `control+alt+delete`), and it reports a bad
// spelling with the token that is wrong. The matcher, the hit test and the compositor claims are in
// `gnoblin/input`; a binding here is the trigger plus the hooks the matcher calls. It reaches no host.
//
// Grammar (case-insensitive except key names, which are keysym names):
//   trigger  := key | pointer | wheel | gesture
//   key      := (modifier "+")* keyname        `Super+Shift+1`, `XF86AudioMute`; a lone `Super` is a tap
//   pointer  := (modifier "+")* button ["+" ("Drag" | "Click")]    `Super+MouseLeft+Drag`, `MouseMiddle`
//   wheel    := (modifier "+")* "Wheel" ["Up" | "Down" | "Left" | "Right"]
//   gesture  := ("Swipe" | "Pinch") fingers(2-5) [direction]       `Swipe3Left`, `Pinch4In`
//   modifier := Shift | Ctrl (Control, Primary) | Alt (Mod1) | Meta | Super (Logo) | Hyper
// A key is a keysym name, never a character that depends on the layout: `Super+1` and `Super+Shift+1` are two
// triggers, and `Super+!` is refused (write `Super+Shift+1` or `Super+exclam`). A literal plus is `plus`.

/** The modifiers a trigger may name, in the order a canonical trigger writes them. */
export type TriggerModifier = "Shift" | "Ctrl" | "Alt" | "Meta" | "Super" | "Hyper";
export type TriggerButton = "MouseLeft" | "MouseMiddle" | "MouseRight";
export type TriggerDirection = "left" | "right" | "up" | "down" | "in" | "out";

/** Every spelling a modifier is accepted as, for the declaration file's `Trigger` type. */
export type TriggerModifierName = TriggerModifier | "Control" | "Primary" | "Mod1" | "Logo";
type ModifierPrefix =
    | ""
    | `${TriggerModifierName}+`
    | `${TriggerModifierName}+${TriggerModifierName}+`
    | `${TriggerModifierName}+${TriggerModifierName}+${TriggerModifierName}+`;
type FingerCount = 2 | 3 | 4 | 5;

/** `Swipe3`, `Swipe3Left`, `Pinch4`, `Pinch4In`: a touchpad gesture with its finger count and optional direction. */
export type GestureTrigger =
    | `Swipe${FingerCount}`
    | `Swipe${FingerCount}${"Left" | "Right" | "Up" | "Down"}`
    | `Pinch${FingerCount}`
    | `Pinch${FingerCount}${"In" | "Out"}`;
export type PointerTrigger =
    `${ModifierPrefix}${TriggerButton}` | `${ModifierPrefix}${TriggerButton}+${"Drag" | "Click"}`;
export type WheelTrigger = `${ModifierPrefix}Wheel${"" | "Up" | "Down" | "Left" | "Right"}`;
/** A key trigger is `modifiers+keysym`; the key names are the compositor's, so the type cannot list them. */
export type KeyTrigger = `${ModifierPrefix}${string}`;
export type Trigger = GestureTrigger | PointerTrigger | WheelTrigger | KeyTrigger;

interface TriggerBase {
    /** The canonical spelling: modifiers in `Shift, Ctrl, Alt, Meta, Super, Hyper` order, canonical key name. */
    readonly text: string;
    readonly modifiers: readonly TriggerModifier[];
}

/** A key, or a lone `Super` tap (`modifiers` empty, `key` `Super`). */
export interface KeyTriggerParts extends TriggerBase {
    readonly kind: "key";
    readonly key: string;
}

/** A mouse button; `action` is `drag` when the trigger ends in `Drag`, else `click`. */
export interface PointerTriggerParts extends TriggerBase {
    readonly kind: "pointer";
    readonly button: TriggerButton;
    readonly action: "click" | "drag";
}

export interface WheelTriggerParts extends TriggerBase {
    readonly kind: "wheel";
    readonly direction: "up" | "down" | "left" | "right" | null;
}

export interface GestureTriggerParts extends TriggerBase {
    readonly kind: "gesture";
    readonly gesture: "swipe" | "pinch";
    readonly fingers: number;
    readonly direction: TriggerDirection | null;
}

export type ParsedTrigger = KeyTriggerParts | PointerTriggerParts | WheelTriggerParts | GestureTriggerParts;

/** A trigger spelling the grammar refuses. `token` is the part that is wrong. */
export class TriggerError extends TypeError {
    readonly token: string;
    readonly trigger: string;

    constructor(what: string, trigger: string, token: string, reason: string) {
        super(`${what}: ${reason} (token "${token}" in "${trigger}")`);
        this.token = token;
        this.trigger = trigger;
    }
}

const MODIFIER_ORDER: readonly TriggerModifier[] = ["Shift", "Ctrl", "Alt", "Meta", "Super", "Hyper"];

const MODIFIERS: Readonly<Record<string, TriggerModifier>> = {
    shift: "Shift",
    ctrl: "Ctrl",
    control: "Ctrl",
    primary: "Ctrl",
    alt: "Alt",
    mod1: "Alt",
    meta: "Meta",
    super: "Super",
    logo: "Super",
    hyper: "Hyper",
};

/** X11 masks whose key depends on the keymap: the compositor refuses them, and so does the grammar. */
const REFUSED_MASKS: Readonly<Record<string, true>> = { mod2: true, mod3: true, mod4: true, mod5: true };

const BUTTONS: Readonly<Record<string, TriggerButton>> = {
    mouseleft: "MouseLeft",
    mousemiddle: "MouseMiddle",
    mouseright: "MouseRight",
};

const BUTTON_NUMBER: Readonly<Record<TriggerButton, number>> = { MouseLeft: 1, MouseMiddle: 2, MouseRight: 3 };

const WHEELS: Readonly<Record<string, WheelTriggerParts["direction"]>> = {
    wheel: null,
    wheelup: "up",
    wheeldown: "down",
    wheelleft: "left",
    wheelright: "right",
};

/** Names the grammar writes one way, so two spellings are not two bindings that shadow each other. */
const KEY_NAMES: Readonly<Record<string, string>> = {
    return: "Return",
    enter: "Return",
    escape: "Escape",
    esc: "Escape",
    tab: "Tab",
    space: "space",
    backspace: "BackSpace",
    delete: "Delete",
    del: "Delete",
    insert: "Insert",
    ins: "Insert",
    home: "Home",
    end: "End",
    pageup: "Page_Up",
    page_up: "Page_Up",
    pagedown: "Page_Down",
    page_down: "Page_Down",
    left: "Left",
    right: "Right",
    up: "Up",
    down: "Down",
    print: "Print",
    menu: "Menu",
    plus: "plus",
    minus: "minus",
};

const GESTURE = /^(swipe|pinch)([0-9]+)(left|right|up|down|in|out)?$/i;
const FUNCTION_KEY = /^f([1-9]|[12][0-9]|3[0-5])$/i;
const KEY_NAME = /^[A-Za-z0-9_]+$/;

/** A table entry that is the table's own, so a key named `constructor` is a key and not `Object`. */
function lookup<T>(table: Readonly<Record<string, T>>, key: string): T | undefined {
    return Object.hasOwn(table, key) ? table[key] : undefined;
}

function fail(what: string, trigger: string, token: string, reason: string): never {
    throw new TriggerError(what, trigger, token, reason);
}

function canonicalKey(token: string): string {
    const known = lookup(KEY_NAMES, token.toLowerCase());
    if (known !== undefined) {
        return known;
    }
    if (FUNCTION_KEY.test(token)) {
        return `F${token.slice(1)}`;
    }
    // A letter is the key, not a shifted letter: `Super+Q` and `Super+q` are one trigger, and the shift is
    // written as `Shift`. Longer names keep their case, because keysym names are case-sensitive.
    return token.length === 1 ? token.toLowerCase() : token;
}

function parseGesture(what: string, trigger: string, token: string): GestureTriggerParts {
    const match = GESTURE.exec(token);
    if (!match) {
        return fail(what, trigger, token, "is not a gesture; expected Swipe or Pinch with a finger count");
    }
    const gesture = match[1].toLowerCase() as "swipe" | "pinch";
    const fingers = Number(match[2]);
    if (fingers < 2 || fingers > 5) {
        return fail(what, trigger, token, `a gesture has 2 to 5 fingers, not ${fingers}`);
    }
    const direction = (match[3]?.toLowerCase() ?? null) as TriggerDirection | null;
    const allowed = gesture === "swipe" ? ["left", "right", "up", "down"] : ["in", "out"];
    if (direction !== null && !allowed.includes(direction)) {
        return fail(what, trigger, token, `a ${gesture} goes ${allowed.join(", ")}, not ${direction}`);
    }
    const name = gesture === "swipe" ? "Swipe" : "Pinch";
    const suffix = direction === null ? "" : direction[0].toUpperCase() + direction.slice(1);
    return { kind: "gesture", gesture, fingers, direction, modifiers: [], text: `${name}${fingers}${suffix}` };
}

/**
 * Parses a trigger into its parts and canonical spelling, or throws a `TriggerError` naming the token that
 * is wrong. `what` names the thing that holds the trigger, for the message.
 */
export function parseTrigger(trigger: unknown, what = "trigger"): ParsedTrigger {
    if (typeof trigger !== "string" || trigger === "") {
        throw new TypeError(`${what} must be a trigger such as "Super+Return", got ${String(trigger)}`);
    }
    const parts = trigger.split("+");
    for (let i = 0; i < parts.length; i++) {
        if (parts[i] === "") {
            const after = i === 0 ? "the start" : `"${parts[i - 1]}"`;
            return fail(what, trigger, "", `an empty part after ${after}; write a literal plus as "plus"`);
        }
        if (/\s/.test(parts[i])) {
            return fail(what, trigger, parts[i], "has whitespace; join the parts with plus alone");
        }
    }
    const gestureAt = parts.findIndex((part) => /^(swipe|pinch)/i.test(part));
    if (gestureAt >= 0) {
        if (parts.length > 1) {
            return fail(
                what,
                trigger,
                parts[gestureAt === 0 ? 1 : gestureAt],
                "a gesture takes no modifiers or other parts",
            );
        }
        return parseGesture(what, trigger, parts[0]);
    }

    const modifiers = new Set<TriggerModifier>();
    let button: TriggerButton | null = null;
    let wheel: { direction: WheelTriggerParts["direction"] } | null = null;
    let action: "click" | "drag" | null = null;
    let key: string | null = null;
    let last = "";
    for (const token of parts) {
        const lower = token.toLowerCase();
        const modifier = lookup(MODIFIERS, lower);
        if (lookup(REFUSED_MASKS, lower)) {
            fail(what, trigger, token, "names a keymap-dependent mask; write Super, Alt, Meta or Hyper");
        }
        if (modifier !== undefined) {
            if (button !== null || wheel !== null || key !== null || action !== null) {
                fail(what, trigger, token, "is a modifier after the key; modifiers come first");
            }
            if (modifiers.has(modifier)) {
                fail(what, trigger, token, `repeats the modifier ${modifier}`);
            }
            modifiers.add(modifier);
        } else if (lookup(BUTTONS, lower) !== undefined) {
            if (button !== null || wheel !== null || key !== null) {
                fail(what, trigger, token, "is a second input; a trigger has one key, button or wheel");
            }
            button = lookup(BUTTONS, lower) ?? null;
        } else if (Object.hasOwn(WHEELS, lower)) {
            if (button !== null || wheel !== null || key !== null) {
                fail(what, trigger, token, "is a second input; a trigger has one key, button or wheel");
            }
            wheel = { direction: WHEELS[lower] };
        } else if (lower === "drag" || lower === "click") {
            if (button === null) {
                fail(what, trigger, token, `needs a mouse button before it, such as Super+MouseLeft+${token}`);
            }
            if (action !== null) {
                fail(what, trigger, token, "is a second action");
            }
            action = lower;
        } else {
            if (button !== null || wheel !== null || action !== null) {
                fail(what, trigger, token, "is a key after a mouse button or wheel; a trigger has one input");
            }
            if (key !== null) {
                fail(what, trigger, token, `is a second key after "${key}"; a trigger has one key`);
            }
            if (token.length === 1 && !/[A-Za-z0-9]/.test(token)) {
                fail(
                    what,
                    trigger,
                    token,
                    "is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key",
                );
            }
            if (!KEY_NAME.test(token)) {
                fail(what, trigger, token, "is not a key name; a key name is letters, digits and underscores");
            }
            key = canonicalKey(token);
        }
        last = token;
    }

    const ordered = MODIFIER_ORDER.filter((modifier) => modifiers.has(modifier));
    const prefix = ordered.map((modifier) => `${modifier}+`).join("");
    if (button !== null) {
        // A bare button is a click, and so is `+Click`: one canonical spelling for the same trigger.
        const click = action !== "drag";
        const canonical = `${prefix}${button}${click ? "" : "+Drag"}`;
        return { kind: "pointer", button, action: click ? "click" : "drag", modifiers: ordered, text: canonical };
    }
    if (wheel !== null) {
        const name =
            wheel.direction === null ? "Wheel" : `Wheel${wheel.direction[0].toUpperCase()}${wheel.direction.slice(1)}`;
        return { kind: "wheel", direction: wheel.direction, modifiers: ordered, text: `${prefix}${name}` };
    }
    if (key === null) {
        // Only modifiers were written. A lone Super is a tap; anything else has no key to wait for.
        if (ordered.length === 1 && ordered[0] === "Super") {
            return { kind: "key", key: "Super", modifiers: [], text: "Super" };
        }
        return fail(
            what,
            trigger,
            last,
            ordered.length === 1
                ? "a lone modifier is only a trigger for Super (a tap)"
                : "ends with a modifier and has no key",
        );
    }
    return { kind: "key", key, modifiers: ordered, text: `${prefix}${key}` };
}

/** What a trigger asks the compositor to claim on the seat. */
export type TriggerClaim =
    | { readonly kind: "chord"; readonly text: string; readonly grab: boolean }
    | { readonly kind: "gesture"; readonly gesture: "swipe" | "pinch"; readonly fingers: number };

const FULL_MODIFIER: Readonly<Record<TriggerModifier, string>> = {
    Shift: "Shift",
    Ctrl: "Control",
    Alt: "Alt",
    Meta: "Meta",
    Super: "Super",
    Hyper: "Hyper",
};

/**
 * The seat claim a trigger needs, in the compositor's own chord spelling (`<Shift><Super>1`), or `null` when
 * the compositor has no claim for it (a wheel). A drag and a click of one button share a claim: the chord's
 * press and release both reach the shell, and the shell tells a drag from a click by how far the pointer went.
 */
export function claimOf(trigger: ParsedTrigger): TriggerClaim | null {
    const modifiers = trigger.modifiers.map((modifier) => `<${FULL_MODIFIER[modifier]}>`).join("");
    switch (trigger.kind) {
        case "key":
            return { kind: "chord", text: `${modifiers}${trigger.key}`, grab: false };
        case "pointer":
            return { kind: "chord", text: `${modifiers}Button${BUTTON_NUMBER[trigger.button]}`, grab: true };
        case "gesture":
            return { kind: "gesture", gesture: trigger.gesture, fingers: trigger.fingers };
        case "wheel":
            return null;
    }
}

/** What the host passes to a hook. The fields are the host's; the library only reads the ones a hook names. */
export interface BindingEvent {
    readonly timestamp: number;
    readonly [field: string]: unknown;
}

export interface Point {
    x: number;
    y: number;
}

export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** What a stock binding reads and writes of a client, so the library never names the host's `Client`. */
export interface BindingClient {
    readonly alive: boolean;
    readonly fullscreen: boolean;
    bounds: Rect;
    minSize?: { width: number; height: number };
    floatingBounds?: Rect | null;
}

/**
 * What the matcher passes to a drag or gesture hook. `position` is the pointer in the compositor's logical
 * space and `localPosition` is relative to the hit client's origin. A gesture event also has `phase`, `dx`,
 * `dy`, `scale`, `rotation` and `progress` (0 to 1, how far a directed gesture has gone).
 */
export interface DragEvent extends BindingEvent {
    readonly context: unknown;
    readonly device: unknown;
    readonly client?: BindingClient;
    readonly layout?: unknown;
    readonly position?: Point;
    readonly localPosition?: Point;
    readonly deltaTime: number;
    readonly modifiers: ReadonlySet<string>;
    readonly trigger: string;
    readonly phase?: string;
    readonly dx?: number;
    readonly dy?: number;
    readonly scale?: number;
    readonly rotation?: number;
    readonly progress?: number;
}

export interface BindingOptions {
    /** Unique among the bindings of one collection. */
    id: string;
    /** The chord, gesture or drag spelling, for example `Super+MouseLeft+Drag`. */
    trigger: string;
    enabled?: boolean;
    /** Higher wins when two bindings of one collection match one input. */
    priority?: number;
}

/** Two bindings of one collection match one input at one priority: the matcher could not choose. */
export class BindingConflictError extends Error {
    readonly trigger: string;
    readonly ids: readonly [string, string];

    constructor(trigger: string, existing: string, added: string, priority: number) {
        super(
            `binding "${added}" conflicts with "${existing}": both match ${trigger} at priority ${priority}; ` +
                "give one a different priority or trigger",
        );
        this.name = "BindingConflictError";
        this.trigger = trigger;
        this.ids = [existing, added];
    }
}

const owners = new WeakMap<Binding, BindingCollection>();

export class Binding {
    readonly id: string;
    enabled: boolean;
    priority: number;
    #trigger: ParsedTrigger;
    #written: string;

    constructor(options: BindingOptions) {
        if (typeof options?.id !== "string" || options.id === "") {
            throw new TypeError("Binding id must be a non-empty string");
        }
        const parsed = parseTrigger(options.trigger, `Binding "${options.id}" trigger`);
        if (options.priority !== undefined && !Number.isFinite(options.priority)) {
            throw new RangeError(`Binding "${options.id}" priority must be a finite number`);
        }
        this.id = options.id;
        this.#trigger = parsed;
        this.#written = options.trigger;
        this.enabled = options.enabled ?? true;
        this.priority = options.priority ?? 0;
    }

    /** The trigger as it was written. Assigning a new one moves the binding, which cancels a gesture it runs. */
    get trigger(): string {
        return this.#written;
    }

    set trigger(value: string) {
        const parsed = parseTrigger(value, `Binding "${this.id}" trigger`);
        const owner = owners.get(this);
        const previous = this.#trigger;
        owner?.assertFree(this, parsed, this.priority);
        this.#trigger = parsed;
        this.#written = value;
        owner?.retriggered(this, previous);
    }

    /** The grammar's reading of `trigger`. */
    get parsed(): ParsedTrigger {
        return this.#trigger;
    }

    /** Takes the binding out of the collection that holds it. A binding that is in none is left alone. */
    unbind(): void {
        owners.get(this)?.remove(this.id);
    }

    /** Returns the state object for this gesture, or false to decline it. */
    onStart(_event: DragEvent): object | false {
        return false;
    }

    onUpdate(_event: DragEvent, _state: object): void {}

    onEnd(_event: DragEvent, _state: object): void {}

    onCancel(_event: DragEvent, _state: object): void {}

    /**
     * A key press or a mouse click of the trigger. Return `false` to decline, which lets the next binding of
     * the input (a lower tier) have it. A binding that only drags or gestures leaves this alone.
     */
    onPress?(event: BindingEvent): void | false;
}

export interface KeyBindingOptions extends BindingOptions {
    /** What runs on a press. A subclass may override `onPress` instead. */
    action?: (event: BindingEvent) => void | false;
}

/** A key (or key chord) that runs a function on press. */
export class KeyBinding extends Binding {
    readonly #action: ((event: BindingEvent) => void | false) | undefined;

    constructor(options: KeyBindingOptions) {
        super(options);
        if (options.action !== undefined && typeof options.action !== "function") {
            throw new TypeError(`Binding "${options.id}" action must be a function`);
        }
        this.#action = options.action;
    }

    onPress(event: BindingEvent): void | false {
        return this.#action?.(event);
    }
}

/** A mouse button or wheel trigger that runs a function (a click). Subclassed for drags. */
export class PointerBinding extends KeyBinding {}

export interface PointerDragBindingOptions extends BindingOptions {
    /** How far the pointer must move from the press before `onStart`, in pixels. Default 4. */
    threshold?: number;
}

/** The base of the drag behaviours: the host calls `onStart`, `onUpdate` and `onEnd` or `onCancel`. */
export class PointerDragBinding extends PointerBinding {
    threshold: number;

    constructor(options: PointerDragBindingOptions) {
        super(options);
        const threshold = options.threshold ?? 4;
        if (!Number.isFinite(threshold) || threshold < 0) {
            throw new RangeError(`Binding "${options.id}" threshold must be a finite number of at least 0`);
        }
        this.threshold = threshold;
    }
}

/** A touchpad gesture the host follows from its begin to its end: same hooks and per-gesture state as a drag. */
export class GestureBinding extends Binding {}

/** A watcher of a collection. The input context registers claims through one. */
export interface BindingWatcher {
    added?(binding: Binding): void;
    removed?(binding: Binding): void;
    /** The trigger changed from `previous`; the binding already holds the new one. */
    retriggered?(binding: Binding, previous: ParsedTrigger): void;
}

/** The bindings of one layout, by id, in the order they were added. */
export class BindingCollection implements Iterable<Binding> {
    private readonly byId = new Map<string, Binding>();
    private readonly watchers: BindingWatcher[] = [];

    get size(): number {
        return this.byId.size;
    }

    /**
     * Adds a binding. A binding that is in another collection, an id that is taken, and a binding that matches
     * the same trigger at the same priority as one already here (the matcher could not choose) are refused.
     */
    add(binding: Binding): Binding {
        if (this.byId.has(binding.id)) {
            throw new Error(`a binding with id "${binding.id}" is already in this collection`);
        }
        const owner = owners.get(binding);
        if (owner !== undefined) {
            throw new Error(`binding "${binding.id}" is already in a collection; remove it there first`);
        }
        this.assertFree(binding, binding.parsed, binding.priority);
        this.byId.set(binding.id, binding);
        owners.set(binding, this);
        try {
            for (const watcher of [...this.watchers]) {
                watcher.added?.(binding);
            }
        } catch (error) {
            // A watcher that cannot take the binding (the compositor refused its claim) leaves the collection
            // as it was, so a refused binding is not half there.
            this.byId.delete(binding.id);
            owners.delete(binding);
            throw error;
        }
        return binding;
    }

    get(id: string): Binding | undefined {
        return this.byId.get(id);
    }

    has(id: string): boolean {
        return this.byId.has(id);
    }

    remove(id: string): boolean {
        const binding = this.byId.get(id);
        if (binding === undefined) {
            return false;
        }
        this.byId.delete(id);
        owners.delete(binding);
        for (const watcher of [...this.watchers]) {
            watcher.removed?.(binding);
        }
        return true;
    }

    clear(): void {
        for (const id of [...this.byId.keys()]) {
            this.remove(id);
        }
    }

    /** Calls `watcher` for later changes. Members that are already here are not replayed. Returns the unwatch. */
    watch(watcher: BindingWatcher): () => void {
        this.watchers.push(watcher);
        return () => {
            const at = this.watchers.indexOf(watcher);
            if (at >= 0) {
                this.watchers.splice(at, 1);
            }
        };
    }

    /** @internal Refuses `trigger` at `priority` for `binding` when another member already matches it. */
    assertFree(binding: Binding, trigger: ParsedTrigger, priority: number): void {
        for (const other of this.byId.values()) {
            if (other !== binding && other.parsed.text === trigger.text && other.priority === priority) {
                throw new BindingConflictError(trigger.text, other.id, binding.id, priority);
            }
        }
    }

    /** @internal Tells the watchers a member's trigger changed. */
    retriggered(binding: Binding, previous: ParsedTrigger): void {
        for (const watcher of [...this.watchers]) {
            watcher.retriggered?.(binding, previous);
        }
    }

    [Symbol.iterator](): Iterator<Binding> {
        return this.byId.values();
    }
}

/** Asserts `trigger` is a trigger the grammar accepts. `what` names the holder in the message. */
export function checkTrigger(trigger: unknown, what: string): asserts trigger is string {
    parseTrigger(trigger, what);
}

/**
 * The trigger of each named binding of a layout: `defaults` with `overrides` on top. A name that is
 * omitted or `undefined` keeps its default and `null` turns that binding off; `{ ...defaults, ...overrides }`
 * would let an explicit `undefined` erase the default (spec-review.md 1.19). A name that is not in
 * `defaults` is a typo and throws.
 */
export function resolveTriggers<K extends string>(
    defaults: Readonly<Record<K, string>>,
    overrides: { readonly [P in K]?: string | null | undefined } = {},
): Record<K, string | null> {
    const resolved = { ...defaults } as Record<K, string | null>;
    for (const name of Object.keys(overrides)) {
        if (!Object.hasOwn(defaults, name)) {
            throw new TypeError(`unknown trigger "${name}"; expected one of ${Object.keys(defaults).join(", ")}`);
        }
        const value = overrides[name as K];
        if (value === undefined) {
            continue;
        }
        if (value !== null) {
            checkTrigger(value, `trigger "${name}"`);
        }
        resolved[name as K] = value;
    }
    return resolved;
}

function moved(start: Point, now: Point): Point {
    return { x: now.x - start.x, y: now.y - start.y };
}

/** What `ClientMoveBinding.onStart` keeps for one drag. */
export interface MoveState {
    client: BindingClient;
    startPointer: Point;
    startBounds: Rect;
}

/**
 * Super+MouseLeft drag: the client follows the pointer by the distance it has moved since the press. Written
 * against the client's `bounds` and the event's `position` alone; there is no native grab behind it.
 */
export class ClientMoveBinding extends PointerDragBinding {
    constructor(options: Partial<PointerDragBindingOptions> = {}) {
        super({ ...options, id: options.id ?? "move", trigger: options.trigger ?? "Super+MouseLeft+Drag" });
    }

    onStart(event: DragEvent): MoveState | false {
        const { client, position } = event;
        if (!client || !position || client.fullscreen) {
            return false;
        }
        return { client, startPointer: { ...position }, startBounds: { ...client.bounds } };
    }

    onUpdate(event: DragEvent, state: object): void {
        const { client, startPointer, startBounds } = state as MoveState;
        if (!client.alive || !event.position) {
            return;
        }
        const delta = moved(startPointer, event.position);
        client.bounds = { ...startBounds, x: startBounds.x + delta.x, y: startBounds.y + delta.y };
    }

    onEnd(_event: DragEvent, state: object): void {
        const { client } = state as MoveState;
        if (client.alive) {
            client.floatingBounds = { ...client.bounds };
        }
    }

    onCancel(_event: DragEvent, state: object): void {
        const { client, startBounds } = state as MoveState;
        if (client.alive) {
            client.bounds = { ...startBounds };
        }
    }
}

/** Where a resize drag holds the client: the corner that moves with the pointer. */
export type ResizeCorner = "top-left" | "top-right" | "bottom-left" | "bottom-right";

export interface ClientResizeOptions extends Partial<PointerDragBindingOptions> {
    /** `quadrant` (default) picks the corner nearest the pointer at the press; or name the corner. */
    edges?: "quadrant" | ResizeCorner;
}

/** What `ClientResizeBinding.onStart` keeps for one drag. `corner` never changes during the drag. */
export interface ResizeState {
    client: BindingClient;
    start: Rect;
    pointer: Point;
    corner: ResizeCorner;
}

const CORNERS: readonly string[] = ["top-left", "top-right", "bottom-left", "bottom-right"];

/** The fewest pixels a client may be resized to when it states no minimum of its own. */
const MIN_SIZE = { width: 100, height: 80 };

/**
 * Super+MouseRight drag: the corner nearest the pointer at the press moves with it and the opposite corner
 * stays. The corner is chosen from the initial position only, so crossing the client's middle mid-drag does
 * not switch corners.
 */
export class ClientResizeBinding extends PointerDragBinding {
    edges: "quadrant" | ResizeCorner;

    constructor(options: ClientResizeOptions = {}) {
        super({ ...options, id: options.id ?? "resize", trigger: options.trigger ?? "Super+MouseRight+Drag" });
        const edges = options.edges ?? "quadrant";
        if (edges !== "quadrant" && !CORNERS.includes(edges)) {
            throw new TypeError(
                `ClientResizeBinding edges must be quadrant or one of ${CORNERS.join(", ")}, got ${String(edges)}`,
            );
        }
        this.edges = edges;
    }

    onStart(event: DragEvent): ResizeState | false {
        const { client, position } = event;
        if (!client || !position || client.fullscreen) {
            return false;
        }
        const start = { ...client.bounds };
        const where = event.localPosition ?? { x: position.x - start.x, y: position.y - start.y };
        const corner =
            this.edges === "quadrant"
                ? (`${where.y < start.height / 2 ? "top" : "bottom"}-${where.x < start.width / 2 ? "left" : "right"}` as ResizeCorner)
                : this.edges;
        return { client, start, pointer: { ...position }, corner };
    }

    onUpdate(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive || !event.position) {
            return;
        }
        const delta = moved(pointer, event.position);
        const r = { ...start };
        if (corner.endsWith("left")) {
            r.x += delta.x;
            r.width -= delta.x;
        } else {
            r.width += delta.x;
        }
        if (corner.startsWith("top")) {
            r.y += delta.y;
            r.height -= delta.y;
        } else {
            r.height += delta.y;
        }
        const min = client.minSize ?? MIN_SIZE;
        // The opposite corner is the anchor: a client held at its minimum stops there and does not slide.
        if (r.width < min.width) {
            if (corner.endsWith("left")) {
                r.x -= min.width - r.width;
            }
            r.width = min.width;
        }
        if (r.height < min.height) {
            if (corner.startsWith("top")) {
                r.y -= min.height - r.height;
            }
            r.height = min.height;
        }
        client.bounds = r;
    }

    onEnd(_event: DragEvent, state: object): void {
        const { client } = state as ResizeState;
        if (client.alive) {
            client.floatingBounds = { ...client.bounds };
        }
    }

    onCancel(_event: DragEvent, state: object): void {
        const { client, start } = state as ResizeState;
        if (client.alive) {
            client.bounds = { ...start };
        }
    }
}

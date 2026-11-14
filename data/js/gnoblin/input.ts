// Input: `gnoblin.input`, its contexts and devices, and the matcher that turns the compositor's claimed chords,
// pointer drags and gestures into calls of `Binding` hooks (owner-spec.md sections 10 and 11).
//
// The compositor does not know what a binding is. A binding's trigger becomes a *claim* on a seat: the
// compositor withholds that chord (or touchpad gesture) from clients and tells the shell each time it changes
// state. This module owns the claims (one per distinct chord, shared by every binding that wants it, released
// when the last of them goes), and decides what a chord event means:
//
//   key chord       press -> the highest-precedence binding that does not decline runs `onPress`.
//   pointer chord   the compositor streams the drag as `pointer_chord` messages (start, update, end, cancel)
//                   that carry the pointer's place. A start records the press place and the client under it;
//                   once an update is past the drag threshold, the candidates' `onStart` are asked in
//                   precedence order and the first to return a state object owns the drag (`onUpdate` per
//                   sample, `onEnd` on the end message, `onCancel` on the compositor's cancel, if the client
//                   dies, the binding is removed, `onUpdate` throws or the generation retires). An end before
//                   any movement is a click.
//   gesture         begin/update/end/cancel of a claimed swipe or pinch map to the same four hooks.
//
// Precedence: device bindings, then the bindings of the layout that owns the targeted client, then the
// context's own. A binding that declines (`onStart` returns false, `onPress` returns false) lets the next
// one have the input. Within a tier, `priority` decides, and two bindings of one collection that match the
// same trigger at the same priority are refused when the second is added (`BindingConflictError`).
//
// Every hook runs as its own scheduled call, under the shell's per-call budget and with its throw reported
// and isolated. A drag takes at most one sample per frame and at most one waits at a time: when scripts are
// behind, the next hook reads the latest position (latest wins). Starts, ends and cancels are never merged or
// dropped.
//
// The host is injected, so this module runs without one: `InputHost` is the seat's claims and properties,
// `InputWorld` is the clients and the scheduler.

import {
    BindingCollection,
    GestureBinding,
    KeyBinding,
    PointerDragBinding,
    claimOf,
    parseTrigger,
} from "gnoblin/bindings";
import type {
    Binding,
    BindingClient,
    BindingEvent,
    DragEvent,
    GestureTriggerParts,
    ParsedTrigger,
    Point,
    Rect,
    TriggerClaim,
} from "gnoblin/bindings";

/**
 * The window management settings the compositor publishes on a seat, as the host reads them (the schema's names in
 * camel case, the compositor's words for the values). Each is the schema default until the compositor says otherwise.
 */
export interface SeatSettings {
    readonly edgeTiling: boolean;
    /** `click`, `hover` or `hover-strict`. */
    readonly focusMode: string;
    /** `allow` or `prevent`. */
    readonly focusNewWindows: string;
    readonly raiseOnClick: boolean;
    readonly autoRaise: boolean;
    readonly autoRaiseDelay: number;
    readonly focusChangeOnPointerRest: boolean;
    /** The user asked for reduced motion: the shell's animations take no time. */
    readonly reducedMotion: boolean;
}

/** What a seat says and does, as plain functions over node ids. The shell's Rust host implements it. */
export interface InputHost {
    seats(): number[];
    seatName(seat: number): string;
    /** The pointer in the compositor's logical space. */
    pointer(seat: number): [number, number];
    /** The node that has keyboard focus, or 0. */
    focus(seat: number): number;
    /** The seat's devices, one `id:name:caps` item each. */
    devices(seat: number): string[];
    /** The seat's `drag_constraint`: `work_area` or `minimum_strip`. */
    dragConstraint(seat: number): string;
    /** The seat's window management settings, or undefined for a seat the tree no longer has. */
    settings(seat: number): SeatSettings | undefined;
    /** Queues the seat's keyboard focus going to no window; a refusal code, or undefined when queued. */
    clearFocus(seat: number): string | undefined;
    /** A claim id, or a refusal code. */
    claim(seat: number, chord: string, grab: boolean, device: string): number | string;
    claimGesture(seat: number, kind: string, fingers: number, device: string): number | string;
    /** Makes the devices that report `device` binding-only: a claim id, or a refusal code. */
    claimDevice(seat: number, device: string): number | string;
    unclaim(seat: number, id: number): void;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
}

/** What the module needs of the clients and of the shell's call queue. */
export interface InputWorld {
    clientOf(node: number): BindingClient | undefined;
    /** Every client, back to front. */
    stack(): BindingClient[];
    /** The rectangle a dragged window stays in (the work area of its output), or null when the tree has no screen yet. */
    workArea(client?: BindingClient): Rect | null;
    /** The layouts the workspaces use now. Their bindings go live in every context while they are in use. */
    layouts(): InputLayout[];
    /** Runs `run` later, under the call budget. A throw is reported against `site`. */
    schedule(site: string, run: () => void): void;
    report(site: string, error: unknown): void;
    error(code: string, target: string, message: string): Error;
}

/** What a seat the host has no record of reads as: the schema defaults. */
const SEAT_DEFAULTS: SeatSettings = Object.freeze({
    edgeTiling: false,
    focusMode: "click",
    focusNewWindows: "prevent",
    raiseOnClick: true,
    autoRaise: false,
    autoRaiseDelay: 500,
    focusChangeOnPointerRest: false,
    reducedMotion: false,
});

/** A claimed chord that changed state, as the compositor sends it. */
export interface InputChordMessage {
    readonly kind: "chord";
    readonly claim: number;
    readonly chord: string;
    readonly state: string;
}

/** A claimed pointer chord's drag that advanced: the compositor's `pointer_chord` signal. */
export interface InputPointerMessage {
    readonly kind: "pointer";
    readonly claim: number;
    /** `start`, `update`, `end` or `cancel`. */
    readonly phase: string;
    /** The pointer in global logical coordinates. */
    readonly x: number;
    readonly y: number;
    /** The pointer's movement since `start`. */
    readonly dx: number;
    readonly dy: number;
    /** The chord's button number: 1 left, 2 middle, 3 right, 8 back, 9 forward. */
    readonly button: number;
    readonly modifiers: string;
}

/** A claimed gesture that advanced. `dx`, `dy` are touchpad units since the begin. */
export interface InputGestureMessage {
    readonly kind: "gesture";
    readonly claim: number;
    readonly gesture: string;
    readonly fingers: number;
    readonly phase: string;
    readonly dx: number;
    readonly dy: number;
    readonly scale: number;
    readonly rotation: number;
}

export type InputMessage = InputChordMessage | InputPointerMessage | InputGestureMessage;

/** The layout side of a registration: all the input module reads of a layout is its bindings. */
export interface InputLayout {
    readonly bindings: BindingCollection;
}

/** The result of a hit test: the topmost client under a point. */
export interface InputHit {
    readonly client: BindingClient;
    readonly localPosition: Point;
}

/** How far the pointer must go from the press before a drag starts, unless the binding says otherwise. */
const DRAG_THRESHOLD = 4;
/** Touchpad units of centroid movement before a swipe has a direction. */
const SWIPE_DECIDE = 12;
/** How far a scale must move from 1 before a pinch has a direction. */
const PINCH_DECIDE = 0.1;
/** Touchpad units for a whole swipe (`progress` 1), and the scale distance for a whole pinch. */
const SWIPE_TRAVEL = 400;
const PINCH_TRAVEL = 0.5;
/** The progress at which a function bound to a gesture fires: Lua's 48 units and 0.12 scale. */
const SWIPE_COMMIT = 48 / SWIPE_TRAVEL;
const PINCH_COMMIT = 0.12 / PINCH_TRAVEL;
/** `deltaTime` never exceeds this, so a simulation that integrates it is not thrown by a stall. */
const MAX_DELTA_TIME = 0.1;

const TIER_CONTEXT = 0;
const TIER_LAYOUT = 1;
const TIER_DEVICE = 2;

let bindingSerial = 0;

interface ClaimEntry {
    readonly id: number;
    readonly specKey: string;
    readonly device: string;
    readonly spec: TriggerClaim;
    readonly pointer: boolean;
    refs: number;
}

interface Registration {
    readonly binding: Binding;
    readonly tier: number;
    readonly order: number;
    readonly owns: (client: BindingClient | null) => boolean;
    readonly layout: InputLayout | null;
    readonly device: string;
    readonly parsed: ParsedTrigger;
    readonly specKey: string;
    readonly claim: ClaimEntry;
}

/** Which of two registrations of one trigger runs first: the higher tier, then the higher priority, then the older. */
function byPrecedence(a: Registration, b: Registration): number {
    return b.tier - a.tier || b.binding.priority - a.binding.priority || a.order - b.order;
}

/** What `InputContext.describeBindings` says of one registered binding. */
export interface InputBindingInfo {
    readonly context: string;
    readonly id: string;
    readonly trigger: string;
    readonly kind: string;
    readonly owner: "context" | "layout" | "device";
    readonly layout: string | null;
    readonly device: string | null;
    readonly priority: number;
    readonly enabled: boolean;
    /** 1 is the binding a press of the trigger reaches first; a binding that declines hands the press to rank 2. */
    readonly rank: number;
}

/** One pointer chord from its start to its end or cancel. */
interface PointerPress {
    readonly claim: ClaimEntry;
    readonly origin: Point;
    readonly button: number;
    readonly client: BindingClient | null;
    readonly candidates: readonly Registration[];
    /** Where the compositor last said the pointer is. */
    at: Point;
    /** Where the last hook that ran saw it. */
    last: Point;
    lastTime: number;
    phase: "pressed" | "dragging" | "done";
    active: Registration | null;
    state: object | null;
    scheduled: boolean;
    /** The pointer went past the drag threshold and no binding took the drag: this press is not a click. */
    declined: boolean;
}

interface GestureSample {
    dx: number;
    dy: number;
    scale: number;
    rotation: number;
}

/** One touchpad gesture from its begin to its end or cancel. */
interface GestureRun {
    readonly claim: ClaimEntry;
    readonly fingers: number;
    readonly client: BindingClient | null;
    readonly candidates: readonly Registration[];
    latest: GestureSample;
    lastTime: number;
    started: Registration | null;
    state: object | null;
    /** No candidate can start for this gesture any more. */
    dead: boolean;
    done: boolean;
    scheduled: boolean;
}

function specKeyOf(spec: TriggerClaim): string {
    return spec.kind === "chord" ? `chord:${spec.text}` : `gesture:${spec.gesture}${spec.fingers}`;
}

/** The layout a client's workspace shows, for a layout attached without its own ownership test. */
function layoutOf(client: BindingClient): unknown {
    const workspace: unknown = Reflect.get(client, "workspace");
    return typeof workspace === "object" && workspace !== null ? Reflect.get(workspace, "layout") : undefined;
}

function distance(a: Point, b: Point): number {
    return Math.hypot(a.x - b.x, a.y - b.y);
}

function clamp01(value: number): number {
    return Math.max(0, Math.min(1, value));
}

/** The way a swipe has gone by its longer axis, or null while it has moved less than `SWIPE_DECIDE`. */
function swipeDirection(sample: GestureSample): "left" | "right" | "up" | "down" | null {
    if (Math.max(Math.abs(sample.dx), Math.abs(sample.dy)) < SWIPE_DECIDE) {
        return null;
    }
    if (Math.abs(sample.dx) >= Math.abs(sample.dy)) {
        return sample.dx > 0 ? "right" : "left";
    }
    return sample.dy > 0 ? "down" : "up";
}

function pinchDirection(sample: GestureSample): "in" | "out" | null {
    if (Math.abs(sample.scale - 1) < PINCH_DECIDE) {
        return null;
    }
    return sample.scale < 1 ? "in" : "out";
}

/** How far a gesture has gone for `trigger`, from 0 to 1. A directed gesture counts only its own way. */
function progressOf(trigger: GestureTriggerParts, sample: GestureSample): number {
    if (trigger.gesture === "pinch") {
        const spread =
            trigger.direction === "in"
                ? 1 - sample.scale
                : trigger.direction === "out"
                  ? sample.scale - 1
                  : Math.abs(sample.scale - 1);
        return clamp01(spread / PINCH_TRAVEL);
    }
    switch (trigger.direction) {
        case "left":
            return clamp01(-sample.dx / SWIPE_TRAVEL);
        case "right":
            return clamp01(sample.dx / SWIPE_TRAVEL);
        case "up":
            return clamp01(-sample.dy / SWIPE_TRAVEL);
        case "down":
            return clamp01(sample.dy / SWIPE_TRAVEL);
        default:
            return clamp01(Math.hypot(sample.dx, sample.dy) / SWIPE_TRAVEL);
    }
}

/** A function bound to a gesture: it runs once, when the gesture ends having gone far enough its way. */
class GestureActionBinding extends GestureBinding {
    readonly #action: (event: BindingEvent) => void | false;

    constructor(id: string, trigger: string, action: (event: BindingEvent) => void | false) {
        super({ id, trigger });
        this.#action = action;
    }

    onStart(): object {
        return {};
    }

    onEnd(event: DragEvent): void {
        const trigger = this.parsed as GestureTriggerParts;
        const commit = trigger.gesture === "pinch" ? PINCH_COMMIT : SWIPE_COMMIT;
        if ((event.progress ?? 0) >= commit) {
            this.#action(event);
        }
    }
}

/**
 * Binds `action` to `trigger` in `collection`. A function becomes a `KeyBinding` (a key or a click) or a
 * gesture action; `false` is a function that does nothing, so the trigger is claimed and swallowed. A drag needs the
 * hooks of a `Binding`, so a bare function for one is a mistake. A `Binding`
 * instance is added as it is, moved to `trigger` when it spells another.
 */
function bindIn(
    collection: BindingCollection,
    trigger: string,
    action: ((event: BindingEvent) => void | false) | Binding | false,
    what: string,
): Binding {
    // `false` claims the trigger and does nothing with it: the chord is withheld from clients and nothing runs, as the
    // Lua shell's `seat:bind({ chord = false })` did.
    if (action === false) {
        return bindIn(collection, trigger, () => undefined, what);
    }
    if (typeof action === "function") {
        const parsed = parseTrigger(trigger, `${what}.bind`);
        if (parsed.kind === "pointer" && parsed.action === "drag") {
            throw new TypeError(
                `${what}.bind: "${parsed.text}" is a drag; bind a PointerDragBinding with onStart, onUpdate and onEnd`,
            );
        }
        bindingSerial += 1;
        const id = `bind-${bindingSerial}:${parsed.text}`;
        const binding =
            parsed.kind === "gesture"
                ? new GestureActionBinding(id, trigger, action)
                : new KeyBinding({ id, trigger, action });
        return collection.add(binding);
    }
    if (typeof action !== "object" || action === null || typeof action.onStart !== "function") {
        throw new TypeError(`${what}.bind: the action must be a function or a Binding`);
    }
    if (action.trigger !== trigger) {
        action.trigger = trigger;
    }
    return collection.add(action);
}

/** A physical input device of a seat. Bindings here match only its own events. */
export class InputDevice {
    readonly id: string;
    readonly name: string;
    readonly capabilities: readonly string[];
    readonly context: InputContext;
    readonly bindings = new BindingCollection();

    constructor(context: InputContext, id: string, name: string, capabilities: readonly string[]) {
        this.context = context;
        this.id = id;
        this.name = name;
        this.capabilities = capabilities;
    }

    /**
     * As `InputContext.bind`, for events from this device. Its bindings run before a layout's or the context's.
     * `options.only: true` first makes the device binding-only (see `bindingOnly`); `only: false` is refused, because
     * it is not a way to say nothing.
     */
    bind(
        trigger: string,
        action: ((event: BindingEvent) => void | false) | Binding | false,
        options?: DeviceBindOptions,
    ): Binding {
        if (options !== undefined && options.only !== undefined && options.only !== true) {
            throw new TypeError(
                `InputDevice "${this.name}".bind: only must be true to make the device binding-only, got ${String(options.only)}`,
            );
        }
        if (options?.only === true) {
            this.bindingOnly = true;
        }
        return bindIn(this.bindings, trigger, action, `InputDevice "${this.name}"`);
    }

    /**
     * Whether this device is binding-only: its keys reach no client and fire only the bindings made for it or for every
     * device, so a macro pad never types into a window. Assigning `true` claims the device on the seat; `false` gives the
     * claim up. The claim is by the device's name, and lasts until the shell goes or it is given up.
     */
    get bindingOnly(): boolean {
        return this.context.isBindingOnly(this.name);
    }

    set bindingOnly(on: boolean) {
        if (typeof on !== "boolean") {
            throw new TypeError(`InputDevice "${this.name}".bindingOnly must be true or false, got ${String(on)}`);
        }
        if (on) {
            this.context.makeBindingOnly(this.name);
        } else {
            this.context.releaseBindingOnly(this.name);
        }
    }
}

/** What `InputDevice.bind` takes beside the trigger and the action. */
export interface DeviceBindOptions {
    /** `true` makes the device binding-only before the binding is made. The only value that means anything. */
    only?: true;
}

/** A device name is one non-empty line: the compositor matches it exactly against what a device reports. */
function checkDeviceName(name: unknown, what: string): void {
    if (typeof name !== "string" || name === "" || name.includes("\n")) {
        throw new TypeError(`${what}: a device name is a non-empty line of text from devices, got ${String(name)}`);
    }
}

/** `id:name:caps`: the id up to the first colon and the capabilities after the last, so a name may hold colons. */
function parseDevice(context: InputContext, item: string): InputDevice | null {
    const first = item.indexOf(":");
    const last = item.lastIndexOf(":");
    if (first <= 0 || last <= first) {
        return null;
    }
    const caps = item.slice(last + 1);
    return new InputDevice(
        context,
        item.slice(0, first),
        item.slice(first + 1, last),
        caps === "" ? [] : caps.split(","),
    );
}

export class InputContext {
    readonly name: string;
    readonly bindings = new BindingCollection();

    readonly #host: InputHost;
    readonly #world: InputWorld;
    readonly #seat: number;
    readonly #claims = new Map<string, ClaimEntry>();
    readonly #claimIds = new Map<number, ClaimEntry>();
    readonly #registrations = new Set<Registration>();
    readonly #layouts = new Map<InputLayout, () => void>();
    readonly #devices = new Map<string, { device: InputDevice; unfollow: () => void }>();
    /** The claim of each binding-only device, by its name. */
    readonly #only = new Map<string, number>();
    readonly #presses = new Map<number, PointerPress>();
    readonly #gestures = new Map<number, GestureRun>();
    #order = 0;
    #disposed = false;
    /** Whether giving up a claim tells the seat: false once the seat itself is gone. */
    #release = true;
    #unfollowOwn: () => void;
    /** The layouts `syncLayouts` attached, which it also detaches; one attached by hand stays until detached by hand. */
    readonly #auto = new Set<InputLayout>();
    /** Layouts whose bindings were refused once, so they are not tried and reported every frame. */
    readonly #failed = new WeakSet<InputLayout>();

    /** @internal Made by `InputManager`: one context per seat the compositor reports. */
    constructor(host: InputHost, world: InputWorld, seat: number) {
        this.#host = host;
        this.#world = world;
        this.#seat = seat;
        this.name = host.seatName(seat);
        this.#unfollowOwn = this.#follow(this.bindings, TIER_CONTEXT, () => true, null, "");
    }

    /** The devices the seat reports now. */
    get devices(): readonly InputDevice[] {
        this.syncDevices();
        return [...this.#devices.values()].map((entry) => entry.device);
    }

    /** The client with keyboard focus, or null. Read through to the seat on every call. */
    get focusedClient(): BindingClient | null {
        return this.#world.clientOf(this.#host.focus(this.#seat)) ?? null;
    }

    /** The window management settings the seat publishes, with the shell's overrides from the config on top. */
    get settings(): SeatSettings {
        return this.#host.settings(this.#seat) ?? SEAT_DEFAULTS;
    }

    /** Takes the keyboard focus from every window. */
    clearFocus(): void {
        const code = this.#host.clearFocus(this.#seat);
        if (code != null) {
            throw this.#world.error("refused", `InputContext "${this.name}".clearFocus`, `the host refused (${code})`);
        }
    }

    /** Not wired: displays have no JavaScript object yet. */
    get activeDisplay(): null {
        return null;
    }

    /** The pointer in the compositor's logical space. */
    get pointer(): Point {
        const [x, y] = this.#host.pointer(this.#seat);
        return { x, y };
    }

    /**
     * Binds `action` to `trigger` for this context and returns the binding; `unbind()` on it, or
     * `bindings.remove(id)`, takes it away and releases the compositor claim when no other binding needs it.
     * Throws a `TriggerError` for a bad trigger and a `BindingConflictError` when another binding of this
     * context matches it at the same priority.
     */
    bind(trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false): Binding {
        return bindIn(this.bindings, trigger, action, `InputContext "${this.name}"`);
    }

    /** Whether the devices that report `name` are binding-only on this seat. */
    isBindingOnly(name: string): boolean {
        return this.#only.has(name);
    }

    /**
     * Makes the devices that report `name` (a name from `devices`) binding-only: a key of theirs reaches no client and
     * fires only the bindings made for the device or for every device, and a chord no binding claims is dropped. Once
     * is enough: a second call changes nothing. Works before the device is plugged in.
     */
    makeBindingOnly(name: string): void {
        checkDeviceName(name, `InputContext "${this.name}".makeBindingOnly`);
        if (this.#only.has(name)) {
            return;
        }
        const result = this.#host.claimDevice(this.#seat, name);
        if (typeof result === "string") {
            throw this.#world.error(
                "refused",
                `InputContext "${this.name}".makeBindingOnly`,
                `the compositor refused to make "${name}" binding-only (${result})`,
            );
        }
        this.#only.set(name, result);
    }

    /** Gives the binding-only claim on `name` up, so its keys reach clients again. False when there was none. */
    releaseBindingOnly(name: string): boolean {
        const id = this.#only.get(name);
        if (id === undefined) {
            return false;
        }
        this.#only.delete(name);
        if (this.#release) {
            this.#host.unclaim(this.#seat, id);
        }
        return true;
    }

    /**
     * Every binding registered on this context, whoever bound it (the context, a layout attached to it, one of its
     * devices) and where it ranks among the bindings of the same trigger. Plain data, for the inspector.
     */
    describeBindings(): InputBindingInfo[] {
        const groups = new Map<string, Registration[]>();
        for (const registration of this.#registrations) {
            const group = groups.get(registration.specKey) ?? [];
            group.push(registration);
            groups.set(registration.specKey, group);
        }
        const owners = ["context", "layout", "device"] as const;
        const described: InputBindingInfo[] = [];
        for (const group of groups.values()) {
            group.sort(byPrecedence);
            for (const [index, registration] of group.entries()) {
                described.push({
                    context: this.name,
                    id: registration.binding.id,
                    trigger: registration.binding.trigger,
                    kind: registration.parsed.kind,
                    owner: owners[registration.tier] ?? "context",
                    layout: registration.layout === null ? null : registration.layout.constructor.name,
                    device: registration.device === "" ? null : registration.device,
                    priority: registration.binding.priority,
                    enabled: registration.binding.enabled,
                    rank: index + 1,
                });
            }
        }
        return described;
    }

    /** The topmost client whose bounds hold `position`, or null. */
    hitTest(position: Point): InputHit | null {
        const stack = this.#world.stack();
        for (let i = stack.length - 1; i >= 0; i--) {
            const client = stack[i];
            const b = client.bounds;
            if (
                client.alive &&
                client.minimised !== true &&
                position.x >= b.x &&
                position.x < b.x + b.width &&
                position.y >= b.y &&
                position.y < b.y + b.height
            ) {
                return { client, localPosition: { x: position.x - b.x, y: position.y - b.y } };
            }
        }
        return null;
    }

    /**
     * Makes the bindings of `layout` live for this context, for the clients `owns` accepts. Without `owns`
     * that is a client whose workspace's `layout` is this one. Bindings the layout gains or loses later follow.
     */
    attachLayout(layout: InputLayout, owns?: (client: BindingClient) => boolean): void {
        if (this.#layouts.has(layout)) {
            throw new Error("this layout is already attached to the input context");
        }
        const scope = owns ?? ((client: BindingClient) => layoutOf(client) === layout);
        this.#layouts.set(
            layout,
            this.#follow(layout.bindings, TIER_LAYOUT, (client) => client !== null && scope(client), layout, ""),
        );
    }

    /** Stops the layout's bindings. A drag one of them runs is cancelled. */
    detachLayout(layout: InputLayout): void {
        this.#layouts.get(layout)?.();
        this.#layouts.delete(layout);
    }

    /** @internal Attaches the layouts the workspaces use and detaches the ones they no longer use. */
    syncLayouts(wanted: readonly InputLayout[]): void {
        const inUse = new Set(wanted);
        for (const layout of [...this.#auto]) {
            if (!inUse.has(layout)) {
                this.#auto.delete(layout);
                this.detachLayout(layout);
            }
        }
        for (const layout of inUse) {
            if (this.#layouts.has(layout) || this.#failed.has(layout)) {
                continue;
            }
            try {
                this.attachLayout(layout);
                this.#auto.add(layout);
            } catch (error) {
                this.#failed.add(layout);
                this.#world.report(`InputContext "${this.name}".attachLayout`, error);
            }
        }
    }

    /** @internal Whether `claim` is one of this context's. */
    ownsClaim(claim: number): boolean {
        return this.#claimIds.has(claim);
    }

    /** @internal Brings the device list up to date with the seat's. */
    syncDevices(): void {
        const items = this.#host.devices(this.#seat);
        const seen = new Set<string>();
        for (const item of items) {
            const device = parseDevice(this, item);
            if (device === null) {
                continue;
            }
            seen.add(device.id);
            if (!this.#devices.has(device.id)) {
                const unfollow = this.#follow(device.bindings, TIER_DEVICE, () => true, null, device.name);
                this.#devices.set(device.id, { device, unfollow });
            }
        }
        for (const [id, entry] of [...this.#devices]) {
            if (!seen.has(id)) {
                entry.unfollow();
                this.#devices.delete(id);
            }
        }
    }

    /** @internal Takes one message from the compositor and schedules what it means. */
    receive(message: InputMessage): void {
        const claim = this.#claimIds.get(message.claim);
        if (claim === undefined) {
            return;
        }
        if (message.kind === "chord") {
            // A pointer chord's press and release are the `pointer` messages' start and end: the compositor
            // sends no `chord` for them.
            if (claim.spec.kind === "chord" && !claim.pointer && message.state === "press") {
                this.#keyPress(claim);
            }
            return;
        }
        if (message.kind === "pointer") {
            if (claim.pointer) {
                this.#pointerMessage(claim, message);
            }
            return;
        }
        if (claim.spec.kind === "gesture") {
            this.#gestureMessage(claim, message);
        }
    }

    /** @internal A client died: gestures on it are cancelled once, and a press on it is dropped. */
    clientGone(client: BindingClient): void {
        for (const press of [...this.#presses.values()]) {
            if (press.client === client) {
                this.#cancelPress(press);
            }
        }
        for (const run of [...this.#gestures.values()]) {
            if (run.client === client) {
                this.#cancelGesture(run);
            }
        }
    }

    /**
     * @internal Cancels everything in flight and gives up every claim: the context is going away. `release`
     * is false when the seat is already gone, so there is nothing to unclaim on.
     */
    dispose(release: boolean): void {
        if (this.#disposed) {
            return;
        }
        this.#disposed = true;
        this.#release = release;
        for (const press of [...this.#presses.values()]) {
            this.#cancelPress(press);
        }
        for (const run of [...this.#gestures.values()]) {
            this.#cancelGesture(run);
        }
        this.#unfollowOwn();
        for (const entry of this.#devices.values()) {
            entry.unfollow();
        }
        this.#devices.clear();
        for (const unfollow of [...this.#layouts.values()]) {
            unfollow();
        }
        this.#layouts.clear();
        this.#auto.clear();
        // Every registration's claim went with it; this is the net for a claim nothing registered against.
        for (const claim of [...this.#claims.values()]) {
            this.#claims.delete(`${claim.specKey}\n${claim.device}`);
            this.#claimIds.delete(claim.id);
            if (release) {
                this.#host.unclaim(this.#seat, claim.id);
            }
        }
        for (const id of this.#only.values()) {
            if (release) {
                this.#host.unclaim(this.#seat, id);
            }
        }
        this.#only.clear();
        this.#registrations.clear();
    }

    // -- registrations and claims ------------------------------------------------------------------------

    #follow(
        source: BindingCollection,
        tier: number,
        owns: (client: BindingClient | null) => boolean,
        layout: InputLayout | null,
        device: string,
    ): () => void {
        const held = new Map<Binding, Registration>();
        const add = (binding: Binding): void => {
            held.set(binding, this.#register(binding, tier, owns, layout, device));
        };
        const remove = (binding: Binding): void => {
            const registration = held.get(binding);
            if (registration !== undefined) {
                held.delete(binding);
                this.#unregister(registration);
            }
        };
        try {
            for (const binding of source) {
                add(binding);
            }
        } catch (error) {
            // A binding the seat refuses leaves nothing of this source behind, so the caller sees all or none.
            for (const binding of [...held.keys()]) {
                remove(binding);
            }
            throw error;
        }
        const unwatch = source.watch({
            added: add,
            removed: remove,
            retriggered: (binding) => {
                remove(binding);
                add(binding);
            },
        });
        return () => {
            unwatch();
            for (const binding of [...held.keys()]) {
                remove(binding);
            }
        };
    }

    #register(
        binding: Binding,
        tier: number,
        owns: (client: BindingClient | null) => boolean,
        layout: InputLayout | null,
        device: string,
    ): Registration {
        const parsed = binding.parsed;
        const spec = claimOf(parsed);
        if (spec === null) {
            throw this.#world.error(
                "unsupported",
                `Binding "${binding.id}"`,
                parsed.kind === "controller"
                    ? `${parsed.text} is an XR controller trigger; bind it in the XR room's layout, not on a seat`
                    : `the compositor has no claim for ${parsed.text}; wheel triggers cannot be bound yet`,
            );
        }
        const specKey = specKeyOf(spec);
        const claim = this.#acquire(spec, specKey, device, parsed.kind === "pointer");
        const registration: Registration = {
            binding,
            tier,
            order: this.#order++,
            owns,
            layout,
            device,
            parsed,
            specKey,
            claim,
        };
        this.#registrations.add(registration);
        return registration;
    }

    #unregister(registration: Registration): void {
        this.#registrations.delete(registration);
        // A drag this binding runs ends here: the binding is gone, so nothing could finish it.
        for (const press of [...this.#presses.values()]) {
            if (press.active === registration) {
                this.#cancelPress(press);
            }
        }
        for (const run of [...this.#gestures.values()]) {
            if (run.started === registration) {
                this.#cancelGesture(run);
            }
        }
        const { claim } = registration;
        claim.refs -= 1;
        if (claim.refs === 0) {
            this.#claims.delete(`${claim.specKey}\n${claim.device}`);
            this.#claimIds.delete(claim.id);
            if (this.#release) {
                this.#host.unclaim(this.#seat, claim.id);
            }
        }
    }

    #acquire(spec: TriggerClaim, specKey: string, device: string, pointer: boolean): ClaimEntry {
        const key = `${specKey}\n${device}`;
        const held = this.#claims.get(key);
        if (held !== undefined) {
            held.refs += 1;
            return held;
        }
        const result =
            spec.kind === "chord"
                ? this.#host.claim(this.#seat, spec.text, spec.grab, device)
                : this.#host.claimGesture(this.#seat, spec.gesture, spec.fingers, device);
        if (typeof result === "string") {
            throw this.#world.error(
                "refused",
                `InputContext "${this.name}"`,
                `the compositor refused the claim for ${specKey} (${result})`,
            );
        }
        const claim: ClaimEntry = { id: result, specKey, device, spec, pointer, refs: 1 };
        this.#claims.set(key, claim);
        this.#claimIds.set(claim.id, claim);
        return claim;
    }

    /** The registrations that can take an input on `claim`, best first. */
    #candidates(claim: ClaimEntry, client: BindingClient | null): Registration[] {
        const list: Registration[] = [];
        for (const registration of this.#registrations) {
            // A claim for one device shadows the claim for every device, so what the context bound for every
            // device still has to answer for that device's events: it is a later tier of the same list.
            const applies = registration.device === claim.device || registration.device === "";
            if (
                registration.specKey === claim.specKey &&
                applies &&
                registration.binding.enabled &&
                registration.owns(client)
            ) {
                list.push(registration);
            }
        }
        return list.sort(byPrecedence);
    }

    // -- events -------------------------------------------------------------------------------------------

    #event(
        registration: Registration,
        client: BindingClient | null,
        position: Point,
        lastTime: number,
        extra: Record<string, unknown> = {},
    ): { event: DragEvent; now: number } {
        const now = this.#host.now();
        const deviceName = registration.device === "" ? null : registration.device;
        const device =
            deviceName === null
                ? null
                : ([...this.#devices.values()].find((entry) => entry.device.name === deviceName)?.device ?? null);
        const base: Record<string, unknown> = {
            context: this,
            device,
            trigger: registration.parsed.text,
            timestamp: now,
            deltaTime: Math.min(MAX_DELTA_TIME, Math.max(0, (now - lastTime) / 1000)),
            modifiers: new Set<string>(registration.parsed.modifiers),
            position,
            ...extra,
        };
        if (client !== null) {
            const b = client.bounds;
            base.client = client;
            base.localPosition = { x: position.x - b.x, y: position.y - b.y };
        }
        if (registration.layout !== null) {
            base.layout = registration.layout;
        }
        return { event: base as unknown as DragEvent, now };
    }

    /** Calls a hook, reporting a throw instead of raising it, so the next binding still has its turn. */
    #call<R>(registration: Registration, hook: string, run: () => R): { ok: true; value: R } | { ok: false } {
        try {
            return { ok: true, value: run() };
        } catch (error) {
            this.#world.report(`${registration.binding.constructor.name} "${registration.binding.id}".${hook}`, error);
            return { ok: false };
        }
    }

    #start(registration: Registration, event: DragEvent): object | null {
        const outcome = this.#call(registration, "onStart", () => registration.binding.onStart(event));
        if (!outcome.ok || outcome.value === false) {
            return null;
        }
        if (typeof outcome.value !== "object" || outcome.value === null) {
            this.#world.report(
                `${registration.binding.constructor.name} "${registration.binding.id}".onStart`,
                new TypeError("onStart must return a state object, or false to decline"),
            );
            return null;
        }
        return outcome.value;
    }

    // -- key chords ---------------------------------------------------------------------------------------

    #keyPress(claim: ClaimEntry): void {
        const client = this.focusedClient;
        const candidates = this.#candidates(claim, client).filter((r) => typeof r.binding.onPress === "function");
        if (candidates.length === 0) {
            return;
        }
        this.#world.schedule(`key ${claim.specKey}`, () => {
            const at = this.pointer;
            for (const registration of candidates) {
                if (!this.#registrations.has(registration)) {
                    continue;
                }
                const { event } = this.#event(registration, client, at, this.#host.now());
                const outcome = this.#call(registration, "onPress", () => registration.binding.onPress?.(event));
                // A throw is that binding's answer: it handled the press, badly. Only a `false` declines.
                if (!outcome.ok || outcome.value !== false) {
                    return;
                }
            }
        });
    }

    // -- pointer drags ------------------------------------------------------------------------------------

    #pointerMessage(claim: ClaimEntry, message: InputPointerMessage): void {
        const at = { x: message.x, y: message.y };
        if (message.phase === "start") {
            this.#pointerPress(claim, at, message.button);
            return;
        }
        const press = this.#presses.get(claim.id);
        if (press === undefined) {
            return;
        }
        press.at = at;
        if (message.phase === "update") {
            this.#schedulePointer(press);
        } else if (message.phase === "end") {
            this.#pointerRelease(press);
        } else if (message.phase === "cancel") {
            this.#cancelPress(press);
        }
    }

    #pointerPress(claim: ClaimEntry, origin: Point, button: number): void {
        const previous = this.#presses.get(claim.id);
        if (previous !== undefined) {
            // A second start before the end: the first one's end was lost. Finish it as cancelled.
            this.#cancelPress(previous);
        }
        const client = this.hitTest(origin)?.client ?? null;
        const press: PointerPress = {
            claim,
            origin,
            button,
            client,
            candidates: this.#candidates(claim, client),
            at: origin,
            last: origin,
            lastTime: this.#host.now(),
            phase: "pressed",
            active: null,
            state: null,
            scheduled: false,
            declined: false,
        };
        this.#presses.set(claim.id, press);
    }

    /** Schedules a sample of the drag unless the pointer has not moved or one already waits (latest wins). */
    #schedulePointer(press: PointerPress): void {
        if (press.phase === "done" || press.scheduled || (press.at.x === press.last.x && press.at.y === press.last.y)) {
            return;
        }
        press.scheduled = true;
        this.#world.schedule(`pointer ${press.claim.specKey}`, () => {
            press.scheduled = false;
            this.#advance(press);
        });
    }

    /** The event of a pointer drag hook: the position, how far it is from the press, the button and the work area. */
    #pointerEvent(registration: Registration, press: PointerPress, position: Point): { event: DragEvent; now: number } {
        return this.#event(registration, press.client, position, press.lastTime, {
            x: position.x,
            y: position.y,
            dx: position.x - press.origin.x,
            dy: position.y - press.origin.y,
            button: press.button,
            workArea: this.#world.workArea(press.client ?? undefined) ?? undefined,
            dragConstraint: this.#host.dragConstraint(this.#seat),
            edgeTiling: this.settings.edgeTiling,
        });
    }

    #pointerRelease(press: PointerPress): void {
        this.#presses.delete(press.claim.id);
        this.#world.schedule(`pointer end ${press.claim.specKey}`, () => {
            if (press.phase === "done") {
                return;
            }
            // The end may arrive in the frame of the update that starts the drag: start it now.
            this.#advance(press);
            if (press.phase === "dragging" && press.active !== null) {
                const registration = press.active;
                const state = press.state as object;
                press.phase = "done";
                const { event } = this.#pointerEvent(registration, press, press.at);
                this.#call(registration, "onEnd", () => registration.binding.onEnd(event, state));
                return;
            }
            const wasPressed = press.phase === "pressed" && !press.declined;
            press.phase = "done";
            if (wasPressed) {
                this.#click(press);
            }
        });
    }

    /** An end with no drag: the best click binding that does not decline has it. */
    #click(press: PointerPress): void {
        for (const registration of press.candidates) {
            const parsed = registration.parsed;
            if (parsed.kind !== "pointer" || parsed.action !== "click" || !this.#registrations.has(registration)) {
                continue;
            }
            if (typeof registration.binding.onPress !== "function") {
                continue;
            }
            const { event } = this.#pointerEvent(registration, press, press.at);
            const outcome = this.#call(registration, "onPress", () => registration.binding.onPress?.(event));
            if (!outcome.ok || outcome.value !== false) {
                return;
            }
        }
    }

    /** Takes the drag as far as the pointer's position allows: starts it past the threshold, else updates it. */
    #advance(press: PointerPress): void {
        if (press.phase === "done") {
            return;
        }
        const at = press.at;
        if (press.phase === "pressed" && !press.declined) {
            this.#tryStart(press, at);
        }
        if (press.phase === "dragging" && press.active !== null && (at.x !== press.last.x || at.y !== press.last.y)) {
            const registration = press.active;
            const state = press.state as object;
            const { event, now } = this.#pointerEvent(registration, press, at);
            press.last = at;
            press.lastTime = now;
            const outcome = this.#call(registration, "onUpdate", () => registration.binding.onUpdate(event, state));
            if (!outcome.ok) {
                // A binding that throws has lost the drag: it is told once, and no other drag is touched.
                this.#cancelPress(press);
            }
        }
    }

    #tryStart(press: PointerPress, at: Point): void {
        const moved = distance(press.origin, at);
        for (const registration of press.candidates) {
            const parsed = registration.parsed;
            if (parsed.kind !== "pointer" || parsed.action !== "drag" || !this.#registrations.has(registration)) {
                continue;
            }
            const threshold =
                registration.binding instanceof PointerDragBinding ? registration.binding.threshold : DRAG_THRESHOLD;
            if (moved < threshold) {
                // A better binding is still waiting for the pointer to go far enough: the others wait too.
                return;
            }
            // The start event says where the press was, not where the pointer has got to, so a binding that
            // measures from `position` measures from the press. The update that follows has the current one.
            const { event } = this.#pointerEvent(registration, press, press.origin);
            const state = this.#start(registration, event);
            if (state !== null) {
                press.phase = "dragging";
                press.active = registration;
                press.state = state;
                press.last = press.origin;
                return;
            }
        }
        // Every candidate that was due has declined, so this is not a drag. The press stays one: its end is
        // still swallowed (the compositor withheld the press from the client), but a pointer that has gone this
        // far is not a click either.
        if (moved >= DRAG_THRESHOLD) {
            press.declined = true;
        }
    }

    #cancelPress(press: PointerPress): void {
        if (this.#presses.get(press.claim.id) === press) {
            this.#presses.delete(press.claim.id);
        }
        const was = press.phase;
        press.phase = "done";
        if (was !== "dragging" || press.active === null) {
            return;
        }
        const registration = press.active;
        const state = press.state as object;
        this.#world.schedule(`${registration.binding.constructor.name} "${registration.binding.id}".onCancel`, () => {
            const { event } = this.#pointerEvent(registration, press, press.at);
            this.#call(registration, "onCancel", () => registration.binding.onCancel(event, state));
        });
    }

    // -- gestures -----------------------------------------------------------------------------------------

    #gestureMessage(claim: ClaimEntry, message: InputGestureMessage): void {
        const sample: GestureSample = {
            dx: message.dx,
            dy: message.dy,
            scale: message.scale,
            rotation: message.rotation,
        };
        const run = this.#gestures.get(claim.id);
        switch (message.phase) {
            case "begin": {
                if (run !== undefined) {
                    this.#cancelGesture(run);
                }
                const client = this.hitTest(this.pointer)?.client ?? null;
                const fresh: GestureRun = {
                    claim,
                    fingers: message.fingers,
                    client,
                    candidates: this.#candidates(claim, client),
                    latest: sample,
                    lastTime: this.#host.now(),
                    started: null,
                    state: null,
                    dead: false,
                    done: false,
                    scheduled: false,
                };
                this.#gestures.set(claim.id, fresh);
                this.#world.schedule(`gesture ${claim.specKey}`, () => this.#decide(fresh));
                break;
            }
            case "update": {
                if (run === undefined) {
                    return;
                }
                // Latest wins: an update that is still waiting its turn is read at its turn, so the sample it
                // will see is this one, and a second call is not queued behind it.
                run.latest = sample;
                if (!run.scheduled) {
                    run.scheduled = true;
                    this.#world.schedule(`gesture ${claim.specKey}`, () => {
                        run.scheduled = false;
                        this.#gestureStep(run);
                    });
                }
                break;
            }
            case "end": {
                if (run === undefined) {
                    return;
                }
                run.latest = sample;
                this.#gestures.delete(claim.id);
                this.#world.schedule(`gesture end ${claim.specKey}`, () => this.#gestureEnd(run));
                break;
            }
            case "cancel": {
                if (run !== undefined) {
                    this.#cancelGesture(run);
                }
                break;
            }
        }
    }

    #gestureEvent(run: GestureRun, registration: Registration, phase: string): DragEvent {
        const trigger = registration.parsed as GestureTriggerParts;
        const sample = run.latest;
        const { event, now } = this.#event(registration, run.client, this.pointer, run.lastTime, {
            phase,
            fingers: run.fingers,
            dx: sample.dx,
            dy: sample.dy,
            scale: sample.scale,
            rotation: sample.rotation,
            progress: progressOf(trigger, sample),
        });
        run.lastTime = now;
        return event;
    }

    /** Starts the gesture on the best candidate whose direction has come true, once there is one. */
    #decide(run: GestureRun): void {
        if (run.started !== null || run.dead || run.done) {
            return;
        }
        for (const registration of run.candidates) {
            const parsed = registration.parsed as GestureTriggerParts;
            if (!this.#registrations.has(registration)) {
                continue;
            }
            if (parsed.direction !== null) {
                const way = parsed.gesture === "swipe" ? swipeDirection(run.latest) : pinchDirection(run.latest);
                if (way === null) {
                    // Not decided yet. A better candidate may still match, so the rest wait.
                    return;
                }
                if (way !== parsed.direction) {
                    continue;
                }
            }
            const state = this.#start(registration, this.#gestureEvent(run, registration, "begin"));
            if (state !== null) {
                run.started = registration;
                run.state = state;
                return;
            }
        }
        // Everyone who could take it has declined, or can never match the way it has gone.
        const undecided = run.candidates.some((registration) => {
            const parsed = registration.parsed as GestureTriggerParts;
            return (
                parsed.direction !== null &&
                (parsed.gesture === "swipe" ? swipeDirection(run.latest) : pinchDirection(run.latest)) === null
            );
        });
        run.dead = !undecided;
    }

    #gestureStep(run: GestureRun): void {
        if (run.done) {
            return;
        }
        this.#decide(run);
        if (run.started !== null) {
            const registration = run.started;
            const state = run.state as object;
            const event = this.#gestureEvent(run, registration, "update");
            this.#call(registration, "onUpdate", () => registration.binding.onUpdate(event, state));
        }
    }

    #gestureEnd(run: GestureRun): void {
        if (run.done) {
            return;
        }
        this.#decide(run);
        run.done = true;
        if (run.started !== null) {
            const registration = run.started;
            const state = run.state as object;
            const event = this.#gestureEvent(run, registration, "end");
            this.#call(registration, "onEnd", () => registration.binding.onEnd(event, state));
        }
    }

    #cancelGesture(run: GestureRun): void {
        this.#gestures.delete(run.claim.id);
        if (run.done) {
            return;
        }
        run.done = true;
        const registration = run.started;
        if (registration === null) {
            return;
        }
        const state = run.state as object;
        this.#world.schedule(`${registration.binding.constructor.name} "${registration.binding.id}".onCancel`, () => {
            const event = this.#gestureEvent(run, registration, "cancel");
            this.#call(registration, "onCancel", () => registration.binding.onCancel(event, state));
        });
    }
}

/** `gnoblin.input`: the seats the compositor reports as contexts, and their devices. */
export class InputManager {
    readonly #host: InputHost;
    readonly #world: InputWorld;
    readonly #contexts = new Map<number, InputContext>();

    constructor(host: InputHost, world: InputWorld) {
        this.#host = host;
        this.#world = world;
    }

    /** One context per seat, in tree order. */
    get contexts(): readonly InputContext[] {
        this.sync();
        return [...this.#contexts.values()];
    }

    /** The first seat's context: the ordinary default. Throws when the compositor reports no seat. */
    get primary(): InputContext {
        this.sync();
        const first = this.#contexts.values().next();
        if (first.done) {
            throw this.#world.error("no-seat", "gnoblin.input.primary", "the compositor reports no input seat");
        }
        return first.value;
    }

    get devices(): readonly InputDevice[] {
        return this.contexts.flatMap((context) => context.devices);
    }

    /** @internal Brings the contexts up to date with the seats, and each context's devices with its seat's. */
    sync(): void {
        const seats = this.#host.seats();
        for (const seat of seats) {
            if (!this.#contexts.has(seat)) {
                this.#contexts.set(seat, new InputContext(this.#host, this.#world, seat));
            }
        }
        for (const [seat, context] of [...this.#contexts]) {
            if (!seats.includes(seat)) {
                // The compositor released the seat's claims with it; there is nothing to unclaim on.
                context.dispose(false);
                this.#contexts.delete(seat);
            }
        }
        const layouts = this.#world.layouts();
        for (const context of this.#contexts.values()) {
            context.syncDevices();
            context.syncLayouts(layouts);
        }
    }

    /** @internal Takes the compositor's chord and gesture messages for this frame and schedules their hooks. */
    deliver(messages: readonly InputMessage[]): void {
        if (this.#contexts.size === 0 && messages.length === 0 && this.#host.seats().length === 0) {
            return;
        }
        this.sync();
        for (const message of messages) {
            for (const context of this.#contexts.values()) {
                if (context.ownsClaim(message.claim)) {
                    context.receive(message);
                    break;
                }
            }
        }
    }

    /** @internal A client was destroyed: the gestures on it are cancelled once. */
    clientGone(client: BindingClient): void {
        for (const context of this.#contexts.values()) {
            context.clientGone(client);
        }
    }

    /** @internal The generation is ending: cancel what is in flight and release every claim. */
    cancelAll(): void {
        for (const context of this.#contexts.values()) {
            context.dispose(true);
        }
        this.#contexts.clear();
    }
}

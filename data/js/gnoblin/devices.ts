// Input devices and their settings (owner-spec.md section 10): `InputDevice` and the five classes a seat's devices
// are, `Keyboard`, `Mouse`, `Touchpad`, `Touchscreen` and `Tablet`.
//
// A device is a physical thing the seat reports (`id:name:caps`); `classOf` picks the class from its capabilities. It
// belongs to one `InputContext` at a time (`context`, reassignable) and holds the bindings that match only its own
// events. The device settings the specification lists are properties of these classes: `sensitivity`,
// `acceleration`, `naturalScroll`, `tapToClick` and `repeatRate`, `repeatDelay`.
//
// What backs a setting. The compositor reads device settings from the `input` section of the configuration document
// (`input.keyboard`, `input.mouse`, `input.touchpad`), once per kind and not per device, so a write on one `Mouse` is
// the setting of every mouse and a read on another shows it. A write is checked here against the schema's range,
// then against the document validator when the shell has attached one (`attachDeviceSettings`), and joins the
// generation's document patch (`deviceDocument`); the shell commits it on load and again, a frame later, after any
// later write (`deviceSettingsRevision` moves). `undefined` takes a setting back to the system's. The keyboard group
// is applied by the compositor; the mouse and touchpad groups are accepted and validated, and the compositor reports
// them as not applied until a libinput applier reads them (crates/gnoblin-config/src/apply.rs). `enabled = false`
// has nothing behind it at all and is refused.
//
// This module reaches no host: the context it belongs to and the shell hooks are injected.

import { BindingCollection } from "gnoblin/bindings";
import type { Binding, BindingEvent } from "gnoblin/bindings";
import { NativeObject } from "gnoblin/native_object";

// ---- Settings ledger ----

/** The `input` document groups a device setting is written to. */
type Group = "keyboard" | "mouse" | "touchpad";

/** One setting: where it goes in the document, how a value is checked and what the document is given for it. */
interface Setting<T> {
    readonly group: Group;
    /** The document key, in the schema's spelling. */
    readonly key: string;
    /** The accepted values, for the error text. */
    readonly accepts: string;
    /** Returns the value when it is acceptable, else undefined. */
    accept(value: unknown): value is T;
    /** What the document holds for `value`. */
    document(value: T): unknown;
}

const isNumberIn =
    (min: number, max: number) =>
    (value: unknown): value is number =>
        typeof value === "number" && Number.isFinite(value) && value >= min && value <= max;

const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";

const ACCELERATION = ["default", "flat", "adaptive"] as const;
type Acceleration = (typeof ACCELERATION)[number];
const isAcceleration = (value: unknown): value is Acceleration => (ACCELERATION as readonly unknown[]).includes(value);

const identity = <T>(value: T): T => value;

/** The pointer speed of mice and touchpads: libinput's -1 to 1. */
const speed = (group: "mouse" | "touchpad"): Setting<number> => ({
    group,
    key: "speed",
    accepts: "a number from -1 to 1",
    accept: isNumberIn(-1, 1),
    document: identity,
});

const accelProfile = (group: "mouse" | "touchpad"): Setting<Acceleration> => ({
    group,
    key: "accel-profile",
    accepts: `one of ${ACCELERATION.join(", ")}`,
    accept: isAcceleration,
    document: identity,
});

const naturalScroll = (group: "mouse" | "touchpad"): Setting<boolean> => ({
    group,
    key: "natural-scroll",
    accepts: "true or false",
    accept: isBoolean,
    document: identity,
});

const SETTINGS = {
    mouseSensitivity: speed("mouse"),
    mouseAcceleration: accelProfile("mouse"),
    mouseNaturalScroll: naturalScroll("mouse"),
    touchpadSensitivity: speed("touchpad"),
    touchpadAcceleration: accelProfile("touchpad"),
    touchpadNaturalScroll: naturalScroll("touchpad"),
    tapToClick: {
        group: "touchpad",
        key: "tap-to-click",
        accepts: "true or false",
        accept: isBoolean,
        document: identity,
    } satisfies Setting<boolean>,
    // The document holds the time between repeats in milliseconds; the specification's `repeatRate` is a frequency.
    repeatRate: {
        group: "keyboard",
        key: "repeat-interval",
        accepts: "a number of repeats per second from 0.1 to 1000",
        accept: isNumberIn(0.1, 1000),
        document: (rate: number) => Math.round(1000 / rate),
    } satisfies Setting<number>,
    repeatDelay: {
        group: "keyboard",
        key: "delay",
        accepts: "a whole number of milliseconds from 1 to 10000",
        accept: (value: unknown): value is number => Number.isInteger(value) && isNumberIn(1, 10000)(value),
        document: identity,
    } satisfies Setting<number>,
} as const;

type SettingName = keyof typeof SETTINGS;

/** What the shell gives this module: the document validator and the error class the shell's objects throw. */
export interface DeviceSettingsHost {
    /** The validator's problems for a document patch (JSON text), or undefined when the document would be accepted. */
    check(patch: string): string | undefined;
    /** The error a refused operation on a native object throws. */
    error(code: string, target: string, message: string): Error;
}

let shell: DeviceSettingsHost | null = null;
/** What each written setting holds, in the order it was first written. */
const ledger = new Map<SettingName, unknown>();
let revision = 0;

/** Gives the module the shell's validator and error class. Called once by the shell as it starts a generation. */
export function attachDeviceSettings(host: DeviceSettingsHost): void {
    shell = host;
}

/**
 * A number that moves with every accepted change of a device setting. The shell commits the document again when it
 * has moved since the last commit.
 */
export function deviceSettingsRevision(): number {
    return revision;
}

type Table = { [key: string]: unknown };

/** The `input` section the written device settings make, `{}` when none is written. */
export function deviceDocument(): Table {
    const groups: Table = {};
    for (const [name, value] of ledger) {
        const setting: Setting<never> = SETTINGS[name] as Setting<never>;
        const group = (groups[setting.group] ??= {}) as Table;
        group[setting.key] = setting.document(value as never);
    }
    return Object.keys(groups).length === 0 ? {} : { input: groups };
}

/** `patch` laid over `base`, table by table; a list or a value in `patch` replaces what `base` holds. */
export function overlayDocument(base: Table, patch: Table): Table {
    const merged: Table = { ...base };
    for (const [key, value] of Object.entries(patch)) {
        const held = merged[key];
        const bothTables =
            typeof value === "object" &&
            value !== null &&
            !Array.isArray(value) &&
            typeof held === "object" &&
            held !== null &&
            !Array.isArray(held);
        merged[key] = bothTables ? overlayDocument(held as Table, value as Table) : value;
    }
    return merged;
}

/** Fails as `shell` does when it is attached, else as a plain `Error`. */
function refuse(code: string, target: string, message: string): Error {
    return shell?.error(code, target, message) ?? new Error(`${target}: ${message}`);
}

/** Writes `value` to the ledger for `name`, after checking it. `undefined` takes the setting back to the system's. */
function writeSetting(name: SettingName, value: unknown, target: string): void {
    const setting: Setting<unknown> = SETTINGS[name] as Setting<unknown>;
    if (value === undefined) {
        if (ledger.delete(name)) {
            revision += 1;
        }
        return;
    }
    if (!setting.accept(value)) {
        throw new RangeError(`${target}: must be ${setting.accepts}, got ${String(value)}`);
    }
    if (shell !== null) {
        const probe: Table = { input: { [setting.group]: { [setting.key]: setting.document(value) } } };
        const problem = shell.check(JSON.stringify(probe));
        if (problem !== undefined) {
            throw refuse("refused", target, `the configuration refused it: ${problem}`);
        }
    }
    if (!ledger.has(name) || ledger.get(name) !== value) {
        ledger.set(name, value);
        revision += 1;
    }
}

function readSetting<T>(name: SettingName): T | undefined {
    return ledger.get(name) as T | undefined;
}

// ---- Devices ----

/** What a device needs of the context it belongs to. `InputContext` implements it. */
export interface DeviceHome {
    readonly name: string;
    /** Makes a binding in the device's own collection and returns it. */
    bindDevice(
        device: InputDevice,
        trigger: string,
        action: ((event: BindingEvent) => void | false) | Binding | false,
        options?: DeviceBindOptions,
    ): Binding;
    isBindingOnly(name: string): boolean;
    makeBindingOnly(name: string): void;
    releaseBindingOnly(name: string): boolean;
    /** Moves `device` from this context to `to`, which must be a context of the same shell. */
    moveDevice(device: InputDevice, to: unknown): void;
}

/** What `InputDevice.bind` takes beside the trigger and the action. */
export interface DeviceBindOptions {
    /** `true` makes the device binding-only before the binding is made. The only value that means anything. */
    only?: true;
}

/**
 * A physical input device of a seat. Bindings made here match only its own events, and run before a layout's and
 * the context's. It belongs to one `InputContext`: the one that holds the seat's other devices until a script
 * assigns another to `context`.
 */
export class InputDevice extends NativeObject {
    readonly id: string;
    readonly name: string;
    readonly capabilities: readonly string[];
    readonly bindings = new BindingCollection();
    #context: DeviceHome;
    #alive = true;

    constructor(context: DeviceHome, id: string, name: string, capabilities: readonly string[]) {
        super();
        this.#context = context;
        this.id = id;
        this.name = name;
        this.capabilities = capabilities;
    }

    /** False once the device was unplugged: its context no longer lists it and a write on it is refused. */
    get alive(): boolean {
        return this.#alive;
    }

    /**
     * Whether the device is enabled. It always is: the compositor cannot disable one device, so assigning `false`
     * throws a `GnoblinError` (`unsupported`) and `true` changes nothing. Use `bindingOnly` to keep a device's keys
     * from clients.
     */
    get enabled(): boolean {
        return true;
    }

    set enabled(value: boolean) {
        if (typeof value !== "boolean") {
            throw new TypeError(`${this.path("enabled")} must be true or false, got ${String(value)}`);
        }
        if (!value) {
            throw refuse(
                "unsupported",
                this.path("enabled"),
                "the compositor cannot disable one device; make it binding-only to keep its keys from clients",
            );
        }
    }

    /**
     * The context the device belongs to. Assigning another moves it: the bindings of the old context stop matching
     * its events (a drag in flight is cancelled, so no button is left held) and those of the new one start. The
     * compositor has one seat that holds every device, so a context other than the seat's decides which bindings
     * match a device, not where its events are sent. Throws a `GnoblinError` for a context that is not part of
     * `gnoblin.input.contexts`, a dead device, or a seat the device does not belong to.
     */
    get context(): DeviceHome {
        return this.#context;
    }

    set context(to: DeviceHome) {
        if (!this.#alive) {
            throw refuse("destroyed", this.path("context"), "the device was unplugged");
        }
        if (to === this.#context) {
            return;
        }
        this.#context.moveDevice(this, to);
    }

    /** @internal Moves the device to `context` after the contexts have swapped its bindings. */
    assignContext(context: DeviceHome): void {
        this.#context = context;
    }

    /** @internal The device left the seat. */
    retire(): void {
        this.#alive = false;
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
        return this.#context.bindDevice(this, trigger, action, options);
    }

    /**
     * Whether this device is binding-only: its keys reach no client and fire only the bindings made for it or for
     * every device, so a macro pad never types into a window. Assigning `true` claims the device on the seat; `false`
     * gives the claim up. The claim is by the device's name, and lasts until the shell goes or it is given up.
     */
    get bindingOnly(): boolean {
        return this.#context.isBindingOnly(this.name);
    }

    set bindingOnly(on: boolean) {
        if (typeof on !== "boolean") {
            throw new TypeError(`InputDevice "${this.name}".bindingOnly must be true or false, got ${String(on)}`);
        }
        if (on) {
            this.#context.makeBindingOnly(this.name);
        } else {
            this.#context.releaseBindingOnly(this.name);
        }
    }

    /** @internal The error path of one of this device's properties, such as `Mouse#8.sensitivity`. */
    path(property: string): string {
        return `${this.constructor.name}#${this.id}.${property}`;
    }
}

/** A keyboard. Its repeat settings are those of every keyboard (see the header). */
export class Keyboard extends InputDevice {
    /**
     * Key repeats per second, from 0.1 to 1000, or `undefined` for the system's. Written to the document's
     * `input.keyboard.repeat-interval` (the reciprocal, in whole milliseconds), which the compositor applies.
     */
    get repeatRate(): number | undefined {
        return readSetting("repeatRate");
    }

    set repeatRate(value: number | undefined) {
        writeSetting("repeatRate", value, this.path("repeatRate"));
    }

    /**
     * Milliseconds a key is held before it repeats, a whole number from 1 to 10000, or `undefined` for the system's.
     * Written to `input.keyboard.delay`, which the compositor applies.
     */
    get repeatDelay(): number | undefined {
        return readSetting("repeatDelay");
    }

    set repeatDelay(value: number | undefined) {
        writeSetting("repeatDelay", value, this.path("repeatDelay"));
    }
}

/** A mouse, trackball or pointing stick. Its settings are those of every pointer that is not a touchpad. */
export class Mouse extends InputDevice {
    /**
     * Pointer speed from -1 (slowest) to 1, or `undefined` for the system's. Written to `input.mouse.speed`. The
     * compositor accepts and checks it and reports it as not applied until a device reads it.
     */
    get sensitivity(): number | undefined {
        return readSetting("mouseSensitivity");
    }

    set sensitivity(value: number | undefined) {
        writeSetting("mouseSensitivity", value, this.path("sensitivity"));
    }

    /**
     * `"default"`, `"flat"` or `"adaptive"`, or `undefined` for the system's. Written to `input.mouse.accel-profile`
     * (not applied by the compositor yet, as `sensitivity`). A custom curve has no property here.
     */
    get acceleration(): "default" | "flat" | "adaptive" | undefined {
        return readSetting("mouseAcceleration");
    }

    set acceleration(value: "default" | "flat" | "adaptive" | undefined) {
        writeSetting("mouseAcceleration", value, this.path("acceleration"));
    }

    /** Reverses the scroll direction, or `undefined` for the system's. Written to `input.mouse.natural-scroll`. */
    get naturalScroll(): boolean | undefined {
        return readSetting("mouseNaturalScroll");
    }

    set naturalScroll(value: boolean | undefined) {
        writeSetting("mouseNaturalScroll", value, this.path("naturalScroll"));
    }
}

/** A touchpad. Its settings are those of every touchpad. */
export class Touchpad extends InputDevice {
    /** As `Mouse.sensitivity`, written to `input.touchpad.speed`. */
    get sensitivity(): number | undefined {
        return readSetting("touchpadSensitivity");
    }

    set sensitivity(value: number | undefined) {
        writeSetting("touchpadSensitivity", value, this.path("sensitivity"));
    }

    /** As `Mouse.acceleration`, written to `input.touchpad.accel-profile`. */
    get acceleration(): "default" | "flat" | "adaptive" | undefined {
        return readSetting("touchpadAcceleration");
    }

    set acceleration(value: "default" | "flat" | "adaptive" | undefined) {
        writeSetting("touchpadAcceleration", value, this.path("acceleration"));
    }

    /** Reverses the scroll direction, or `undefined` for the system's. Written to `input.touchpad.natural-scroll`. */
    get naturalScroll(): boolean | undefined {
        return readSetting("touchpadNaturalScroll");
    }

    set naturalScroll(value: boolean | undefined) {
        writeSetting("touchpadNaturalScroll", value, this.path("naturalScroll"));
    }

    /** Treats a tap as a click, or `undefined` for the system's. Written to `input.touchpad.tap-to-click`. */
    get tapToClick(): boolean | undefined {
        return readSetting("tapToClick");
    }

    set tapToClick(value: boolean | undefined) {
        writeSetting("tapToClick", value, this.path("tapToClick"));
    }
}

/** A touch screen. The compositor has no setting for one: its calibration belongs to the seat. */
export class Touchscreen extends InputDevice {}

/** A drawing tablet's tool or pad. Tablet settings are keyed by vendor and product, which a device here does not carry. */
export class Tablet extends InputDevice {}

/**
 * The class a device with these capabilities (`Seat.device`'s words) is: a tablet tool or pad, a touch screen, a
 * touchpad (it reports gestures), a keyboard (also one with a pointing stick) or a mouse (any other pointer). A device
 * that is none of them, a lid switch, is the base `InputDevice`.
 */
export function classOf(capabilities: readonly string[]): typeof InputDevice {
    const has = (name: string): boolean => capabilities.includes(name);
    if (has("tablet-tool") || has("tablet-pad")) {
        return Tablet;
    }
    if (has("touch")) {
        return Touchscreen;
    }
    if (has("gesture")) {
        return Touchpad;
    }
    if (has("keyboard")) {
        return Keyboard;
    }
    return has("pointer") ? Mouse : InputDevice;
}

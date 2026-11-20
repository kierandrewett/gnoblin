import { BindingCollection } from "gnoblin-core/bindings/binding";
import type { Binding } from "gnoblin-core/bindings/binding";
import type { BindingEvent } from "gnoblin-core/bindings/types";
import { NativeObject } from "gnoblin-core/native_object";
import type { ErrorCode } from "gnoblin-core/errors";
/** What the shell gives this module: the document validator and the error class the shell's objects throw. */
export interface DeviceSettingsHost {
    /** The validator's problems for a document patch (JSON text), or undefined when the document would be accepted. */
    check(patch: string): string | undefined;
    /** The error a refused operation on a native object throws. */
    error(code: ErrorCode, target: string, message: string): Error;
}
/** Gives the module the shell's validator and error class. Called once by the shell as it starts a generation. */
export declare function attachDeviceSettings(host: DeviceSettingsHost): void;
/**
 * A number that moves with every accepted change of a device setting. The shell commits the document again when it
 * has moved since the last commit.
 */
export declare function deviceSettingsRevision(): number;
type Table = {
    [key: string]: unknown;
};
/** The `input` and `compositor` sections the written settings make, `{}` when none is written. */
export declare function deviceDocument(): Table;
/**
 * The settings of the compositor itself that a shell may write: `gnoblin.compositor`. They are written to the `compositor` section of
 * the configuration document, which the compositor reads when it starts, so a change is accepted at once and applies at the next
 * login. `undefined` takes a setting back to the default.
 */
export declare class CompositorSettings extends NativeObject {
    /**
     * Whether a shell may give a window its own shader (a `ShaderFilter` in a style). Written to
     * `compositor.user-shaders`, which defaults to `false` and is read when the compositor starts: after the first login that
     * sets it, log out and in again. See docs/custom-shaders.md.
     */
    get userShaders(): boolean | undefined;
    /** Sets whether the compositor accepts a shader effect from a shell, from the next login. */
    set userShaders(value: boolean | undefined);
}
/** What a device needs of the context it belongs to. `InputContext` implements it. */
export interface DeviceHome {
    /** Name of the context. */
    readonly name: string;
    /** Makes a binding in the device's own collection and returns it. */
    bindDevice(device: InputDevice, trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false, options?: DeviceBindOptions): Binding;
    /** Whether the named device's events are claimed by bindings only. */
    isBindingOnly(name: string): boolean;
    /** Keeps the device's events from reaching clients. */
    makeBindingOnly(name: string): void;
    /** Allows the device's events to reach clients again. */
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
export declare class InputDevice extends NativeObject {
    #private;
    /** Identifier reported by the seat. */
    readonly id: string;
    /** Device name reported by the seat. */
    readonly name: string;
    /** Capability names reported by the seat. */
    readonly capabilities: readonly string[];
    /** Bindings that respond only to this device's events. */
    readonly bindings: BindingCollection;
    constructor(context: DeviceHome, id: string, name: string, capabilities: readonly string[]);
    /** False once the device was unplugged: its context no longer lists it and a write on it is refused. */
    get alive(): boolean;
    /**
     * Whether the device is enabled. It always is: the compositor cannot disable one device, so assigning `false`
     * throws a `GnoblinError` (`unsupported`) and `true` changes nothing. Use `bindingOnly` to keep a device's keys
     * from clients.
     */
    get enabled(): boolean;
    /** Accepts `true`; assigning `false` throws `GnoblinError` (`unsupported`). */
    set enabled(value: boolean);
    /**
     * The context the device belongs to. Assigning another moves it: the bindings of the old context stop matching
     * its events (a drag in flight is cancelled, so no button is left held) and those of the new one start. The
     * compositor has one seat that holds every device, so a context other than the seat's decides which bindings
     * match a device, not where its events are sent. Throws a `GnoblinError` for a context that is not part of
     * `gnoblin.input.contexts`, a dead device, or a seat the device does not belong to.
     */
    get context(): DeviceHome;
    /** Moves the device to another input context. */
    set context(to: DeviceHome);
    /** @internal Moves the device to `context` after the contexts have swapped its bindings. */
    assignContext(context: DeviceHome): void;
    /** @internal The device left the seat. */
    retire(): void;
    /**
     * As `InputContext.bind`, for events from this device. Its bindings run before a layout's or the context's.
     * `options.only: true` first makes the device binding-only (see `bindingOnly`); `only: false` is refused, because
     * it is not a way to say nothing.
     */
    bind(trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false, options?: DeviceBindOptions): Binding;
    /**
     * Whether this device is binding-only: its keys reach no client and fire only the bindings made for it or for
     * every device, so a macro pad never types into a window. Assigning `true` claims the device on the seat; `false`
     * gives the claim up. The claim is by the device's name, and lasts until the shell goes or it is given up.
     */
    get bindingOnly(): boolean;
    /** Claims or releases the device so its events do not reach clients. */
    set bindingOnly(on: boolean);
    /** @internal The error path of one of this device's properties, such as `Mouse#8.sensitivity`. */
    path(property: string): string;
}
/** A keyboard. Its repeat settings are those of every keyboard (see the header). */
export declare class Keyboard extends InputDevice {
    /**
     * Key repeats per second, from 0.1 to 1000, or `undefined` for the system's. Written to the document's
     * `input.keyboard.repeat-interval` (the reciprocal, in whole milliseconds), which the compositor applies.
     */
    get repeatRate(): number | undefined;
    /** Sets the shared key repeat rate in repeats per second. */
    set repeatRate(value: number | undefined);
    /**
     * Milliseconds a key is held before it repeats, a whole number from 1 to 10000, or `undefined` for the system's.
     * Written to `input.keyboard.delay`, which the compositor applies.
     */
    get repeatDelay(): number | undefined;
    /** Sets the shared key repeat delay in milliseconds. */
    set repeatDelay(value: number | undefined);
}
/** A mouse, trackball or pointing stick. Its settings are those of every pointer that is not a touchpad. */
export declare class Mouse extends InputDevice {
    /**
     * Pointer speed from -1 (slowest) to 1, or `undefined` for the system's. Written to `input.mouse.speed`. The
     * hardware backend gives it to every mouse through libinput; a nested session does not (its host decides).
     */
    get sensitivity(): number | undefined;
    /** Sets the shared pointer speed from -1 to 1. */
    set sensitivity(value: number | undefined);
    /**
     * `"default"`, `"flat"` or `"adaptive"`, or `undefined` for the system's. Written to `input.mouse.accel-profile`
     * (applied as `sensitivity` is). A custom curve has no property here.
     */
    get acceleration(): "default" | "flat" | "adaptive" | undefined;
    /** Sets the shared pointer acceleration profile. */
    set acceleration(value: "default" | "flat" | "adaptive" | undefined);
    /** Reverses the scroll direction, or `undefined` for the system's. Written to `input.mouse.natural-scroll`. */
    get naturalScroll(): boolean | undefined;
    /** Sets the shared natural scrolling preference. */
    set naturalScroll(value: boolean | undefined);
}
/** A touchpad. Its settings are those of every touchpad. */
export declare class Touchpad extends InputDevice {
    /** As `Mouse.sensitivity`, written to `input.touchpad.speed`. */
    get sensitivity(): number | undefined;
    /** Sets the shared touchpad pointer speed from -1 to 1. */
    set sensitivity(value: number | undefined);
    /** As `Mouse.acceleration`, written to `input.touchpad.accel-profile`. */
    get acceleration(): "default" | "flat" | "adaptive" | undefined;
    /** Sets the shared touchpad acceleration profile. */
    set acceleration(value: "default" | "flat" | "adaptive" | undefined);
    /** Reverses the scroll direction, or `undefined` for the system's. Written to `input.touchpad.natural-scroll`. */
    get naturalScroll(): boolean | undefined;
    /** Sets the shared natural scrolling preference. */
    set naturalScroll(value: boolean | undefined);
    /** Treats a tap as a click, or `undefined` for the system's. Written to `input.touchpad.tap-to-click`. */
    get tapToClick(): boolean | undefined;
    /** Sets whether a tap acts as a click. */
    set tapToClick(value: boolean | undefined);
}
/** A touch screen. The compositor has no setting for one: its calibration belongs to the seat. */
export declare class Touchscreen extends InputDevice {
}
/**
 * A drawing tablet's tool or pad. Tablet settings are keyed by vendor and product, which a device here does not
 * carry.
 */
export declare class Tablet extends InputDevice {
}
/**
 * The class a device with these capabilities (`Seat.device`'s words) is: a tablet tool or pad, a touch screen, a
 * touchpad (it reports gestures), a keyboard (also one with a pointing stick) or a mouse (any other pointer). A device
 * that is none of them, a lid switch, is the base `InputDevice`.
 */
export declare function classOf(capabilities: readonly string[]): typeof InputDevice;
export {};

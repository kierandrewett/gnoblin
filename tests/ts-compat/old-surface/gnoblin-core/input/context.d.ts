import type { Binding } from "gnoblin-core/bindings/binding";
import type { BindingClient, BindingEvent, Point } from "gnoblin-core/bindings/types";
import { InputDevice } from "gnoblin-core/devices";
import type { DeviceHome } from "gnoblin-core/devices";
import { NativeObject } from "gnoblin-core/native_object";
import type { Client } from "gnoblin-core/host/client";
import { BindingCollection } from "gnoblin-core/bindings/binding";
import type { InputHit, InputHost, InputLayout, InputWorld, SeatSettings } from "gnoblin-core/input/host";
import type { InputMessage } from "gnoblin-core/input/messages";
import type { InputBindingInfo } from "gnoblin-core/input/registration";
/** Provides the host and world used by contexts in this generation. */
export declare function bindInputRuntime(host: InputHost, world: InputWorld): void;
/** What `new InputContext` takes. */
export interface InputContextOptions {
    /** The context's name: one non-empty line, unique among the contexts of `gnoblin.input.contexts`. */
    name: string;
}
/**
 * An input context owns bindings for a group of devices. The compositor reports one context per seat; scripts can
 * add contexts that share the primary seat's devices.
 *
 * Context bindings apply to all held devices. Device bindings apply only to their device, and layout bindings apply
 * to clients whose workspace uses that layout. Pointer and keyboard focus remain those of the underlying seat.
 */
export declare class InputContext extends NativeObject implements DeviceHome {
    #private;
    /** Name assigned to this context. */
    readonly name: string;
    /** Bindings registered directly on this context. */
    readonly bindings: BindingCollection;
    /**
     * Makes a context that is in no manager yet: it collects bindings, and holds no device and makes no claim until it
     * is added to `gnoblin.input.contexts`. Throws a `TypeError` for a name that is not one non-empty line.
     */
    constructor(options: InputContextOptions);
    /** The name: a context's identity among the contexts of `gnoblin.input.contexts`. */
    get id(): string;
    /** False once the context was removed from `gnoblin.input.contexts` or its seat went. */
    get alive(): boolean;
    /** The devices this context holds now. A context that is not in `gnoblin.input.contexts` holds none. */
    get devices(): readonly InputDevice[];
    /**
     * The client with keyboard focus, or null. Read through to the seat on every call. The world is the shell's, so the
     * client is its own `Client` (`close()`, `moveToWorkspace(...)`), though the stock bindings ask only for a
     * `BindingClient` of it.
     */
    get focusedClient(): Client | null;
    /** The window management settings the seat publishes. */
    get settings(): SeatSettings;
    /** Takes the keyboard focus from every window. */
    clearFocus(): void;
    /**
     * Whether the compositor plays its own animations on window nodes: the fade in when a window maps, the fade out
     * when it closes, unmaps or hides, and the tween of a window's placement towards its value. True until a shell
     * turns it off, which a shell does once, at startup, when it animates windows itself: a window then appears, moves
     * and leaves with no compositor motion. It affects only the changes that start after the write. The compositor
     * turns it on again when the shell lets go of the tree. Reduced motion still turns every animation off.
     */
    get builtinMotion(): boolean;
    set builtinMotion(enabled: boolean);
    /** The pointer in the compositor's logical space. */
    get pointer(): Point;
    /**
     * Binds `action` to `trigger` for this context and returns the binding; `unbind()` on it, or
     * `bindings.remove(id)`, takes it away and releases the compositor claim when no other binding needs it.
     * Throws a `TriggerError` for a bad trigger and a `BindingConflictError` when another binding of this
     * context matches it at the same priority.
     */
    bind(trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false): Binding;
    /** Whether the devices that report `name` are binding-only on this seat. */
    isBindingOnly(name: string): boolean;
    /**
     * Makes the devices that report `name` (a name from `devices`) binding-only: a key of theirs reaches no client and
     * fires only the bindings made for the device or for every device, and a chord no binding claims is dropped. Once
     * is enough: a second call changes nothing. Works before the device is plugged in.
     */
    makeBindingOnly(name: string): void;
    /** Gives the binding-only claim on `name` up, so its keys reach clients again. False when there was none. */
    releaseBindingOnly(name: string): boolean;
    /** Returns each registered binding and its rank among bindings of the same trigger. */
    describeBindings(): InputBindingInfo[];
    /** The topmost client whose bounds hold `position`, or null. */
    hitTest(position: Point): InputHit | null;
    /**
     * Makes the bindings of `layout` live for this context, for the clients `owns` accepts. Without `owns`
     * that is a client whose workspace's `layout` is this one. Bindings the layout gains or loses later follow.
     */
    attachLayout(layout: InputLayout, owns?: (client: BindingClient) => boolean): void;
    /** Stops the layout's bindings. A drag one of them runs is cancelled. */
    detachLayout(layout: InputLayout): void;
    /** @internal Attaches the layouts the workspaces use and detaches the ones they no longer use. */
    syncLayouts(wanted: readonly InputLayout[]): void;
    /** @internal Whether `claim` is one of this context's. */
    ownsClaim(claim: number): boolean;
    /**
     * @internal Puts the context into `gnoblin.input.contexts` on `seat`: the seat's own context (`virtual` false)
     * makes claims for its bindings at once; one a script made holds devices and registers its bindings per device.
     */
    attach(seat: number, virtual: boolean): void;
    /** @internal The seat this context makes claims on, or -1 before it is in `gnoblin.input.contexts`. */
    get seat(): number;
    /** @internal Whether a script made this context, as opposed to the seat's own. */
    get virtual(): boolean;
    /**
     * @internal Brings the roster up to date with the seat's device list: a device that appeared joins this context, a
     * device that went is retired and leaves whichever context held it. Does nothing for a context that is not a seat.
     */
    syncDevices(): void;
    /** Makes a binding for one of this context's devices, in the device's own collection. */
    bindDevice(device: InputDevice, trigger: string, action: ((event: BindingEvent) => void | false) | Binding | false): Binding;
    /**
     * Moves `device` to context `to`: its bindings, and the bindings `to` made for all its devices, start matching the
     * device's events, and this context's stop. A drag the old bindings run is cancelled, and the device's
     * binding-only claim goes with it. Refused (`GnoblinError`) for a context that is not in `gnoblin.input.contexts`
     * and for one on another seat, because the compositor attaches every device to one seat.
     */
    moveDevice(device: InputDevice, to: unknown): void;
    /** @internal Takes one message from the compositor and schedules what it means. */
    receive(message: InputMessage): void;
    /** @internal A client died: gestures on it are cancelled once, and a press on it is dropped. */
    clientGone(client: BindingClient): void;
    /**
     * @internal Cancels everything in flight and gives up every claim: the context is going away. `release`
     * is false when the seat is already gone, so there is nothing to unclaim on.
     */
    dispose(release: boolean): void;
}

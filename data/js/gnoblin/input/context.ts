// Handles input for one logical group of devices. The compositor claims triggers for the context; matchers route
// incoming chord, pointer and gesture events to its bindings.

// Bindings run in device, layout, then context order. Within each tier, priority decides; a binding can decline an
// event to let the next candidate handle it. Equal-priority bindings in one collection cannot share a trigger.
//
// Each hook runs as a scheduled call. A drag keeps only its latest pending position, while starts, ends and cancels
// remain distinct.

import type { Binding } from "gnoblin/bindings/binding";
import type { BindingClient, BindingEvent, Point } from "gnoblin/bindings/types";
import { InputDevice, classOf } from "gnoblin/devices";
import type { DeviceHome } from "gnoblin/devices";
import { errorCodeOfRefusal } from "gnoblin/errors";
import { NativeObject } from "gnoblin/native_object";
import type { Client } from "gnoblin/host/client";
import type { Display } from "gnoblin/display/display";
import { BindingCollection } from "gnoblin/bindings/binding";
import { bindIn, checkDeviceName, splitDevice } from "gnoblin/input/bind";
import { ClaimTable } from "gnoblin/input/claims";
import { GestureMatcher } from "gnoblin/input/gesture";
import { Hooks } from "gnoblin/input/hooks";
import type { MatchEnv } from "gnoblin/input/hooks";
import { SEAT_DEFAULTS } from "gnoblin/input/host";
import type { InputHit, InputHost, InputLayout, InputWorld, SeatSettings } from "gnoblin/input/host";
import { KeyMatcher } from "gnoblin/input/keys";
import type { InputMessage } from "gnoblin/input/messages";
import { PointerMatcher } from "gnoblin/input/pointer";
import { TIER_CONTEXT, TIER_DEVICE, TIER_LAYOUT } from "gnoblin/input/registration";
import type { InputBindingInfo } from "gnoblin/input/registration";

/** The layout a client's workspace shows, for a layout attached without its own ownership test. */
function layoutOf(client: BindingClient): unknown {
    const workspace: unknown = Reflect.get(client, "workspace");
    return typeof workspace === "object" && workspace !== null ? Reflect.get(workspace, "layout") : undefined;
}

/** Runtime services shared by contexts in this generation. */
let runtime: { host: InputHost; world: InputWorld } | null = null;

/** Provides the host and world used by contexts in this generation. */
export function bindInputRuntime(host: InputHost, world: InputWorld): void {
    runtime = { host, world };
}

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
export class InputContext extends NativeObject implements DeviceHome {
    /** Name assigned to this context. */
    readonly name: string;
    /** Bindings registered directly on this context. */
    readonly bindings = new BindingCollection();

    readonly #host: InputHost;
    readonly #world: InputWorld;
    /** The seat whose claims this context makes; -1 while the context is not in `gnoblin.input.contexts`. */
    #seat = -1;
    /** Made by a script: it holds devices of a seat, not a seat. */
    #virtual = false;
    #state: "detached" | "live" | "removed" = "detached";
    readonly #table: ClaimTable;
    readonly #keys: KeyMatcher;
    readonly #pointers: PointerMatcher;
    readonly #gestures: GestureMatcher;
    readonly #layouts = new Map<InputLayout, () => void>();
    /** Every device the seat reports, whichever context holds it. Only a context that is a seat has any. */
    readonly #roster = new Map<string, InputDevice>();
    /** The devices this context holds, each with the call that gives up what its bindings registered. */
    readonly #owned = new Map<InputDevice, () => void>();
    /** The claim of each binding-only device, by its name. */
    readonly #only = new Map<string, number>();
    #disposed = false;
    /** Whether giving up a claim tells the seat: false once the seat itself is gone. */
    #release = true;
    #unfollowOwn: () => void = () => {};
    /**
     * The layouts `syncLayouts` attached, which it also detaches. One attached by hand stays until detached by hand.
     */
    readonly #auto = new Set<InputLayout>();
    /** Layouts whose bindings were refused once, so they are not tried and reported every frame. */
    readonly #failed = new WeakSet<InputLayout>();

    /**
     * Makes a context that is in no manager yet: it collects bindings, and holds no device and makes no claim until it
     * is added to `gnoblin.input.contexts`. Throws a `TypeError` for a name that is not one non-empty line.
     */
    constructor(options: InputContextOptions) {
        super();
        if (runtime === null) {
            throw new Error("an InputContext needs the shell's input manager, which this generation has not started");
        }
        if (options === null || typeof options !== "object") {
            throw new TypeError("InputContext: expects an object with a name");
        }
        if (typeof options.name !== "string") {
            throw new TypeError(`InputContext: the name must be text, got ${String(options.name)}`);
        }
        this.name = options.name;
        this.#host = runtime.host;
        this.#world = runtime.world;
        const env: MatchEnv = {
            context: this,
            host: this.#host,
            world: this.#world,
            claims: undefined as unknown as ClaimTable,
            seat: () => this.#seat,
            settings: () => this.settings,
            pointer: () => this.pointer,
            hitTest: (position) => this.hitTest(position),
            focusedClient: () => this.focusedClient,
            deviceNamed: (name) => [...this.#owned.keys()].find((owned) => owned.name === name) ?? null,
        };
        this.#table = new ClaimTable(
            this.#host,
            this.#world,
            this.name,
            () => this.#seat,
            () => this.#release,
            (registration) => {
                this.#pointers.cancelBy(registration);
                this.#gestures.cancelBy(registration);
            },
        );
        Object.defineProperty(env, "claims", { value: this.#table });
        const hooks = new Hooks(env);
        this.#keys = new KeyMatcher(env, hooks);
        this.#pointers = new PointerMatcher(env, hooks);
        this.#gestures = new GestureMatcher(env, hooks);
    }

    /** The name: a context's identity among the contexts of `gnoblin.input.contexts`. */
    get id(): string {
        return this.name;
    }

    /** False once the context was removed from `gnoblin.input.contexts` or its seat went. */
    get alive(): boolean {
        return this.#state !== "removed";
    }

    /** The devices this context holds now. A context that is not in `gnoblin.input.contexts` holds none. */
    get devices(): readonly InputDevice[] {
        this.syncDevices();
        return this.#virtual ? [...this.#owned.keys()] : [...this.#roster.values()].filter((d) => this.#owned.has(d));
    }

    /**
     * The client with keyboard focus, or null. Read through to the seat on every call. The world is the shell's, so the
     * client is its own `Client` (`close()`, `moveToWorkspace(...)`), though the stock bindings ask only for a
     * `BindingClient` of it.
     */
    get focusedClient(): Client | null {
        return this.#state === "live"
            ? ((this.#world.clientOf(this.#host.focus(this.#seat)) as Client | undefined) ?? null)
            : null;
    }

    /** The window management settings the seat publishes. */
    get settings(): SeatSettings {
        return this.#host.settings(this.#seat) ?? SEAT_DEFAULTS;
    }

    /** Takes the keyboard focus from every window. */
    clearFocus(): void {
        this.#live("clearFocus");
        const code = this.#host.clearFocus(this.#seat);
        if (code != null) {
            throw this.#world.error(
                errorCodeOfRefusal(code),
                `InputContext "${this.name}".clearFocus`,
                `the host refused (${code})`,
            );
        }
    }

    /**
     * The display under the pointer, or the primary display when the pointer is outside every output. Returns null
     * when no display has an output. Assigning `viewport.content` switches that display's workspace.
     */
    get activeDisplay(): Display | null {
        return this.#world.displayAt(this.pointer) as Display | null;
    }

    /** The pointer in the compositor's logical space. */
    get pointer(): Point {
        this.#live("pointer");
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
        this.#live("makeBindingOnly");
        if (this.#only.has(name)) {
            return;
        }
        const result = this.#host.claimDevice(this.#seat, name);
        if (typeof result === "string") {
            throw this.#world.error(
                errorCodeOfRefusal(result),
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

    /** Returns each registered binding and its rank among bindings of the same trigger. */
    describeBindings(): InputBindingInfo[] {
        return this.#table.describe();
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
        this.#live("attachLayout");
        if (this.#layouts.has(layout)) {
            throw new Error("this layout is already attached to the input context");
        }
        const scope = owns ?? ((client: BindingClient) => layoutOf(client) === layout);
        this.#layouts.set(
            layout,
            this.#table.follow(layout.bindings, TIER_LAYOUT, (client) => client !== null && scope(client), layout, ""),
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
        return this.#table.byId(claim) !== undefined;
    }

    /**
     * @internal Puts the context into `gnoblin.input.contexts` on `seat`: the seat's own context (`virtual` false)
     * makes claims for its bindings at once; one a script made holds devices and registers its bindings per device.
     */
    attach(seat: number, virtual: boolean): void {
        this.#seat = seat;
        this.#virtual = virtual;
        this.#state = "live";
        if (!virtual) {
            this.#unfollowOwn = this.#table.follow(this.bindings, TIER_CONTEXT, () => true, null, "");
        }
    }

    /** @internal The seat this context makes claims on, or -1 before it is in `gnoblin.input.contexts`. */
    get seat(): number {
        return this.#seat;
    }

    /** @internal Whether a script made this context, as opposed to the seat's own. */
    get virtual(): boolean {
        return this.#virtual;
    }

    /**
     * @internal Brings the roster up to date with the seat's device list: a device that appeared joins this context, a
     * device that went is retired and leaves whichever context held it. Does nothing for a context that is not a seat.
     */
    syncDevices(): void {
        if (this.#state !== "live" || this.#virtual) {
            return;
        }
        const seen = new Set<string>();
        for (const item of this.#host.devices(this.#seat)) {
            const parts = splitDevice(item);
            if (parts === null) {
                continue;
            }
            seen.add(parts.id);
            if (!this.#roster.has(parts.id)) {
                const device = new (classOf(parts.capabilities))(this, parts.id, parts.name, parts.capabilities);
                this.#roster.set(parts.id, device);
                this.#own(device);
            }
        }
        for (const [id, device] of [...this.#roster]) {
            if (!seen.has(id)) {
                this.#roster.delete(id);
                (device.context as InputContext).#disown(device);
                device.retire();
            }
        }
    }

    /** Starts matching this device's bindings, plus this context's bindings when it is script-created. */
    #own(device: InputDevice): void {
        const undo: (() => void)[] = [];
        try {
            undo.push(this.#table.follow(device.bindings, TIER_DEVICE, () => true, null, device.name));
            if (this.#virtual) {
                undo.push(this.#table.follow(this.bindings, TIER_CONTEXT, () => true, null, device.name));
            }
        } catch (error) {
            for (const give of undo) {
                give();
            }
            throw error;
        }
        this.#owned.set(device, () => {
            for (const give of undo) {
                give();
            }
        });
    }

    /** Stops matching `device`'s events: a drag one of its bindings runs is cancelled, so no button stays held. */
    #disown(device: InputDevice): void {
        this.#owned.get(device)?.();
        this.#owned.delete(device);
    }

    /** Throws unless the context is in `gnoblin.input.contexts`. */
    #live(what: string): void {
        if (this.#state !== "live") {
            throw this.#world.error(
                this.#state === "removed" ? "not_found" : "invalid_state",
                `InputContext "${this.name}".${what}`,
                this.#state === "removed"
                    ? "the context was removed from gnoblin.input.contexts"
                    : "add the context to gnoblin.input.contexts first",
            );
        }
    }

    /** Makes a binding for one of this context's devices, in the device's own collection. */
    bindDevice(
        device: InputDevice,
        trigger: string,
        action: ((event: BindingEvent) => void | false) | Binding | false,
    ): Binding {
        return bindIn(device.bindings, trigger, action, `InputDevice "${device.name}"`);
    }

    /**
     * Moves `device` to context `to`: its bindings, and the bindings `to` made for all its devices, start matching the
     * device's events, and this context's stop. A drag the old bindings run is cancelled, and the device's
     * binding-only claim goes with it. Refused (`GnoblinError`) for a context that is not in `gnoblin.input.contexts`
     * and for one on another seat, because the compositor attaches every device to one seat.
     */
    moveDevice(device: InputDevice, to: unknown): void {
        const target = `${device.path("context")}`;
        if (!(to instanceof InputContext) || to.#state !== "live") {
            throw this.#world.error("invalid_argument", target, "assign a context that is in gnoblin.input.contexts");
        }
        if (to.#seat !== this.#seat) {
            throw this.#world.error(
                "unsupported",
                target,
                `the compositor attaches "${device.name}" to seat ${this.#seat}, so it cannot move to context "${to.name}"`,
            );
        }
        const wasOnly = this.releaseBindingOnly(device.name);
        this.#disown(device);
        try {
            to.#own(device);
            if (wasOnly) {
                to.makeBindingOnly(device.name);
            }
        } catch (error) {
            to.#disown(device);
            this.#own(device);
            if (wasOnly) {
                this.makeBindingOnly(device.name);
            }
            throw error;
        }
        device.assignContext(to);
    }

    /** @internal Takes one message from the compositor and schedules what it means. */
    receive(message: InputMessage): void {
        const claim = this.#table.byId(message.claim);
        if (claim === undefined) {
            return;
        }
        if (message.kind === "chord") {
            // A pointer chord's press and release are the `pointer` messages' start and end: the compositor
            // sends no `chord` for them.
            if (claim.spec.kind === "chord" && !claim.pointer && message.state === "press") {
                this.#keys.press(claim);
            }
            return;
        }
        if (message.kind === "pointer") {
            if (claim.pointer) {
                this.#pointers.message(claim, message);
            }
            return;
        }
        if (claim.spec.kind === "gesture") {
            this.#gestures.message(claim, message);
        }
    }

    /** @internal A client died: gestures on it are cancelled once, and a press on it is dropped. */
    clientGone(client: BindingClient): void {
        this.#pointers.cancelOn(client);
        this.#gestures.cancelOn(client);
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
        this.#pointers.cancelAll();
        this.#gestures.cancelAll();
        this.#unfollowOwn();
        for (const give of this.#owned.values()) {
            give();
        }
        this.#owned.clear();
        this.#roster.clear();
        this.#state = "removed";
        for (const unfollow of [...this.#layouts.values()]) {
            unfollow();
        }
        this.#layouts.clear();
        this.#auto.clear();
        // Every registration's claim went with it; this is the net for a claim nothing registered against.
        this.#table.drop(release);
        for (const id of this.#only.values()) {
            if (release) {
                this.#host.unclaim(this.#seat, id);
            }
        }
        this.#only.clear();
    }
}

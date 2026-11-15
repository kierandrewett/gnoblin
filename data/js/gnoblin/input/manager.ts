// Manages input contexts and devices, keeps them in sync with seats, and routes compositor messages to their contexts.

import type { BindingClient } from "gnoblin/bindings/types";
import { InputList, MutableInputList } from "gnoblin/input_list";
import type { InputDevice } from "gnoblin/devices";
import { InputContext, bindInputRuntime } from "gnoblin/input/context";
import type { InputHost, InputWorld } from "gnoblin/input/host";
import type { InputMessage } from "gnoblin/input/messages";

/**
 * Manages the contexts and devices exposed as `gnoblin.input`. There is one context per seat, followed by any
 * contexts added by scripts. `primary` is the first seat's context.
 */
export class InputManager {
    /** The seats' contexts, then the scripts'. `add` and `remove` are for the scripts' own. */
    readonly contexts: MutableInputList<InputContext>;
    /** Every device of every context. */
    readonly devices: InputList<InputDevice>;

    readonly #host: InputHost;
    readonly #world: InputWorld;
    readonly #seats = new Map<number, InputContext>();
    /** The contexts a script made and added, in the order it added them. */
    #scripted: InputContext[] = [];

    constructor(host: InputHost, world: InputWorld) {
        this.#host = host;
        this.#world = world;
        bindInputRuntime(host, world);
        this.contexts = new MutableInputList(
            () => this.#all(),
            (context) => this.#add(context),
            (context) => this.#remove(context),
        );
        this.devices = new InputList(() => this.#all().flatMap((context) => context.devices));
    }

    /** The first seat's context: the ordinary default. Throws when the compositor reports no seat. */
    get primary(): InputContext {
        this.sync();
        const first = this.#seats.values().next();
        if (first.done) {
            throw this.#world.error("unavailable", "gnoblin.input.primary", "the compositor reports no input seat");
        }
        return first.value;
    }

    /** Every context, up to date with the seats. */
    #all(): InputContext[] {
        this.sync();
        return [...this.#seats.values(), ...this.#scripted];
    }

    /**
     * Puts a context a script made into `contexts`, over the primary seat's devices. Refused (`GnoblinError`) when the
     * name is taken, the context was removed, or the compositor reports no seat; a name that is not one non-empty
     * line is a `TypeError`. Adding a context that is already there changes nothing.
     */
    #add(context: InputContext): InputContext {
        const where = "gnoblin.input.contexts.add";
        if (!(context instanceof InputContext)) {
            throw new TypeError(`${where}: expects an InputContext made with new InputContext({ name })`);
        }
        const present = this.#all();
        if (present.includes(context)) {
            return context;
        }
        if (!context.alive) {
            throw this.#world.error("not_found", where, `the context "${context.name}" was removed`);
        }
        if (context.name === "" || context.name.includes("\n")) {
            throw new TypeError(`${where}: a context name is one non-empty line, got ${JSON.stringify(context.name)}`);
        }
        if (present.some((other) => other.name === context.name)) {
            throw this.#world.error("already_exists", where, `a context named "${context.name}" is already there`);
        }
        const [seat] = this.#seats.values();
        if (seat === undefined) {
            throw this.#world.error("unavailable", where, "the compositor reports no input seat to hold the context");
        }
        context.attach(seat.seat, true);
        this.#scripted.push(context);
        return context;
    }

    /**
     * Takes a context a script added out of `contexts`: its devices go back to their seat's context and its bindings
     * stop. False for a context that is not there. Refused (`GnoblinError`) for a seat's own context.
     */
    #remove(context: InputContext): boolean {
        if ([...this.#seats.values()].includes(context)) {
            throw this.#world.error(
                "denied",
                "gnoblin.input.contexts.remove",
                `"${context.name}" is a seat the compositor reports; only contexts a script added can be removed`,
            );
        }
        if (!this.#scripted.includes(context)) {
            return false;
        }
        const home = [...this.#seats.values()].find((seat) => seat.seat === context.seat);
        for (const device of [...context.devices]) {
            if (home !== undefined) {
                device.context = home;
            }
        }
        this.#scripted = this.#scripted.filter((other) => other !== context);
        context.dispose(true);
        return true;
    }

    /** @internal Brings the contexts up to date with the seats, and each context's devices with its seat's. */
    sync(): void {
        const seats = this.#host.seats();
        for (const seat of seats) {
            if (!this.#seats.has(seat)) {
                const context = new InputContext({ name: this.#host.seatName(seat) });
                context.attach(seat, false);
                this.#seats.set(seat, context);
            }
        }
        for (const [seat, context] of [...this.#seats]) {
            if (!seats.includes(seat)) {
                // The compositor released the seat's claims with it; there is nothing to unclaim on. A context a script
                // made on that seat has nothing left to hold either.
                for (const scripted of this.#scripted.filter((other) => other.seat === seat)) {
                    scripted.dispose(false);
                }
                this.#scripted = this.#scripted.filter((other) => other.seat !== seat);
                context.dispose(false);
                this.#seats.delete(seat);
            }
        }
        const layouts = this.#world.layouts();
        for (const context of this.#seats.values()) {
            context.syncDevices();
            context.syncLayouts(layouts);
        }
    }

    /** @internal Takes the compositor's chord and gesture messages for this frame and schedules their hooks. */
    deliver(messages: readonly InputMessage[]): void {
        if (this.#seats.size === 0 && messages.length === 0 && this.#host.seats().length === 0) {
            return;
        }
        this.sync();
        for (const message of messages) {
            for (const context of [...this.#seats.values(), ...this.#scripted]) {
                if (context.ownsClaim(message.claim)) {
                    context.receive(message);
                    break;
                }
            }
        }
    }

    /** @internal A client was destroyed: the gestures on it are cancelled once. */
    clientGone(client: BindingClient): void {
        for (const context of [...this.#seats.values(), ...this.#scripted]) {
            context.clientGone(client);
        }
    }

    /** @internal The generation is ending: cancel what is in flight and release every claim. */
    cancelAll(): void {
        for (const context of [...this.#scripted, ...this.#seats.values()]) {
            context.dispose(true);
        }
        this.#scripted = [];
        this.#seats.clear();
    }
}

import type { BindingClient } from "gnoblin-core/bindings/types";
import { InputList, MutableInputList } from "gnoblin-core/input_list";
import type { InputDevice } from "gnoblin-core/devices";
import { InputContext } from "gnoblin-core/input/context";
import type { InputHost, InputWorld } from "gnoblin-core/input/host";
import type { InputMessage } from "gnoblin-core/input/messages";
/**
 * Manages the contexts and devices exposed as `gnoblin.input`. There is one context per seat, followed by any
 * contexts added by scripts. `primary` is the first seat's context.
 */
export declare class InputManager {
    #private;
    /** The seats' contexts, then the scripts'. `add` and `remove` are for the scripts' own. */
    readonly contexts: MutableInputList<InputContext>;
    /** Every device of every context. */
    readonly devices: InputList<InputDevice>;
    constructor(host: InputHost, world: InputWorld);
    /** The first seat's context: the ordinary default. Throws when the compositor reports no seat. */
    get primary(): InputContext;
    /** @internal Brings the contexts up to date with the seats, and each context's devices with its seat's. */
    sync(): void;
    /** @internal Takes the compositor's chord and gesture messages for this frame and schedules their hooks. */
    deliver(messages: readonly InputMessage[]): void;
    /** @internal A client was destroyed: the gestures on it are cancelled once. */
    clientGone(client: BindingClient): void;
    /** @internal The generation is ending: cancel what is in flight and release every claim. */
    cancelAll(): void;
}

import type { BindingCollection } from "gnoblin-core/bindings/binding";
import type { BindingClient } from "gnoblin-core/bindings/types";
import type { InputHost, InputLayout, InputWorld } from "gnoblin-core/input/host";
import type { ClaimEntry, InputBindingInfo, Registration } from "gnoblin-core/input/registration";
/** Tracks the claims and registrations held by one input context. */
export declare class ClaimTable {
    #private;
    constructor(host: InputHost, world: InputWorld, name: string, seat: () => number, releasing: () => boolean, retire: (registration: Registration) => void);
    /** The claim with compositor id `id`, or undefined when it is not this context's. */
    byId(id: number): ClaimEntry | undefined;
    /** Whether `registration` is still registered: a binding removed mid-drag is not. */
    has(registration: Registration): boolean;
    /** Returns plain data about each registration and its rank among bindings of the same trigger. */
    describe(): InputBindingInfo[];
    /**
     * Follows every binding in `source`, including later additions, and returns a function that removes them all.
     * If the compositor refuses a claim, no binding from this source remains registered.
     */
    follow(source: BindingCollection, tier: number, owns: (client: BindingClient | null) => boolean, layout: InputLayout | null, device: string): () => void;
    /** The registrations that can take an input on `claim`, best first. */
    candidates(claim: ClaimEntry, client: BindingClient | null): Registration[];
    /** Gives up every claim. Set `release` to false when the seat is already gone. */
    drop(release: boolean): void;
}

// Tracks the compositor claims held for a context's bindings and the registrations that use them. Bindings for the same
// trigger and device share a claim; the last registration releases it. Candidates are returned in precedence order.

import type { Binding, BindingCollection } from "gnoblin/bindings/binding";
import { claimOf } from "gnoblin/bindings/claim";
import type { TriggerClaim } from "gnoblin/bindings/claim";
import type { BindingClient } from "gnoblin/bindings/types";
import { errorCodeOfRefusal } from "gnoblin/errors";
import type { InputHost, InputLayout, InputWorld } from "gnoblin/input/host";
import { byPrecedence, specKeyOf } from "gnoblin/input/registration";
import type { ClaimEntry, InputBindingInfo, Registration } from "gnoblin/input/registration";

/** Tracks the claims and registrations held by one input context. */
export class ClaimTable {
    readonly #host: InputHost;
    readonly #world: InputWorld;
    readonly #name: string;
    readonly #seat: () => number;
    readonly #releasing: () => boolean;
    /** Called before a registration's claim is given up, so a drag its binding runs can be cancelled first. */
    readonly #retire: (registration: Registration) => void;
    readonly #claims = new Map<string, ClaimEntry>();
    readonly #claimIds = new Map<number, ClaimEntry>();
    readonly #registrations = new Set<Registration>();
    #order = 0;

    constructor(
        host: InputHost,
        world: InputWorld,
        name: string,
        seat: () => number,
        releasing: () => boolean,
        retire: (registration: Registration) => void,
    ) {
        this.#host = host;
        this.#world = world;
        this.#name = name;
        this.#seat = seat;
        this.#releasing = releasing;
        this.#retire = retire;
    }

    /** The claim with compositor id `id`, or undefined when it is not this context's. */
    byId(id: number): ClaimEntry | undefined {
        return this.#claimIds.get(id);
    }

    /** Whether `registration` is still registered: a binding removed mid-drag is not. */
    has(registration: Registration): boolean {
        return this.#registrations.has(registration);
    }

    /** Returns plain data about each registration and its rank among bindings of the same trigger. */
    describe(): InputBindingInfo[] {
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
                    context: this.#name,
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

    /**
     * Follows every binding in `source`, including later additions, and returns a function that removes them all.
     * If the compositor refuses a claim, no binding from this source remains registered.
     */
    follow(
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
        this.#retire(registration);
        const { claim } = registration;
        claim.refs -= 1;
        if (claim.refs === 0) {
            this.#claims.delete(`${claim.specKey}\n${claim.device}`);
            this.#claimIds.delete(claim.id);
            if (this.#releasing()) {
                this.#host.unclaim(this.#seat(), claim.id);
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
                ? this.#host.claim(this.#seat(), spec.text, spec.grab, device)
                : this.#host.claimGesture(this.#seat(), spec.gesture, spec.fingers, device);
        if (typeof result === "string") {
            throw this.#world.error(
                errorCodeOfRefusal(result),
                `InputContext "${this.#name}"`,
                `the compositor refused the claim for ${specKey} (${result})`,
            );
        }
        const claim: ClaimEntry = { id: result, specKey, device, spec, pointer, refs: 1 };
        this.#claims.set(key, claim);
        this.#claimIds.set(claim.id, claim);
        return claim;
    }

    /** The registrations that can take an input on `claim`, best first. */
    candidates(claim: ClaimEntry, client: BindingClient | null): Registration[] {
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

    /** Gives up every claim. Set `release` to false when the seat is already gone. */
    drop(release: boolean): void {
        for (const claim of [...this.#claims.values()]) {
            this.#claims.delete(`${claim.specKey}\n${claim.device}`);
            this.#claimIds.delete(claim.id);
            if (release) {
                this.#host.unclaim(this.#seat(), claim.id);
            }
        }
        this.#registrations.clear();
    }
}

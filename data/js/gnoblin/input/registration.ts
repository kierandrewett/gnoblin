// What a binding becomes once a context has accepted it: a registration of its trigger against a claim on the seat.
// A claim is shared by every registration of one chord (or gesture) for one device and is released with the last of
// them. Types and the ordering of registrations only; `ClaimTable` keeps them.

import type { Binding } from "gnoblin/bindings/binding";
import type { TriggerClaim } from "gnoblin/bindings/claim";
import type { ParsedTrigger } from "gnoblin/bindings/trigger_types";
import type { BindingClient } from "gnoblin/bindings/types";
import type { InputLayout } from "gnoblin/input/host";

/** Where a binding came from, which is also how it ranks: a device's bindings run before a layout's, a layout's before the context's. */
export const TIER_CONTEXT = 0;
export const TIER_LAYOUT = 1;
export const TIER_DEVICE = 2;

/** One claim the context holds on its seat, and how many registrations stand on it. */
export interface ClaimEntry {
    readonly id: number;
    readonly specKey: string;
    readonly device: string;
    readonly spec: TriggerClaim;
    readonly pointer: boolean;
    refs: number;
}

/** One binding of a followed collection, registered against the claim of its trigger. */
export interface Registration {
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
export function byPrecedence(a: Registration, b: Registration): number {
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

/** The key that tells two triggers that need one claim apart: a chord by its spelling, a gesture by kind and fingers. */
export function specKeyOf(spec: TriggerClaim): string {
    return spec.kind === "chord" ? `chord:${spec.text}` : `gesture:${spec.gesture}${spec.fingers}`;
}

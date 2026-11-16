// Records which bindings use each compositor claim and the order in which matching bindings are considered.

import type { Binding } from "gnoblin-core/bindings/binding";
import type { TriggerClaim } from "gnoblin-core/bindings/claim";
import type { ParsedTrigger } from "gnoblin-core/bindings/trigger_types";
import type { BindingClient } from "gnoblin-core/bindings/types";
import type { InputLayout } from "gnoblin-core/input/host";

/** Precedence tier of a binding added to the context itself. The lowest tier: layout and device bindings run first. */
export const TIER_CONTEXT = 0;
/** Precedence tier of a binding that comes from a layout. It runs before the context's bindings. */
export const TIER_LAYOUT = 1;
/** Precedence tier of a binding that comes from a device. The highest tier: it runs before layout bindings. */
export const TIER_DEVICE = 2;

/** One claim the context holds on its seat, and how many registrations stand on it. */
export interface ClaimEntry {
    /** Compositor id for this claim. */
    readonly id: number;
    /** Key identifying this claim's trigger. */
    readonly specKey: string;
    /** Device name for the claim, or an empty string for all devices. */
    readonly device: string;
    /** Parsed chord or gesture requested from the compositor. */
    readonly spec: TriggerClaim;
    /** Whether the claim handles pointer input. */
    readonly pointer: boolean;
    /** Number of registrations using this claim. */
    refs: number;
}

/** One binding of a followed collection, registered against the claim of its trigger. */
export interface Registration {
    /** Binding represented by this registration. */
    readonly binding: Binding;
    /** Source tier used to order this binding against other candidates. */
    readonly tier: number;
    /** Registration order, used to break precedence ties. */
    readonly order: number;
    /** Whether this binding applies to the target client. */
    readonly owns: (client: BindingClient | null) => boolean;
    /** Layout that supplied this binding, or null when it did not come from a layout. */
    readonly layout: InputLayout | null;
    /** Device name for a device-specific registration, or an empty string for all devices. */
    readonly device: string;
    /** Parsed form of the binding's trigger. */
    readonly parsed: ParsedTrigger;
    /** Key identifying this registration's trigger. */
    readonly specKey: string;
    /** Compositor claim used by this registration. */
    readonly claim: ClaimEntry;
}

/** Which of two registrations of one trigger runs first: the higher tier, then the higher priority, then the older. */
export function byPrecedence(a: Registration, b: Registration): number {
    return b.tier - a.tier || b.binding.priority - a.binding.priority || a.order - b.order;
}

/** What `InputContext.describeBindings` says of one registered binding. */
export interface InputBindingInfo {
    /** Name of the context containing the binding. */
    readonly context: string;
    /** Binding id. */
    readonly id: string;
    /** Trigger text accepted by the binding. */
    readonly trigger: string;
    /** Parsed trigger category. */
    readonly kind: string;
    /** Source of the binding. */
    readonly owner: "context" | "layout" | "device";
    /** Layout class name, or null when the binding is not from a layout. */
    readonly layout: string | null;
    /** Device name, or null when the binding applies to every device. */
    readonly device: string | null;
    /** Binding priority. */
    readonly priority: number;
    /** Whether the binding is enabled. */
    readonly enabled: boolean;
    /** 1 is the binding a press of the trigger reaches first; a binding that declines hands the press to rank 2. */
    readonly rank: number;
}

/** Returns the key shared by triggers that use the same chord or gesture kind and finger count. */
export function specKeyOf(spec: TriggerClaim): string {
    return spec.kind === "chord" ? `chord:${spec.text}` : `gesture:${spec.gesture}${spec.fingers}`;
}

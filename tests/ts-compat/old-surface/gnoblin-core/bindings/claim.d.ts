import type { ParsedTrigger } from "gnoblin-core/bindings/trigger_types";
/** What a trigger asks the compositor to claim on the seat. */
export type TriggerClaim = {
    readonly kind: "chord";
    readonly text: string;
    readonly grab: boolean;
} | {
    readonly kind: "gesture";
    readonly gesture: "swipe" | "pinch";
    readonly fingers: number;
};
/**
 * The seat claim a trigger needs, in the compositor's own chord spelling (`<Shift><Super>1`), or `null` when
 * the compositor has no claim for it (a wheel). A drag and a click of one button share a claim: the chord's
 * press and release both reach the shell, and the shell tells a drag from a click by how far the pointer went.
 */
export declare function claimOf(trigger: ParsedTrigger): TriggerClaim | null;

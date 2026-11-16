// What a trigger claims on a seat: the chord or touchpad gesture the compositor must hand to the shell, so that
// `gnoblin-core/input` can claim it once however many bindings share it.

import { BUTTON_NUMBER } from "gnoblin-core/bindings/trigger";
import type { ParsedTrigger, TriggerModifier } from "gnoblin-core/bindings/trigger_types";

/** What a trigger asks the compositor to claim on the seat. */
export type TriggerClaim =
    | { readonly kind: "chord"; readonly text: string; readonly grab: boolean }
    | { readonly kind: "gesture"; readonly gesture: "swipe" | "pinch"; readonly fingers: number };

const FULL_MODIFIER: Readonly<Record<TriggerModifier, string>> = {
    Shift: "Shift",
    Ctrl: "Control",
    Alt: "Alt",
    Meta: "Meta",
    Super: "Super",
    Hyper: "Hyper",
};

/**
 * The seat claim a trigger needs, in the compositor's own chord spelling (`<Shift><Super>1`), or `null` when
 * the compositor has no claim for it (a wheel). A drag and a click of one button share a claim: the chord's
 * press and release both reach the shell, and the shell tells a drag from a click by how far the pointer went.
 */
export function claimOf(trigger: ParsedTrigger): TriggerClaim | null {
    const modifiers = trigger.modifiers.map((modifier) => `<${FULL_MODIFIER[modifier]}>`).join("");
    switch (trigger.kind) {
        case "key":
            return { kind: "chord", text: `${modifiers}${trigger.key}`, grab: false };
        case "pointer":
            return { kind: "chord", text: `${modifiers}Button${BUTTON_NUMBER[trigger.button]}`, grab: true };
        case "gesture":
            return { kind: "gesture", gesture: trigger.gesture, fingers: trigger.fingers };
        case "wheel":
        case "controller":
            return null;
    }
}

// The shapes a parsed trigger takes: the four kinds (key, pointer, wheel, gesture) and the names their parts use.
// Types only; the grammar that produces them is `trigger`.

/** The modifiers a trigger may name, in the order a canonical trigger writes them. */
export type TriggerModifier = "Shift" | "Ctrl" | "Alt" | "Meta" | "Super" | "Hyper";

/** A mouse button a trigger can name. */
export type TriggerButton = "MouseLeft" | "MouseMiddle" | "MouseRight";

/** A direction a gesture trigger can name: swipes use the four sides, pinches use `in` and `out`. */
export type TriggerDirection = "left" | "right" | "up" | "down" | "in" | "out";

/** Every spelling a modifier is accepted as, for the declaration file's `Trigger` type. */
export type TriggerModifierName = TriggerModifier | "Control" | "Primary" | "Mod1" | "Logo";

type ModifierPrefix =
    | ""
    | `${TriggerModifierName}+`
    | `${TriggerModifierName}+${TriggerModifierName}+`
    | `${TriggerModifierName}+${TriggerModifierName}+${TriggerModifierName}+`;

type FingerCount = 2 | 3 | 4 | 5;

/** `Swipe3`, `Swipe3Left`, `Pinch4`, `Pinch4In`: a touchpad gesture with its finger count and optional direction. */
export type GestureTrigger =
    | `Swipe${FingerCount}`
    | `Swipe${FingerCount}${"Left" | "Right" | "Up" | "Down"}`
    | `Pinch${FingerCount}`
    | `Pinch${FingerCount}${"In" | "Out"}`;

/** A mouse button, optionally held with modifiers and optionally followed by `Drag` or `Click`. */
export type PointerTrigger =
    `${ModifierPrefix}${TriggerButton}` | `${ModifierPrefix}${TriggerButton}+${"Drag" | "Click"}`;

/** A wheel input, optionally held with modifiers and optionally with a direction. */
export type WheelTrigger = `${ModifierPrefix}Wheel${"" | "Up" | "Down" | "Left" | "Right"}`;

/** A key trigger is `modifiers+keysym`; the key names are the compositor's, so the type cannot list them. */
export type KeyTrigger = `${ModifierPrefix}${string}`;

/** Any trigger the grammar accepts: a key, a mouse button, a wheel or a gesture. */
export type Trigger = GestureTrigger | PointerTrigger | WheelTrigger | KeyTrigger;

interface TriggerBase {
    /** The canonical spelling: modifiers in `Shift, Ctrl, Alt, Meta, Super, Hyper` order, canonical key name. */
    readonly text: string;
    readonly modifiers: readonly TriggerModifier[];
}

/** A key, or a lone `Super` tap (`modifiers` empty, `key` `Super`). */
export interface KeyTriggerParts extends TriggerBase {
    readonly kind: "key";
    readonly key: string;
}

/** A mouse button; `action` is `drag` when the trigger ends in `Drag`, else `click`. */
export interface PointerTriggerParts extends TriggerBase {
    readonly kind: "pointer";
    readonly button: TriggerButton;
    readonly action: "click" | "drag";
}

/** A parsed wheel trigger. `direction` is null when the trigger names none. */
export interface WheelTriggerParts extends TriggerBase {
    readonly kind: "wheel";
    readonly direction: "up" | "down" | "left" | "right" | null;
}

/** A parsed touchpad gesture trigger: its finger count and, when named, its direction. */
export interface GestureTriggerParts extends TriggerBase {
    readonly kind: "gesture";
    readonly gesture: "swipe" | "pinch";
    readonly fingers: number;
    readonly direction: TriggerDirection | null;
}

/** An XR controller button a trigger can name. Grip is the squeeze, Primary is A or X, Secondary is B or Y. */
export type ControllerButton = "Trigger" | "Grip" | "Thumbstick" | "Primary" | "Secondary";

/**
 * A grab with an XR controller: `Grip+Drag`, `Right.Trigger+Grip+Drag`. Only the XR module's bindings read one
 * (`gnoblin/xr`): no seat claims it, and a context ignores a binding that has one.
 */
export interface ControllerTriggerParts extends TriggerBase {
    readonly kind: "controller";
    /** The hand every button must be on, or null for either. */
    readonly hand: "Left" | "Right" | null;
    /** The buttons that must be down together, in the order `Trigger, Grip, Thumbstick, Primary, Secondary`. */
    readonly buttons: readonly ControllerButton[];
    readonly action: "drag";
}

/** A trigger after parsing. The `kind` field says which kind of input it names. */
export type ParsedTrigger =
    KeyTriggerParts | PointerTriggerParts | WheelTriggerParts | GestureTriggerParts | ControllerTriggerParts;

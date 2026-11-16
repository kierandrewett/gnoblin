// The trigger grammar: a trigger is the string a user writes for an input (`Super+Return`, `Super+MouseLeft+Drag`,
// `Swipe3Left`), and `parseTrigger` is the one place its grammar lives. It turns the spelling into a canonical one,
// so two spellings of one trigger are one trigger (`Ctrl+Alt+Delete` and `control+alt+delete`), and it reports a bad
// spelling with the token that is wrong. The matcher, the hit test and the compositor claims are in `gnoblin-core/input`.
//
// Grammar (case-insensitive except key names, which are keysym names):
//   trigger  := key | pointer | wheel | gesture
//   key      := (modifier "+")* keyname        `Super+Shift+1`, `XF86AudioMute`; a lone `Super` is a tap
//   pointer  := (modifier "+")* button ["+" ("Drag" | "Click")]    `Super+MouseLeft+Drag`, `MouseMiddle`
//   wheel    := (modifier "+")* "Wheel" ["Up" | "Down" | "Left" | "Right"]
//   gesture  := ("Swipe" | "Pinch") fingers(2-5) [direction]       `Swipe3Left`, `Pinch4In`
//   modifier := Shift | Ctrl (Control, Primary) | Alt (Mod1) | Meta | Super (Logo) | Hyper
// A key is a keysym name, never a character that depends on the layout: `Super+1` and `Super+Shift+1` are two
// triggers, and `Super+!` is refused (write `Super+Shift+1` or `Super+exclam`). A literal plus is `plus`.

import type {
    GestureTriggerParts,
    ParsedTrigger,
    TriggerButton,
    TriggerDirection,
    TriggerModifier,
    WheelTriggerParts,
} from "gnoblin-core/bindings/trigger_types";
import type { ErrorCode } from "gnoblin-core/errors";

/** A trigger spelling the grammar refuses. `token` is the part that is wrong. */
export class TriggerError extends TypeError {
    /** A spelling the grammar refuses is a bad argument. */
    readonly code: ErrorCode = "invalid_argument";
    /** The part of the trigger spelling that caused the error. */
    readonly token: string;
    /** The complete trigger spelling that was rejected. */
    readonly trigger: string;

    constructor(what: string, trigger: string, token: string, reason: string) {
        super(`${what}: ${reason} (token "${token}" in "${trigger}")`);
        this.token = token;
        this.trigger = trigger;
    }
}

const MODIFIER_ORDER: readonly TriggerModifier[] = ["Shift", "Ctrl", "Alt", "Meta", "Super", "Hyper"];

const MODIFIERS: Readonly<Record<string, TriggerModifier>> = {
    shift: "Shift",
    ctrl: "Ctrl",
    control: "Ctrl",
    primary: "Ctrl",
    alt: "Alt",
    mod1: "Alt",
    meta: "Meta",
    super: "Super",
    logo: "Super",
    hyper: "Hyper",
};

/** X11 masks whose key depends on the keymap: the compositor refuses them, and so does the grammar. */
const REFUSED_MASKS: Readonly<Record<string, true>> = { mod2: true, mod3: true, mod4: true, mod5: true };

const BUTTONS: Readonly<Record<string, TriggerButton>> = {
    mouseleft: "MouseLeft",
    mousemiddle: "MouseMiddle",
    mouseright: "MouseRight",
};

/** The kernel-side number of each button a trigger can name. */
export const BUTTON_NUMBER: Readonly<Record<TriggerButton, number>> = { MouseLeft: 1, MouseMiddle: 2, MouseRight: 3 };

const WHEELS: Readonly<Record<string, WheelTriggerParts["direction"]>> = {
    wheel: null,
    wheelup: "up",
    wheeldown: "down",
    wheelleft: "left",
    wheelright: "right",
};

/** Names the grammar writes one way, so two spellings are not two bindings that shadow each other. */
const KEY_NAMES: Readonly<Record<string, string>> = {
    return: "Return",
    enter: "Return",
    escape: "Escape",
    esc: "Escape",
    tab: "Tab",
    space: "space",
    backspace: "BackSpace",
    delete: "Delete",
    del: "Delete",
    insert: "Insert",
    ins: "Insert",
    home: "Home",
    end: "End",
    pageup: "Page_Up",
    page_up: "Page_Up",
    pagedown: "Page_Down",
    page_down: "Page_Down",
    left: "Left",
    right: "Right",
    up: "Up",
    down: "Down",
    print: "Print",
    menu: "Menu",
    plus: "plus",
    minus: "minus",
};

const GESTURE = /^(swipe|pinch)([0-9]+)(left|right|up|down|in|out)?$/i;

const FUNCTION_KEY = /^f([1-9]|[12][0-9]|3[0-5])$/i;

const KEY_NAME = /^[A-Za-z0-9_]+$/;

/** A table entry that is the table's own, so a key named `constructor` is a key and not `Object`. */
function lookup<T>(table: Readonly<Record<string, T>>, key: string): T | undefined {
    return Object.hasOwn(table, key) ? table[key] : undefined;
}

function fail(what: string, trigger: string, token: string, reason: string): never {
    throw new TriggerError(what, trigger, token, reason);
}

function canonicalKey(token: string): string {
    const known = lookup(KEY_NAMES, token.toLowerCase());
    if (known !== undefined) {
        return known;
    }
    if (FUNCTION_KEY.test(token)) {
        return `F${token.slice(1)}`;
    }
    // A letter is the key, not a shifted letter: `Super+Q` and `Super+q` are one trigger, and the shift is
    // written as `Shift`. Longer names keep their case, because keysym names are case-sensitive.
    return token.length === 1 ? token.toLowerCase() : token;
}

function parseGesture(what: string, trigger: string, token: string): GestureTriggerParts {
    const match = GESTURE.exec(token);
    if (!match) {
        return fail(what, trigger, token, "is not a gesture; expected Swipe or Pinch with a finger count");
    }
    const gesture = match[1].toLowerCase() as "swipe" | "pinch";
    const fingers = Number(match[2]);
    if (fingers < 2 || fingers > 5) {
        return fail(what, trigger, token, `a gesture has 2 to 5 fingers, not ${fingers}`);
    }
    const direction = (match[3]?.toLowerCase() ?? null) as TriggerDirection | null;
    const allowed = gesture === "swipe" ? ["left", "right", "up", "down"] : ["in", "out"];
    if (direction !== null && !allowed.includes(direction)) {
        return fail(what, trigger, token, `a ${gesture} goes ${allowed.join(", ")}, not ${direction}`);
    }
    const name = gesture === "swipe" ? "Swipe" : "Pinch";
    const suffix = direction === null ? "" : direction[0].toUpperCase() + direction.slice(1);
    return { kind: "gesture", gesture, fingers, direction, modifiers: [], text: `${name}${fingers}${suffix}` };
}

/**
 * Parses a trigger into its parts and canonical spelling, or throws a `TriggerError` naming the token that
 * is wrong. `what` names the thing that holds the trigger, for the message.
 */
export function parseTrigger(trigger: unknown, what = "trigger"): ParsedTrigger {
    if (typeof trigger !== "string" || trigger === "") {
        throw new TypeError(`${what} must be a trigger such as "Super+Return", got ${String(trigger)}`);
    }
    const parts = trigger.split("+");
    for (let i = 0; i < parts.length; i++) {
        if (parts[i] === "") {
            const after = i === 0 ? "the start" : `"${parts[i - 1]}"`;
            return fail(what, trigger, "", `an empty part after ${after}; write a literal plus as "plus"`);
        }
        if (/\s/.test(parts[i])) {
            return fail(what, trigger, parts[i], "has whitespace; join the parts with plus alone");
        }
    }
    const gestureAt = parts.findIndex((part) => /^(swipe|pinch)/i.test(part));
    if (gestureAt >= 0) {
        if (parts.length > 1) {
            return fail(
                what,
                trigger,
                parts[gestureAt === 0 ? 1 : gestureAt],
                "a gesture takes no modifiers or other parts",
            );
        }
        return parseGesture(what, trigger, parts[0]);
    }

    const modifiers = new Set<TriggerModifier>();
    let button: TriggerButton | null = null;
    let wheel: { direction: WheelTriggerParts["direction"] } | null = null;
    let action: "click" | "drag" | null = null;
    let key: string | null = null;
    let last = "";
    for (const token of parts) {
        const lower = token.toLowerCase();
        const modifier = lookup(MODIFIERS, lower);
        if (lookup(REFUSED_MASKS, lower)) {
            fail(what, trigger, token, "names a keymap-dependent mask; write Super, Alt, Meta or Hyper");
        }
        if (modifier !== undefined) {
            if (button !== null || wheel !== null || key !== null || action !== null) {
                fail(what, trigger, token, "is a modifier after the key; modifiers come first");
            }
            if (modifiers.has(modifier)) {
                fail(what, trigger, token, `repeats the modifier ${modifier}`);
            }
            modifiers.add(modifier);
        } else if (lookup(BUTTONS, lower) !== undefined) {
            if (button !== null || wheel !== null || key !== null) {
                fail(what, trigger, token, "is a second input; a trigger has one key, button or wheel");
            }
            button = lookup(BUTTONS, lower) ?? null;
        } else if (Object.hasOwn(WHEELS, lower)) {
            if (button !== null || wheel !== null || key !== null) {
                fail(what, trigger, token, "is a second input; a trigger has one key, button or wheel");
            }
            wheel = { direction: WHEELS[lower] };
        } else if (lower === "drag" || lower === "click") {
            if (button === null) {
                fail(what, trigger, token, `needs a mouse button before it, such as Super+MouseLeft+${token}`);
            }
            if (action !== null) {
                fail(what, trigger, token, "is a second action");
            }
            action = lower;
        } else {
            if (button !== null || wheel !== null || action !== null) {
                fail(what, trigger, token, "is a key after a mouse button or wheel; a trigger has one input");
            }
            if (key !== null) {
                fail(what, trigger, token, `is a second key after "${key}"; a trigger has one key`);
            }
            if (token.length === 1 && !/[A-Za-z0-9]/.test(token)) {
                fail(
                    what,
                    trigger,
                    token,
                    "is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key",
                );
            }
            if (!KEY_NAME.test(token)) {
                fail(what, trigger, token, "is not a key name; a key name is letters, digits and underscores");
            }
            key = canonicalKey(token);
        }
        last = token;
    }

    const ordered = MODIFIER_ORDER.filter((modifier) => modifiers.has(modifier));
    const prefix = ordered.map((modifier) => `${modifier}+`).join("");
    if (button !== null) {
        // A bare button is a click, and so is `+Click`: one canonical spelling for the same trigger.
        const click = action !== "drag";
        const canonical = `${prefix}${button}${click ? "" : "+Drag"}`;
        return { kind: "pointer", button, action: click ? "click" : "drag", modifiers: ordered, text: canonical };
    }
    if (wheel !== null) {
        const name =
            wheel.direction === null ? "Wheel" : `Wheel${wheel.direction[0].toUpperCase()}${wheel.direction.slice(1)}`;
        return { kind: "wheel", direction: wheel.direction, modifiers: ordered, text: `${prefix}${name}` };
    }
    if (key === null) {
        // Only modifiers were written. A lone Super is a tap; anything else has no key to wait for.
        if (ordered.length === 1 && ordered[0] === "Super") {
            return { kind: "key", key: "Super", modifiers: [], text: "Super" };
        }
        return fail(
            what,
            trigger,
            last,
            ordered.length === 1
                ? "a lone modifier is only a trigger for Super (a tap)"
                : "ends with a modifier and has no key",
        );
    }
    return { kind: "key", key, modifiers: ordered, text: `${prefix}${key}` };
}

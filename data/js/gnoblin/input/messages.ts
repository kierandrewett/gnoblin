// What the compositor sends the input module: a key chord that changed state, the steps of a pointer chord's drag and
// the steps of a touchpad gesture. Types only.

/** A claimed chord that changed state, as the compositor sends it. */
export interface InputChordMessage {
    readonly kind: "chord";
    readonly claim: number;
    readonly chord: string;
    readonly state: string;
}

/** A claimed pointer chord's drag that advanced: the compositor's `pointer_chord` signal. */
export interface InputPointerMessage {
    readonly kind: "pointer";
    readonly claim: number;
    /** `start`, `update`, `end` or `cancel`. */
    readonly phase: string;
    /** The pointer in global logical coordinates. */
    readonly x: number;
    readonly y: number;
    /** The pointer's movement since `start`. */
    readonly dx: number;
    readonly dy: number;
    /** The chord's button number: 1 left, 2 middle, 3 right, 8 back, 9 forward. */
    readonly button: number;
    readonly modifiers: string;
}

/** A claimed gesture that advanced. `dx`, `dy` are touchpad units since the begin. */
export interface InputGestureMessage {
    readonly kind: "gesture";
    readonly claim: number;
    readonly gesture: string;
    readonly fingers: number;
    readonly phase: string;
    readonly dx: number;
    readonly dy: number;
    readonly scale: number;
    readonly rotation: number;
}

/** Any message the compositor sends to the input module: a key chord, a pointer event or a gesture. */
export type InputMessage = InputChordMessage | InputPointerMessage | InputGestureMessage;

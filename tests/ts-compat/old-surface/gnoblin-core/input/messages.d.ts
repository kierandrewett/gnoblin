/** A claimed chord that changed state, as the compositor sends it. */
export interface InputChordMessage {
    /** Discriminates chord messages from pointer and gesture messages. */
    readonly kind: "chord";
    /** Id of the compositor claim that produced this message. */
    readonly claim: number;
    /** Key chord that changed state. */
    readonly chord: string;
    /** New chord state, such as `press` or `release`. */
    readonly state: string;
}
/** A claimed pointer chord's drag that advanced: the compositor's `pointer_chord` signal. */
export interface InputPointerMessage {
    /** Discriminates pointer messages from chord and gesture messages. */
    readonly kind: "pointer";
    /** Id of the compositor claim that produced this message. */
    readonly claim: number;
    /** `start`, `update`, `end` or `cancel`. */
    readonly phase: string;
    /** The pointer's horizontal position in global logical pixels. */
    readonly x: number;
    /** The pointer's vertical position in global logical pixels. */
    readonly y: number;
    /** The pointer's movement since `start`, in logical pixels. */
    readonly dx: number;
    /** Vertical pointer movement since `start`, in logical pixels. */
    readonly dy: number;
    /** The chord's button number: 1 left, 2 middle, 3 right, 8 back, 9 forward. */
    readonly button: number;
    /** Modifier keys held during the pointer event. */
    readonly modifiers: string;
}
/** A claimed gesture that advanced. `dx`, `dy` are touchpad units since the begin. */
export interface InputGestureMessage {
    /** Discriminates gesture messages from chord and pointer messages. */
    readonly kind: "gesture";
    /** Id of the compositor claim that produced this message. */
    readonly claim: number;
    /** Gesture type, such as `swipe` or `pinch`. */
    readonly gesture: string;
    /** Number of fingers involved in the gesture. */
    readonly fingers: number;
    /** Gesture lifecycle step: `begin`, `update`, `end` or `cancel`. */
    readonly phase: string;
    /** Horizontal centroid movement since the gesture began, in touchpad units. */
    readonly dx: number;
    /** Vertical centroid movement since the gesture began, in touchpad units. */
    readonly dy: number;
    /** Pinch scale relative to its starting size. */
    readonly scale: number;
    /** Rotation reported for this gesture. */
    readonly rotation: number;
}
/** Any message the compositor sends to the input module: a key chord, a pointer event or a gesture. */
export type InputMessage = InputChordMessage | InputPointerMessage | InputGestureMessage;

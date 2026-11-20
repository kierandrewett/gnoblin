import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import type { BuiltinBinding } from "gnoblin-std/emitted/layouts";
import type { DragOutcome, DragTile, Split } from "gnoblin-std/emitted/records";
/** The number the library knows `client` by in a step: its node id, or one given on first use for an object that has none. */
export declare function idOf(client: object): number;
/**
 * What a step reads of a client beyond `BindingClient`. `tiled` is the flag a layout sets when it places the client, and it stays
 * set when a script floats the client afterwards, so `floating` and `parentWindow` (a dialog is never tiled) are read as well.
 */
export interface StepClient extends BindingClient {
    /** True when the layout places the client in a tile. */
    readonly tiled?: boolean;
    /** True when the client is floating instead of occupying a tile. */
    readonly floating?: boolean;
    /** Parent window when the client is a dialog. */
    readonly parentWindow?: unknown;
    /** Whether the client is visible to the user. */
    readonly visible?: boolean;
    /** Whether the client is minimised. */
    readonly minimised?: boolean;
    /** The workspace that holds the client's tile, when it is on one. */
    readonly workspace?: unknown;
}
/** What a step is told of a client: the flags of `DragKind.CLIENT_*` its script-side state sets. */
export declare function clientFlags(client: StepClient): number;
/** The tile a swap step is told of `client`. */
export declare function tileOf(client: StepClient): DragTile;
/** What a step may add to what it knows of the pressed client. */
export interface StepExtras {
    /** Where the press was: on every step but the start. Default the pointer. */
    origin?: Point;
    /** The client's rectangle when the drag started: an update's and a cancel's. Default its rectangle now. */
    start?: Rect;
    /** A resize's update: the `SideFlags` the start's answer gave. */
    sides?: number;
    /** A split resize: the divider the layout found for the press, with the ratios it had then. */
    split?: Split;
    /** A swap's update and end: the tiles of the dragged client's workspace, in its order. */
    tiles?: DragTile[];
}
/**
 * Gives the library's `binding` one step of a drag: `phase` is `DragKind.START`, `UPDATE`, `END` or `CANCEL`, `event` what the
 * engine's matcher passed the hook, `client` the window the press landed on, as the script holds it. The answer says what to write
 * to the script's own objects.
 */
export declare function takeStep(binding: BuiltinBinding, phase: number, event: DragEvent, client: StepClient | undefined, extras?: StepExtras): DragOutcome;
/** Whether the answer of a step sets `flag` (`OutcomeFlags.*`). */
export declare function has(outcome: DragOutcome, flag: number): boolean;
/** A copy of the rectangle an answer gives, for a script to keep. */
export declare function rectOf(rect: Rect): Rect;

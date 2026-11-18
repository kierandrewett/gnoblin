// The stock drag bindings' hold on the library's drag rules (`libgnoblin-std`'s `drag_step`).
//
// A stock binding of this package (move, resize, swap, split resize) is a `PointerDragBinding`, so the engine's matcher
// (`gnoblin-core/input`: the chord, the threshold, the tiers, the layout's scope, the competition and the decline) keeps
// deciding which binding has a drag and the hooks a script overrides keep their names and their state objects. What the
// binding DOES, which window may be dragged, where it goes, which side a resize takes hold of, where a drop snaps, which tile a
// release trades places with, how far a divider moves, is the library's, once, for every language: a hook builds what the step
// knows of the window from the script's own objects, gives it to the library, and writes the answer back to them. The library
// keeps nothing of a drag between its steps; what a step needs of the drag's start is in the state object `onStart` returned.
//
// Nothing here decides anything. The spelling of the objects (a client's `maximised`, its `frameInset`, its `minSize`) is the
// script's; the library takes numbers and flags.

import { NO_INSET } from "gnoblin-core/inset";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import { ClientFlags, ConstraintKind, DragKind } from "gnoblin-std/generated/constants";
import type { BuiltinBinding } from "gnoblin-std/generated/layouts";
import type {
    DragEvent as NativeDragEvent,
    ClientState,
    DragOutcome,
    DragTile,
    Split,
} from "gnoblin-std/generated/records";
import { dragConstraintMode } from "gnoblin-std/bindings/drag";

const NO_RECT: Rect = Object.freeze({ x: 0, y: 0, width: 0, height: 0 });

const known = new WeakMap<object, number>();
// Above any node id: a script's object that has none is told apart from one that has.
let issued = 2 ** 40;

/** The number the library knows `client` by in a step: its node id, or one given on first use for an object that has none. */
export function idOf(client: object): number {
    const id = "id" in client ? client.id : undefined;
    if (typeof id === "number" && Number.isSafeInteger(id) && id > 0) {
        return id;
    }
    let found = known.get(client);
    if (found === undefined) {
        issued += 1;
        found = issued;
        known.set(client, found);
    }
    return found;
}

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
export function clientFlags(client: StepClient): number {
    let flags = 0;
    const set = (on: boolean, flag: number): void => {
        if (on) {
            flags |= flag;
        }
    };
    set(client.fullscreen === true, DragKind.CLIENT_FULLSCREEN);
    set(client.maximised === true, DragKind.CLIENT_MAXIMISED);
    set(client.tiled === true, DragKind.CLIENT_TILED);
    set(client.floating === true, DragKind.CLIENT_FLOATING);
    set(client.parentWindow != null, DragKind.CLIENT_DIALOG);
    set(client.minimised === true, DragKind.CLIENT_MINIMISED);
    set(client.visible === false, DragKind.CLIENT_HIDDEN);
    set(client.workspace != null, DragKind.CLIENT_ON_WORKSPACE);
    return flags;
}

/** The room a client's frame takes and the sizes it allows itself, as the library's client state. */
function limitsOf(client: BindingClient): ClientState {
    const inset = client.frameInset ?? NO_INSET;
    const { minSize, maxSize } = client;
    return {
        flags: (minSize === undefined ? 0 : ClientFlags.HAS_MIN) | (maxSize === undefined ? 0 : ClientFlags.HAS_MAX),
        insetTop: inset.top,
        insetRight: inset.right,
        insetBottom: inset.bottom,
        insetLeft: inset.left,
        minWidth: minSize?.width ?? 0,
        minHeight: minSize?.height ?? 0,
        maxWidth: maxSize?.width ?? 0,
        maxHeight: maxSize?.height ?? 0,
        floating: NO_RECT,
        snapZone: 0,
        snapFloating: NO_RECT,
    };
}

/** The tile a swap step is told of `client`. */
export function tileOf(client: StepClient): DragTile {
    return { id: idOf(client), flags: clientFlags(client), rect: client.bounds };
}

/** The event of a step, as the library tells it to a binding of the shell's own. */
function eventOf(
    phase: number,
    event: DragEvent,
    origin: Point | undefined,
    pressed: object | undefined,
): NativeDragEvent {
    const at = event.position ?? { x: 0, y: 0 };
    const local = event.localPosition;
    const area = event.workArea;
    let flags = 0;
    if (local !== undefined) {
        flags |= DragKind.HAS_LOCAL;
    }
    if (area !== undefined) {
        flags |= DragKind.HAS_WORK_AREA;
    }
    // A drop with no position is not on an edge.
    if (event.edgeTiling === true && event.position !== undefined) {
        flags |= DragKind.EDGE_TILING;
    }
    // The seat's rule is read, and refused when it is not one, only where a drag is held to the work area.
    const constraint =
        phase === DragKind.UPDATE && area !== undefined && dragConstraintMode(event.dragConstraint) === "minimum_strip"
            ? ConstraintKind.MINIMUM_STRIP
            : ConstraintKind.WORK_AREA;
    return {
        phase,
        flags,
        constraint,
        button: event.button ?? 0,
        client: pressed === undefined ? 0 : idOf(pressed),
        x: at.x,
        y: at.y,
        originX: (origin ?? at).x,
        originY: (origin ?? at).y,
        localX: local?.x ?? 0,
        localY: local?.y ?? 0,
        workArea: area ?? NO_RECT,
    };
}

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
export function takeStep(
    binding: BuiltinBinding,
    phase: number,
    event: DragEvent,
    client: StepClient | undefined,
    extras: StepExtras = {},
): DragOutcome {
    const bounds = client?.bounds ?? NO_RECT;
    return binding.step({
        clientFlags: client === undefined ? 0 : clientFlags(client),
        event: eventOf(phase, event, extras.origin, client),
        bounds,
        start: extras.start ?? bounds,
        sides: extras.sides ?? 0,
        state: client === undefined ? undefined : limitsOf(client),
        split: extras.split,
        tiles: extras.tiles ?? [],
    });
}

/** Whether the answer of a step sets `flag` (`OutcomeFlags.*`). */
export function has(outcome: DragOutcome, flag: number): boolean {
    return (outcome.flags & flag) !== 0;
}

/** A copy of the rectangle an answer gives, for a script to keep. */
export function rectOf(rect: Rect): Rect {
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
}

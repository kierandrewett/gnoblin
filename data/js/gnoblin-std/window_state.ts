// Handles window requests from clients and taskbars; the compositor waits for the shell to answer each request.
// Client listeners run first, followed by collection listeners in attachment order. The first `request.handle()`
// answers; if none does, the built-in policy runs. A throwing listener is reported and skipped.

import { ClientMoveBinding } from "gnoblin-std/bindings/move";
import { ClientResizeBinding } from "gnoblin-std/bindings/resize";
import type { ResizeCorner } from "gnoblin-std/bindings/resize";
import type { DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import type { FrameInset } from "gnoblin-core/inset";
import { onUiFrame, onUiRequest } from "gnoblin-std/ui/frame";
import { uiPointer } from "gnoblin-std/ui/session";
import type { UiRequest } from "gnoblin-std/ui/frame";
import { NEVER, onWake } from "gnoblin-core/wake";

/** Every request the compositor makes of a window. `menu` is answered by the window menu (`gnoblin/menus`). */
export type ClientRequestKind =
    | "maximize"
    | "unmaximize"
    | "fullscreen"
    | "unfullscreen"
    | "minimize"
    | "unminimize"
    | "close"
    | "raise"
    | "lower"
    | "activate"
    | "move"
    | "resize"
    | "menu";

/** Every `ClientRequestKind`, in the order the type lists them. Frozen, so a caller cannot widen the set. */
export const CLIENT_REQUEST_KINDS: readonly ClientRequestKind[] = Object.freeze([
    "maximize",
    "unmaximize",
    "fullscreen",
    "unfullscreen",
    "minimize",
    "unminimize",
    "close",
    "raise",
    "lower",
    "activate",
    "move",
    "resize",
    "menu",
]);

/** How near two sides of a window a resize request's pointer must be for the corner between them to move. */
export const RESIZE_CORNER_ZONE = 16;

/**
 * A drag a request started ends after this long even when no button edge arrives, so it cannot follow the pointer
 * indefinitely.
 */
export const REQUEST_DRAG_LIMIT_MS = 30_000;

/**
 * A request a client made of its window. A listener answers it by calling `handle()` (see the precedence in the header
 * of this module); a listener that does not leaves it to the next one and then to the built-in answer.
 */
export class ClientRequest<C = unknown> {
    /** Kind of compositor request. */
    readonly kind: ClientRequestKind;
    /** Window that received the request. */
    readonly client: C;
    /** Where a `menu` request is anchored, in the compositor's logical coordinates; 0 for every other kind. */
    readonly x: number;
    /** Vertical position of the menu request anchor in logical pixels. */
    readonly y: number;
    #handled = false;

    constructor(kind: ClientRequestKind, client: C, x = 0, y = 0) {
        this.kind = kind;
        this.client = client;
        this.x = x;
        this.y = y;
    }

    /** Whether a listener has answered the request. */
    get handled(): boolean {
        return this.#handled;
    }

    /** Takes the request: no later listener hears it and the built-in answer does not run. */
    handle(): void {
        this.#handled = true;
    }
}

/** What the policy reads and writes of a window. The host's `Client` satisfies it. */
export interface StateClient {
    /** Whether the window still exists. */
    readonly alive: boolean;
    /** Whether the window covers its output. */
    fullscreen: boolean;
    /** Whether the window fills its work area. */
    maximised: boolean;
    /** Whether the window is hidden. */
    minimised: boolean;
    /** A window a layout tiles is placed by the layout, so a move or resize request leaves it alone. */
    readonly tiled: boolean;
    /** Window rectangle in logical pixels. */
    bounds: Rect;
    /** The rectangle a snap restores for a maximised window; kept current here so a snap after a maximise is right. */
    floatingBounds?: Rect | null;
    /** Frame inset, when the window has a frame. */
    readonly frameInset?: FrameInset;
    /** Minimum content size in pixels, when set. */
    minSize?: { width: number; height: number };
    /** Maximum content size in pixels, when set. */
    maxSize?: { width: number; height: number };
}

/** What the policy needs of the shell around the windows. */
export interface WindowStateWorld<C extends StateClient> {
    /** Finds a window by its compositor node id. */
    clientOf(node: number): C | undefined;
    /** The windows of the workspace `client` is on, back to front. */
    siblings(client: C): readonly C[];
    /** Puts the window last among its siblings and keeps a fullscreen sibling over it. */
    raise(client: C): void;
    /** Lowers the window among its siblings. */
    lower(client: C): void;
    /** Closes the window. */
    close(client: C): void;
    /** The window with the keyboard focus, or null. */
    focused(): C | null;
    /** Gives the keyboard to `client`, or to nobody. A tree with no seat has nobody to give it to. */
    focus(client: C | null): void;
    /** Whether the window's workspace is the one on show. */
    onActiveWorkspace(client: C): boolean;
    /** The whole output `client` is on: what a fullscreen window covers. Null while the tree has no screen. */
    output(client?: C): Rect | null;
    /** The output `client` is on less what the bars take: what a maximised window fills. */
    workArea(client?: C): Rect | null;
    /** Converts a frame rectangle to the content rectangle by applying the frame inset. */
    framed(client: C, frame: Rect): Rect;
    /** Whether the window has a snap position. */
    snapped(client: C): boolean;
    /** Removes the window's snap position. */
    unsnap(client: C): void;
    /** Calls the `onRequest` of the client's listeners, in order, until one handles `request`. */
    dispatch(client: C, request: ClientRequest<C>): void;
    /** Reports an error from a window request. */
    report(site: string, error: unknown): void;
    /** Returns the shell clock reading in milliseconds. */
    now(): number;
    /** Returns the current pointer drag constraint, if any. */
    dragConstraint(): string | undefined;
    /** Whether edge tiling is enabled. */
    edgeTiling(): boolean;
}

interface RequestDrag {
    readonly client: StateClient;
    readonly binding: ClientMoveBinding | ClientResizeBinding;
    readonly state: object;
    readonly origin: Point;
    readonly started: number;
    last: Point;
}

/** The raw requests a listener handled, so the window menu does not open one a script answered. */
const handledRaw = new WeakSet<object>();

/** Whether a listener handled the compositor's request (`handle()`), as the window menu asks before it opens. */
export function isRequestHandled(request: UiRequest): boolean {
    return handledRaw.has(request);
}

/** The rectangles a window goes back to when it stops being maximised or fullscreen. */
export interface KeptRects {
    /** Floating rectangle restored when maximising ends, or `null` if none was saved. */
    floating: Rect | null;
    /** Windowed rectangle restored when fullscreen ends, or `null` if none was saved. */
    windowed: Rect | null;
}

/**
 * The stock answers to a window's requests, and the drags that `move` and `resize` start.
 *
 * - `maximize` fills the work area (the area left by bars) and keeps the floating rectangle. A snapped window is
 *   unsnapped first. Refused if already maximised or fullscreen.
 * - `fullscreen` covers the whole output (bars hide through `barsGiveWayTo` in gnoblin-std/widgets), keeps the windowed
 *   rectangle and puts the window last among its siblings. Refused if already fullscreen.
 * - `minimize` hides the window and, if it held focus, focuses the topmost visible sibling or nobody. `unminimize`
 *   shows it and focuses it only when nothing else has focus.
 * - `activate` presents a window only on the active workspace: unminimise, focus and raise. Other workspaces are
 *   unchanged.
 * - `raise` puts the window last among its siblings, below any fullscreen sibling (`Client.raise`).
 * - `move` and `resize` start the pointer drags used by `ClientMoveBinding` and `ClientResizeBinding`. Tiled,
 *   fullscreen and maximised windows are not dragged. The pointer chooses the resize edge or corner.
 */
export class WindowState {
    #world: WindowStateWorld<StateClient> | null = null;
    /** The floating rectangle a maximised window had, so un-maximising restores it. */
    readonly #floating = new WeakMap<object, Rect>();
    /** The rectangle a fullscreen window had, so leaving fullscreen restores it. */
    readonly #windowed = new WeakMap<object, Rect>();
    #drag: RequestDrag | null = null;

    /**
     * The rectangles `client` returns to when it leaves maximised or fullscreen. They live here, in the shell's script,
     * so a reload takes them with it unless the host carries them across: it reads this before it goes and `keep`s it
     * in the next.
     */
    kept(client: StateClient): KeptRects {
        return { floating: this.#floating.get(client) ?? null, windowed: this.#windowed.get(client) ?? null };
    }

    /** Gives `client` back the rectangles `kept` took, so it can leave maximised or fullscreen as it would have. */
    keep(client: StateClient, kept: Partial<KeptRects>): void {
        if (kept.floating) {
            this.#floating.set(client, { ...kept.floating });
        }
        if (kept.windowed) {
            this.#windowed.set(client, { ...kept.windowed });
        }
    }

    /** Whether the policy answers the compositor's requests. */
    get started(): boolean {
        return this.#world !== null;
    }

    /** Starts answering requests. A shell does this once, before the window menu starts. */
    start<C extends StateClient>(world: WindowStateWorld<C>): void {
        if (this.#world !== null) {
            return;
        }
        this.#world = world as unknown as WindowStateWorld<StateClient>;
        onUiRequest((raw) => this.#hear(raw));
        onUiFrame(() => this.#follow());
        // A request drag ends when it has run too long, whether or not the pointer moves.
        onWake("window drag limit", () => (this.#drag === null ? NEVER : this.#drag.started + REQUEST_DRAG_LIMIT_MS));
    }

    #need(): WindowStateWorld<StateClient> {
        if (this.#world === null) {
            throw new Error("windowState is not started: it starts with the shell");
        }
        return this.#world;
    }

    // -- the answers --------------------------------------------------------------------------------------

    /** Answers `kind` for `client` as the built-in policy does. A listener that handled a request may call it. */
    answer(client: StateClient, kind: ClientRequestKind): void {
        switch (kind) {
            case "maximize":
                this.maximise(client);
                break;
            case "unmaximize":
                this.unmaximise(client);
                break;
            case "fullscreen":
                this.enterFullscreen(client);
                break;
            case "unfullscreen":
                this.leaveFullscreen(client);
                break;
            case "minimize":
                this.minimise(client);
                break;
            case "unminimize":
                this.unminimise(client);
                break;
            case "close":
                this.#need().close(client);
                break;
            case "raise":
                this.raise(client);
                break;
            case "lower":
                this.lower(client);
                break;
            case "activate":
                this.present(client);
                break;
            case "move":
                this.beginMove(client);
                break;
            case "resize":
                this.beginResize(client);
                break;
            case "menu":
                break;
        }
    }

    /** Fills the work area and keeps the floating rectangle. Refused (false) when maximised or fullscreen already. */
    maximise(client: StateClient): boolean {
        const world = this.#need();
        if (client.maximised || client.fullscreen) {
            return false;
        }
        if (world.snapped(client)) {
            world.unsnap(client);
        }
        const before = { ...client.bounds };
        this.#floating.set(client, before);
        client.floatingBounds = { ...before };
        client.maximised = true;
        const area = world.workArea(client);
        if (area !== null) {
            client.bounds = world.framed(client, area);
        }
        return true;
    }

    /** Leaves maximised and returns the floating rectangle. Refused (false) when it is not maximised. */
    unmaximise(client: StateClient): boolean {
        if (!client.maximised) {
            return false;
        }
        client.maximised = false;
        const before = this.#floating.get(client);
        this.#floating.delete(client);
        if (before !== undefined) {
            client.bounds = before;
        }
        return true;
    }

    /**
     * Covers the whole output, bars included, and goes last among its siblings. The rectangle it had is kept, so
     * leaving fullscreen returns a floating, snapped or maximised window to it. Refused (false) when it is already
     * fullscreen.
     */
    enterFullscreen(client: StateClient): boolean {
        const world = this.#need();
        if (client.fullscreen) {
            return false;
        }
        const output = world.output(client);
        if (output !== null) {
            this.#windowed.set(client, { ...client.bounds });
        }
        client.fullscreen = true;
        if (output !== null) {
            client.bounds = output;
            world.raise(client);
        }
        return true;
    }

    /** Returns the rectangle the window had before it went fullscreen. Refused (false) when it is not fullscreen. */
    leaveFullscreen(client: StateClient): boolean {
        if (!client.fullscreen) {
            return false;
        }
        client.fullscreen = false;
        const before = this.#windowed.get(client);
        this.#windowed.delete(client);
        if (before !== undefined) {
            client.bounds = before;
        }
        return true;
    }

    /**
     * Hides the window and hands the keyboard focus on, as the window manager did: the topmost sibling still showing,
     * or nobody. A window that did not hold the focus leaves it where it is. Refused (false) when minimised already.
     */
    minimise(client: StateClient): boolean {
        const world = this.#need();
        if (client.minimised) {
            return false;
        }
        client.minimised = true;
        if (world.focused() === client) {
            const siblings = world.siblings(client);
            let next: StateClient | null = null;
            for (let at = siblings.length - 1; at >= 0; at--) {
                const candidate = siblings[at];
                if (candidate !== client && candidate.alive && !candidate.minimised) {
                    next = candidate;
                    break;
                }
            }
            world.focus(next);
        }
        return true;
    }

    /**
     * Shows the window again. The keyboard goes to it only when nothing holds it. Refused (false) when not minimised.
     */
    unminimise(client: StateClient): boolean {
        const world = this.#need();
        if (!client.minimised) {
            return false;
        }
        client.minimised = false;
        if (world.focused() === null) {
            world.focus(client);
        }
        return true;
    }

    /**
     * An activation: raises and focuses a window on the active workspace, and brings a minimised one back. A window on
     * a workspace that is not showing is left alone. Returns whether it ran.
     */
    present(client: StateClient): boolean {
        const world = this.#need();
        if (!world.onActiveWorkspace(client)) {
            return false;
        }
        if (client.minimised) {
            this.unminimise(client);
        }
        world.focus(client);
        world.raise(client);
        return true;
    }

    /** Raises the window among its siblings. */
    raise(client: StateClient): void {
        this.#need().raise(client);
    }

    /** Lowers the window among its siblings. */
    lower(client: StateClient): void {
        this.#need().lower(client);
    }

    // -- the drags a move or resize request starts ---------------------------------------------------------

    /** Starts the stock move drag for `client`, following the seat's pointer. False when none starts. */
    beginMove(client: StateClient): boolean {
        return this.#begin(client, () => new ClientMoveBinding({ id: "request-move" }));
    }

    /** Starts the stock resize drag for `client`. The corner or side is chosen from where the pointer is. */
    beginResize(client: StateClient): boolean {
        return this.#begin(
            client,
            (at) => new ClientResizeBinding({ id: "request-resize", edges: this.#resizeEdges(client, at) }),
        );
    }

    /** Whether a drag a request started is following the pointer. */
    get dragging(): boolean {
        return this.#drag !== null;
    }

    /**
     * The shell tells the policy of every pointer button edge on a window. A press or a release ends the drag a request
     * started, after one last step to where the pointer is.
     */
    pointerEdge(kind: string): void {
        if (this.#drag !== null && (kind === "press" || kind === "release")) {
            this.#end();
        }
    }

    #resizeEdges(client: StateClient, at: Point): ResizeCorner | "nearest" {
        const b = client.bounds;
        const x = at.x - b.x;
        const y = at.y - b.y;
        const nearX = Math.min(x, b.width - x);
        const nearY = Math.min(y, b.height - y);
        if (nearX <= RESIZE_CORNER_ZONE && nearY <= RESIZE_CORNER_ZONE) {
            return `${y < b.height / 2 ? "top" : "bottom"}-${x < b.width / 2 ? "left" : "right"}`;
        }
        return "nearest";
    }

    #event(client: StateClient, origin: Point, at: Point): DragEvent {
        const world = this.#need();
        const b = client.bounds;
        const event = {
            context: null,
            device: null,
            trigger: "request",
            timestamp: world.now(),
            deltaTime: 0,
            modifiers: new Set<string>(),
            client,
            position: at,
            localPosition: { x: at.x - b.x, y: at.y - b.y },
            x: at.x,
            y: at.y,
            dx: at.x - origin.x,
            dy: at.y - origin.y,
            button: 1,
            workArea: world.workArea(client) ?? undefined,
            dragConstraint: world.dragConstraint(),
            edgeTiling: world.edgeTiling(),
        };
        return event as unknown as DragEvent;
    }

    #begin(client: StateClient, make: (at: Point) => ClientMoveBinding | ClientResizeBinding): boolean {
        const world = this.#need();
        if (!client.alive || client.tiled || client.fullscreen || client.maximised) {
            return false;
        }
        const pointer = uiPointer();
        if (pointer === null) {
            return false;
        }
        if (this.#drag !== null) {
            this.#end();
        }
        const binding = make(pointer);
        const state = binding.onStart(this.#event(client, pointer, pointer));
        if (state === false) {
            return false;
        }
        this.#drag = { client, binding, state, origin: pointer, started: world.now(), last: pointer };
        return true;
    }

    /**
     * Moves the drag to the current pointer position. A missing window or expired limit ends the drag. `ending` marks
     * the final update, so it does not request another update for the same limit.
     */
    #follow(ending = false): void {
        const drag = this.#drag;
        if (drag === null) {
            return;
        }
        const world = this.#need();
        if (!drag.client.alive) {
            this.#drag = null;
            return;
        }
        if (!ending && world.now() - drag.started >= REQUEST_DRAG_LIMIT_MS) {
            this.#end();
            return;
        }
        const at = uiPointer();
        if (at === null || (at.x === drag.last.x && at.y === drag.last.y)) {
            return;
        }
        drag.last = at;
        try {
            drag.binding.onUpdate(this.#event(drag.client, drag.origin, at), drag.state);
        } catch (error) {
            this.#drag = null;
            world.report("windowState.drag", error);
        }
    }

    #end(): void {
        const drag = this.#drag;
        if (drag === null) {
            return;
        }
        this.#follow(true);
        this.#drag = null;
        if (!drag.client.alive) {
            return;
        }
        try {
            drag.binding.onEnd(this.#event(drag.client, drag.origin, drag.last), drag.state);
        } catch (error) {
            this.#need().report("windowState.drag", error);
        }
    }

    // -- the compositor's requests -------------------------------------------------------------------------

    #hear(raw: UiRequest): void {
        const world = this.#world;
        if (world === null || !(CLIENT_REQUEST_KINDS as readonly string[]).includes(raw.request)) {
            return;
        }
        const client = world.clientOf(raw.node);
        if (client === undefined || !client.alive) {
            return;
        }
        const request = new ClientRequest<StateClient>(raw.request as ClientRequestKind, client, raw.x, raw.y);
        world.dispatch(client, request);
        if (request.handled) {
            handledRaw.add(raw);
            return;
        }
        this.answer(client, request.kind);
    }
}

/** The stock policy. The shell starts it; a script calls its methods to answer a request it handled. */
export const windowState = new WindowState();

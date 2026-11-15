// Keyboard move and resize modes let a user place a window without a pointer. The menu's Move and Resize actions start
// them; the arrow keys move by `REPOSITION_STEP` pixels (or grow/shrink by that amount when resizing).
//
// Return, Enter or Space keeps the new place; Escape restores the previous one. The mode holds the keyboard and gives
// it back to the window when it ends. It also ends when the window closes or the session locks. Only floating windows
// can move this way; maximised, fullscreen and tiled windows are refused. A snapped window is unsnapped first.
// Only one window can be repositioned at a time. See `crates/gnoblin-js/tests/reposition.rs`.

import { clampAxis, dragConstraintMode } from "gnoblin/bindings/drag";
import { growRect, NO_INSET, shrinkRect } from "gnoblin/frame/inset";
import type { FrameInset } from "gnoblin/frame/inset";
import type { Rect } from "gnoblin/layout";
import { restoreSnap, snapRecord, unsnapClient } from "gnoblin/snap";
import type { SnapClient, SnapRecord } from "gnoblin/snap";
import { Box } from "gnoblin/ui/box";
import { onUiFrame } from "gnoblin/ui/frame";
import { uiLocked } from "gnoblin/ui/session";
import { Text } from "gnoblin/ui/text";
import type { UiKeyEvent } from "gnoblin/ui/events";

/** How far an arrow key moves or resizes the window, in pixels. */
export const REPOSITION_STEP = 16;
/** The least width a resize keeps, in pixels. */
export const REPOSITION_MIN_WIDTH = 120;
/** The least height a resize keeps, in pixels. */
export const REPOSITION_MIN_HEIGHT = 80;

/** What an interactive drag does to a window: `move` carries it, `resize` changes its size. */
export type RepositionMode = "move" | "resize";

/** What the mode needs of a window. A `Client` of the host module is one. */
export interface RepositionClient extends SnapClient {
    /** False after the window closes. */
    readonly alive: boolean;
    /** Whether the window fills its output. */
    readonly fullscreen: boolean;
    /** Whether the window is maximised. */
    readonly maximised: boolean;
    /** Whether the window is arranged by a layout. */
    readonly tiled: boolean;
    /** Whether the window's contents are secure. */
    readonly secure?: boolean;
    /** Frame extents around the content, in logical pixels. */
    readonly frameInset?: FrameInset;
    /** Current workspace, or null when the window has none. */
    readonly workspace: { readonly active: boolean } | null;
    /** Saved floating rectangle, in logical pixels. */
    floatingBounds?: Rect | null;
}

/** What the mode needs of the shell around it. */
export interface RepositionWorld {
    /** The area the window may move in: the work area of the output it is on. */
    workArea(client: RepositionClient): Rect | null;
    /** The seat's `drag_constraint`: how far a window may leave the area. */
    dragConstraint(): unknown;
    /** Gives the keyboard to the window. */
    focus(client: RepositionClient): void;
}

/** Arrow key names to the step each moves: `[dx, dy]` in units of `REPOSITION_STEP`. */
const ARROWS: Readonly<Record<string, readonly [number, number]>> = {
    Left: [-1, 0],
    Right: [1, 0],
    Up: [0, -1],
    Down: [0, 1],
};

/** The node that holds the keyboard while the mode runs. It draws one line: what the keys do. */
export class RepositionGrab extends Box {
    /** Whether arrow keys move or resize the window. */
    readonly mode: RepositionMode;
    /** Window being moved or resized. */
    readonly client: RepositionClient;
    /** Original rectangle, restored when the mode is cancelled. */
    readonly from: Rect;
    /** Previous snap state, restored when the mode is cancelled. */
    readonly snapped: SnapRecord | null;
    readonly #owner: Repositioning;

    constructor(
        owner: Repositioning,
        client: RepositionClient,
        mode: RepositionMode,
        from: Rect,
        snapped: SnapRecord | null,
    ) {
        super({
            x: 20,
            y: 20,
            width: 520,
            height: 36,
            radius: 8,
            fill: "#1e1e2e",
            padding: 8,
            direction: "row",
            secure: client.secure === true,
        });
        this.#owner = owner;
        this.client = client;
        this.mode = mode;
        this.from = from;
        this.snapped = snapped;
        const what = mode === "move" ? "Move" : "Resize";
        this.add(new Text({ text: `${what}: arrow keys, Return to keep it, Escape to cancel`, size: 14, grow: 1 }));
    }

    /** One arrow key: the window moves, or grows or shrinks, by `dx`, `dy`. */
    step(dx: number, dy: number, world: RepositionWorld): void {
        const { client } = this;
        const area = world.workArea(client);
        if (area === null) {
            return;
        }
        const at = client.bounds;
        if (this.mode === "move") {
            // The frame is what stays inside the area: the window's frame lies round the content.
            const inset = client.frameInset ?? NO_INSET;
            const mode = dragConstraintMode(world.dragConstraint());
            const frame = growRect(at, inset);
            const kept = {
                ...frame,
                x: clampAxis(frame.x + dx, frame.width, area.x, area.width, mode),
                y: clampAxis(frame.y + dy, frame.height, area.y, area.height, mode),
            };
            client.bounds = { ...shrinkRect(kept, inset), width: at.width, height: at.height };
        } else {
            // The frame's far edges are what reach the area's.
            const inset = client.frameInset ?? NO_INSET;
            const widest = area.x + area.width - at.x - inset.right;
            const tallest = area.y + area.height - at.y - inset.bottom;
            client.bounds = {
                x: at.x,
                y: at.y,
                width: Math.max(REPOSITION_MIN_WIDTH, Math.min(at.width + dx, widest)),
                height: Math.max(REPOSITION_MIN_HEIGHT, Math.min(at.height + dy, tallest)),
            };
        }
    }

    /** Handles the repositioning mode's arrow, accept and cancel keys. */
    override onKey(event: UiKeyEvent): void {
        // The keys are taken, pressed or released: this node is the keyboard's until it ends.
        event.stop();
        if (!event.pressed) {
            return;
        }
        const arrow = ARROWS[event.key];
        if (arrow !== undefined) {
            this.#owner.step(this, arrow[0] * REPOSITION_STEP, arrow[1] * REPOSITION_STEP);
        } else if (event.key === "Return" || event.key === "KP_Enter" || event.key === "space") {
            this.#owner.finish(true);
        } else if (event.key === "Escape") {
            this.#owner.finish(false);
        }
    }
}

/** The keyboard move and resize mode of the shell: at most one window is repositioned at a time. */
export class Repositioning {
    #world: RepositionWorld | null = null;
    #current: RepositionGrab | null = null;

    /** Starts watching for the window going and the session locking. A shell does this once. */
    start(world: RepositionWorld): void {
        this.#world = world;
        onUiFrame(() => this.#watch());
    }

    /** The window being repositioned, or `null`. */
    get current(): RepositionClient | null {
        return this.#current?.client ?? null;
    }

    /** The node that holds the keyboard while the mode runs, or `null`. */
    get grab(): RepositionGrab | null {
        return this.#current;
    }

    /**
     * Starts the mode for `client`. False for a window that is fullscreen, maximised, tiled, dead or not showing on its
     * output; a snapped one is unsnapped first.
     */
    begin(client: RepositionClient, mode: RepositionMode): boolean {
        const world = this.#world;
        if (world === null) {
            return false;
        }
        this.finish(true);
        if (
            !client.alive ||
            client.fullscreen ||
            client.maximised ||
            client.tiled ||
            client.workspace === null ||
            !client.workspace.active
        ) {
            return false;
        }
        const from = { ...client.bounds };
        const snapped = snapRecord(client);
        if (snapped !== null) {
            unsnapClient(client);
        }
        const grab = new RepositionGrab(this, client, mode, from, snapped);
        grab.mountTop();
        grab.focus();
        this.#current = grab;
        return true;
    }

    /** One arrow key of the running mode. */
    step(grab: RepositionGrab, dx: number, dy: number): void {
        if (grab === this.#current && this.#world !== null) {
            grab.step(dx, dy, this.#world);
        }
    }

    /**
     * Ends the mode. `keep` keeps where the window is; otherwise it goes back to where the mode found it, snapped again
     * if it was. False when no mode was running.
     */
    finish(keep: boolean): boolean {
        const grab = this.#current;
        if (grab === null) {
            return false;
        }
        this.#current = null;
        const { client } = grab;
        if (client.alive) {
            if (keep) {
                client.floatingBounds = { ...client.bounds };
            } else {
                client.bounds = { ...grab.from };
                if (grab.snapped !== null) {
                    restoreSnap(client, grab.snapped);
                }
            }
        }
        grab.destroy();
        if (client.alive) {
            this.#world?.focus(client);
        }
        return true;
    }

    #watch(): void {
        const grab = this.#current;
        if (grab === null) {
            return;
        }
        if (!grab.client.alive) {
            this.finish(false);
        } else if (uiLocked()) {
            this.finish(true);
        }
    }
}

/** The mode of the stock shell. Started by the module that owns the clients. */
export const repositioning = new Repositioning();

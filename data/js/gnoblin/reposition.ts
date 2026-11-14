// Moving and resizing a window with the keyboard, ported from data/retired shell/gnoblin/shell/reposition.retired shell. The window menu's Move
// and Resize start it, so a window can be placed without a pointer.
//
//   Client.moveWithKeys()     the arrow keys move the window by STEP pixels, inside the work area
//   Client.resizeWithKeys()   Right and Down grow the window by STEP pixels, Left and Up shrink it
//
// Return, Enter and Space keep the new place; Escape puts the window back where it was. The mode takes the keyboard while
// it runs and gives it back to the window when it ends. It ends when the window closes and when the session locks. Only a
// floating window can be moved this way: a maximised, fullscreen or tiled window is refused (`false`), and a snapped one
// is unsnapped first, as a drag does. One window is repositioned at a time. Each case has its test in
// crates/gnoblin-js/tests/reposition.rs (the cases of tests/retired shell/retired corpus reposition).

import { clampAxis, dragConstraintMode } from "gnoblin/bindings";
import type { Rect } from "gnoblin/layout";
import { restoreSnap, snapRecord, unsnapClient } from "gnoblin/snap";
import type { SnapClient, SnapRecord } from "gnoblin/snap";
import { Box, Text, onUiFrame, uiLocked } from "gnoblin/ui";
import type { UiKeyEvent } from "gnoblin/ui";

/** How far an arrow key moves or resizes the window, in pixels. */
export const REPOSITION_STEP = 16;
/** The least size a resize keeps, in pixels. */
export const REPOSITION_MIN_WIDTH = 120;
export const REPOSITION_MIN_HEIGHT = 80;

export type RepositionMode = "move" | "resize";

/** What the mode needs of a window. A `Client` of the host module is one. */
export interface RepositionClient extends SnapClient {
    readonly alive: boolean;
    readonly fullscreen: boolean;
    readonly maximised: boolean;
    readonly tiled: boolean;
    readonly secure?: boolean;
    readonly frameInset?: number;
    readonly workspace: { readonly active: boolean } | null;
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

const ARROWS: Readonly<Record<string, readonly [number, number]>> = {
    Left: [-1, 0],
    Right: [1, 0],
    Up: [0, -1],
    Down: [0, 1],
};

/** The node that holds the keyboard while the mode runs. It draws one line: what the keys do. */
export class RepositionGrab extends Box {
    readonly mode: RepositionMode;
    readonly client: RepositionClient;
    readonly from: Rect;
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
            // The frame is what stays inside the area: a titlebar lies above the content.
            const inset = client.frameInset ?? 0;
            const mode = dragConstraintMode(world.dragConstraint());
            client.bounds = {
                x: clampAxis(at.x + dx, at.width, area.x, area.width, mode),
                y: clampAxis(at.y + dy - inset, at.height + inset, area.y, area.height, mode) + inset,
                width: at.width,
                height: at.height,
            };
        } else {
            const widest = area.x + area.width - at.x;
            const tallest = area.y + area.height - at.y;
            client.bounds = {
                x: at.x,
                y: at.y,
                width: Math.max(REPOSITION_MIN_WIDTH, Math.min(at.width + dx, widest)),
                height: Math.max(REPOSITION_MIN_HEIGHT, Math.min(at.height + dy, tallest)),
            };
        }
    }

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
     * Ends the mode. `keep` keeps where the window is; otherwise it goes back to where the mode found it, snapped again if
     * it was. False when no mode was running.
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

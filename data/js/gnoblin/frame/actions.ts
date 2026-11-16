// `WindowActions`: shared actions for a window frame, including close, minimise, maximise and seat click gestures.
// Custom frame nodes and stock parts use the same actions.

import type { FrameClient, FrameHooks } from "gnoblin/frame/client";
import { uiNow, uiSeatAction } from "gnoblin/ui/session";
import { NEVER, onWake } from "gnoblin/wake";

/** The retired Lua shell's fade on close: out over 0.18 s, as a window shrinks to 0.92 of its size. */
export const CLOSE_FADE_MS = 180;
/**
 * A window still there this long after it was asked to close comes back, so a client that asks "save changes?" is
 * not left faded out.
 */
export const CLOSE_RESTORE_MS = 1500;
/** The fade back in of a window that stayed open after it was asked to close, in milliseconds. */
const RESTORE_DURATION_MS = 120;

/** What the seat's property says each click does when the tree has no seat to ask: its schema default. */
const DEFAULT_ACTIONS = { double_click: "toggle-maximize", middle_click: "lower", right_click: "menu" } as const;

/** The three clicks the seat gives an action to. */
export type SeatGesture = keyof typeof DEFAULT_ACTIONS;

/** The actions whose window is fading out to close, which come due on the clock whatever the compositor does. */
const closingActions = new Set<WindowActions>();
onWake("window close fade", () => {
    let at = NEVER;
    for (const actions of closingActions) {
        const due = actions.wakeAt();
        if (due === NEVER) {
            closingActions.delete(actions);
        }
        at = Math.min(at, due);
    }
    return at;
});

/** The fade of a window being closed: when to close it, and when to bring it back if it stayed. */
interface Closing {
    closeAt: number | null;
    restoreAt: number | null;
}

/**
 * The window's actions. `advance` runs once a frame (a frame does it), which takes a fade on to its next step.
 *
 * `close` fades the window out first (`CLOSE_FADE_MS`) and then closes it; a window that is still there
 * `CLOSE_RESTORE_MS` later is shown again, as a client that asked the user something is. A window the client closes
 * itself is gone from the compositor at once, so there is nothing to fade.
 */
export class WindowActions {
    /** The window acted on by these shared actions. */
    readonly client: FrameClient;
    /** The shell actions used for operations that need compositor support. */
    readonly hooks: FrameHooks;
    /** The clock the double click and the fade are timed by, in milliseconds. */
    now: () => number = () => uiNow();
    #closing: Closing | null = null;

    constructor(client: FrameClient, hooks: FrameHooks = {}) {
        this.client = client;
        this.hooks = hooks;
    }

    /** Whether the window is fading out to be closed, or has been asked to close and has not gone yet. */
    get closing(): boolean {
        return this.#closing !== null;
    }

    /**
     * Time of the next frame needed for the close fade or to restore a window that stayed open.
     */
    wakeAt(): number {
        return this.#closing === null ? NEVER : (this.#closing.closeAt ?? this.#closing.restoreAt ?? NEVER);
    }

    /** Starts the fade out of a window that is being closed. A second call while it fades does nothing. */
    close(): void {
        if (this.#closing !== null) {
            return;
        }
        const { client } = this;
        if (client.set === undefined) {
            client.close();
            return;
        }
        closingActions.add(this);
        const fade = this.hooks.motion?.(CLOSE_FADE_MS) ?? CLOSE_FADE_MS;
        client.set({ opacity: 0, scale: [0.92, 0.92, 1] }, { duration: fade, easing: "ease-in" });
        if (fade === 0) {
            // Reduced motion: nothing to wait for, so the window is asked to close now. It comes back if it stays.
            this.#closing = { closeAt: null, restoreAt: this.now() + CLOSE_RESTORE_MS };
            client.close();
            return;
        }
        this.#closing = { closeAt: this.now() + fade, restoreAt: null };
    }

    /**
     * Advances a close: closes the window after the fade, or restores it if the client stayed open.
     */
    advance(): void {
        const closing = this.#closing;
        const { client } = this;
        if (closing === null) {
            return;
        }
        if (client.alive === false) {
            this.#closing = null;
            return;
        }
        const now = this.now();
        if (closing.closeAt !== null && now >= closing.closeAt) {
            closing.closeAt = null;
            closing.restoreAt = now + CLOSE_RESTORE_MS;
            client.close();
        } else if (closing.restoreAt !== null && now >= closing.restoreAt) {
            this.#closing = null;
            client.set?.({ opacity: 1, scale: [1, 1, 1] }, { duration: RESTORE_DURATION_MS, easing: "ease-out" });
        }
    }

    /** Maximises the window, or restores it, as the shell does it (the hook), else by writing the flag. */
    toggleMaximise(): void {
        const { client } = this;
        const on = !client.maximised;
        if (this.hooks.maximise === undefined) {
            client.maximised = on;
        } else {
            this.hooks.maximise(client, on);
        }
    }

    /** Minimises the window as the shell does it (the hook), else by writing the flag. */
    minimise(): void {
        const { client } = this;
        if (this.hooks.minimise === undefined) {
            client.minimised = true;
        } else {
            this.hooks.minimise(client);
        }
    }

    /** Runs what the seat says a click does. An action the schema does not name is an error. */
    gesture(property: SeatGesture): void {
        const { client } = this;
        const action = uiSeatAction(property) ?? DEFAULT_ACTIONS[property];
        switch (action) {
            case "toggle-maximize":
                this.toggleMaximise();
                return;
            case "lower":
                this.hooks.lower?.(client);
                return;
            case "menu":
                this.hooks.menu?.(client);
                return;
            case "minimize":
                this.minimise();
                return;
            // Maximising along one axis has no stock behaviour yet, as in the retired Lua shell: it does nothing.
            case "toggle-maximize-horizontally":
            case "toggle-maximize-vertically":
            case "none":
                return;
            default:
                throw new Error(`${property} must name a titlebar action, got ${action}`);
        }
    }
}

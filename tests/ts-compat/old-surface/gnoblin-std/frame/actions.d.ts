import type { FrameClient, FrameHooks } from "gnoblin-std/frame/client";
/** The retired Lua shell's fade on close: out over 0.18 s, as a window shrinks to 0.92 of its size. */
export declare const CLOSE_FADE_MS = 180;
/**
 * A window still there this long after it was asked to close comes back, so a client that asks "save changes?" is
 * not left faded out.
 */
export declare const CLOSE_RESTORE_MS = 1500;
/** What the seat's property says each click does when the tree has no seat to ask: its schema default. */
declare const DEFAULT_ACTIONS: {
    readonly double_click: "toggle-maximize";
    readonly middle_click: "lower";
    readonly right_click: "menu";
};
/** The three clicks the seat gives an action to. */
export type SeatGesture = keyof typeof DEFAULT_ACTIONS;
/**
 * The window's actions. `advance` runs once a frame (a frame does it), which takes a fade on to its next step.
 *
 * `close` fades the window out first (`CLOSE_FADE_MS`) and then closes it; a window that is still there
 * `CLOSE_RESTORE_MS` later is shown again, as a client that asked the user something is. A window the client closes
 * itself is gone from the compositor at once, so there is nothing to fade.
 */
export declare class WindowActions {
    #private;
    /** The window acted on by these shared actions. */
    readonly client: FrameClient;
    /** The shell actions used for operations that need compositor support. */
    readonly hooks: FrameHooks;
    /** The clock the double click and the fade are timed by, in milliseconds. */
    now: () => number;
    constructor(client: FrameClient, hooks?: FrameHooks);
    /** Whether the window is fading out to be closed, or has been asked to close and has not gone yet. */
    get closing(): boolean;
    /**
     * Time of the next frame needed for the close fade or to restore a window that stayed open.
     */
    wakeAt(): number;
    /** Starts the fade out of a window that is being closed. A second call while it fades does nothing. */
    close(): void;
    /**
     * Advances a close: closes the window after the fade, or restores it if the client stayed open.
     */
    advance(): void;
    /** Maximises the window, or restores it, as the shell does it (the hook), else by writing the flag. */
    toggleMaximise(): void;
    /** Minimises the window as the shell does it (the hook), else by writing the flag. */
    minimise(): void;
    /** Runs what the seat says a click does. An action the schema does not name is an error. */
    gesture(property: SeatGesture): void;
}
export {};

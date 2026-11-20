import type { Client } from "gnoblin-core/host/client";
import type { Rect } from "gnoblin-core/types";
/** What a script wrote to a window the library does not know yet. */
interface Held {
    floating?: boolean;
    floatingBounds?: Rect | null;
}
/** What is held for `client`, made when there is none. */
export declare function holdFor(client: Client): Held;
/**
 * Runs a call on the library's window; the answer of `waiting` is given when the library does not know the window yet. Any
 * other failure goes on.
 */
export declare function whenKnown<T>(call: () => T, waiting: () => T): T;
/** Whether the script marked the window floating: a layout leaves it where it is. */
export declare function isFloating(client: Client): boolean;
/** Marks the window floating, or not. */
export declare function setFloating(client: Client, floating: boolean): void;
/** Whether the layout that last placed the window wants it hidden (a monocle's other windows), whatever workspace it is on. */
export declare function hiddenByLayout(client: Client): boolean;
/** The rectangle the window floats at, or null while the script has kept none. */
export declare function floatingBoundsOf(client: Client): Rect | null;
/** Keeps the rectangle the window floats at, or forgets it with null. */
export declare function setFloatingBounds(client: Client, bounds: Rect | null | undefined): void;
/** Whether the library knows the window yet. */
export declare function isKnown(client: Client): boolean;
/** Gives the library what was held for the windows it knows now; a window that closed is forgotten. */
export declare function flushHeld(): void;
export {};

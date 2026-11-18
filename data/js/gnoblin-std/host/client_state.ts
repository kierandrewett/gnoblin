// What the library keeps about a window beyond the tree, as a script reads and writes it: whether the script marked it floating
// and the rectangle it floats at.
//
// The engine hears of a window a turn before the library does (the engine's frame reads the compositor's word first, the
// library's turn follows it), so a script that answers a new window in its own listener can name a window the library does not
// know yet. A write to such a window is held here and given to the library in the next frame, when it knows the window; a read of
// it answers what was written, or what a window that was never written to has.

import type { Client } from "gnoblin-core/host/client";
import type { Rect } from "gnoblin-core/types";
import { ClientFlags } from "gnoblin-std/generated/constants";
import { failureCode, FAILED, nativeClient } from "gnoblin-std/host/library";

/** What a script wrote to a window the library does not know yet. */
interface Held {
    floating?: boolean;
    floatingBounds?: Rect | null;
}

const held = new Map<Client, Held>();

/** What is held for `client`, made when there is none. */
export function holdFor(client: Client): Held {
    let found = held.get(client);
    if (found === undefined) {
        found = {};
        held.set(client, found);
    }
    return found;
}

/**
 * Runs a call on the library's window; the answer of `waiting` is given when the library does not know the window yet. Any
 * other failure goes on.
 */
export function whenKnown<T>(call: () => T, waiting: () => T): T {
    try {
        return call();
    } catch (error) {
        // A window the library has not heard of is a node it does not know, or a member it cannot find in a collection.
        const code = failureCode(error);
        if (code !== FAILED.noSuchNode && code !== FAILED.noSuchMember) {
            throw error;
        }
        return waiting();
    }
}

/** Whether the script marked the window floating: a layout leaves it where it is. */
export function isFloating(client: Client): boolean {
    return whenKnown(
        () => nativeClient(client).isFloating(),
        () => held.get(client)?.floating ?? false,
    );
}

/** Marks the window floating, or not. */
export function setFloating(client: Client, floating: boolean): void {
    whenKnown(
        () => nativeClient(client).setFloating(floating ? 1 : 0),
        () => {
            holdFor(client).floating = floating;
            return 0;
        },
    );
}

/** Whether the layout that last placed the window wants it hidden (a monocle's other windows), whatever workspace it is on. */
export function hiddenByLayout(client: Client): boolean {
    return whenKnown(
        () => (nativeClient(client).state().flags & ClientFlags.LAYOUT_HIDES) !== 0,
        () => false,
    );
}

/** The rectangle the window floats at, or null while the script has kept none. */
export function floatingBoundsOf(client: Client): Rect | null {
    return whenKnown(
        () => {
            const state = nativeClient(client).state();
            return (state.flags & ClientFlags.HAS_FLOATING) === 0 ? null : { ...state.floating };
        },
        () => held.get(client)?.floatingBounds ?? null,
    );
}

/** Keeps the rectangle the window floats at, or forgets it with null. */
export function setFloatingBounds(client: Client, bounds: Rect | null | undefined): void {
    whenKnown(
        () => {
            const native = nativeClient(client);
            const state = native.state();
            native.setState({
                ...state,
                flags:
                    bounds == null ? state.flags & ~ClientFlags.HAS_FLOATING : state.flags | ClientFlags.HAS_FLOATING,
                floating: bounds == null ? { x: 0, y: 0, width: 0, height: 0 } : { ...bounds },
            });
            return 0;
        },
        () => {
            holdFor(client).floatingBounds = bounds ?? null;
            return 0;
        },
    );
}

/** Whether the library knows the window yet. */
export function isKnown(client: Client): boolean {
    return whenKnown(
        () => {
            nativeClient(client).state();
            return true;
        },
        () => false,
    );
}

/** Gives the library what was held for the windows it knows now; a window that closed is forgotten. */
export function flushHeld(): void {
    for (const [client, writes] of [...held]) {
        if (!client.alive) {
            held.delete(client);
        } else if (isKnown(client)) {
            held.delete(client);
            if (writes.floating !== undefined) {
                setFloating(client, writes.floating);
            }
            if (writes.floatingBounds !== undefined) {
                setFloatingBounds(client, writes.floatingBounds);
            }
        }
    }
}

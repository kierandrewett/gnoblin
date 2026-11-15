// What the shell reads of the first seat: which window holds the keyboard focus, the focus settings it published,
// and whether it asks for reduced motion. The stock policy reads the seat and not `client.focused`: a shell gives
// focus by writing the seat's `focus`.

import { FOCUS_DEFAULTS, seatFocusSettings } from "gnoblin/focus";
import type { FocusSettings } from "gnoblin/focus";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import type { Member } from "gnoblin/host/collection_state";
import { ctx } from "gnoblin/host/context";
import { isClient } from "gnoblin/host/node_state";

/**
 * The duration in milliseconds the shell gives an animation: `duration`, or 0 while the seat asks for reduced motion
 * (`appearance.reduced-motion`, the seat's `reduced_motion`). A 0 makes a slide, a fade or `Client.animate` an instant write.
 * The compositor lands a timed write at once under reduced motion too, so a shell that forgets this still obeys.
 */
export function motion(duration: number): number {
    const seat = mainSeat();
    return seat !== undefined && host.input.settings(seat)?.reducedMotion === true ? 0 : duration;
}

/** The window the first seat's keyboard focus is on, if it is one of the compositor's. */
export function focusedClient(): Client | null {
    const seat = host.input.seats()[0];
    return seat === undefined ? null : (ctx.clientsByNode.get(host.input.focus(seat)) ?? null);
}

/**
 * Whether the first seat holds its keyboard focus on `client`. The stock policy reads the seat, not
 * `client.focused`: a shell gives focus by writing the seat's `focus`, and a real compositor was seen not to keep a
 * window's own `focused` current while a shell holds the tree.
 */
export function hasFocus(client: Client): boolean {
    const seat = host.input.seats()[0];
    return seat !== undefined && ctx.clientsByNode.get(host.input.focus(seat)) === client;
}

/**
 * The member of `list` that holds the keyboard focus. With a seat it is the one the seat's focus names; a tree with
 * no seat has no seat focus to read, so the member whose own `focused` is set is the answer.
 */
export function focusedAmong(list: readonly Member[]): Member | null {
    const seat = host.input.seats()[0];
    if (seat === undefined) {
        return list.find((member) => isClient(member) && member.focused) ?? null;
    }
    const held = ctx.clientsByNode.get(host.input.focus(seat));
    return list.find((member) => member === held) ?? null;
}

/** Resolves the effective focus policy from the first seat's published settings. */
export function currentFocusSettings(): FocusSettings {
    const seat = host.input.seats()[0];
    const published = seat === undefined ? undefined : host.input.settings(seat);
    return published === undefined ? FOCUS_DEFAULTS : seatFocusSettings(published);
}

/** The seat that is not the XR program's: the keyboard and pointer of the desktop. */
export function mainSeat(): number | undefined {
    return host.input.seats().find((seat) => host.input.seatName(seat) !== "xr");
}

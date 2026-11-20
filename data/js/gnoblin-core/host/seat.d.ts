import type { Client } from "gnoblin-core/host/client";
import type { Member } from "gnoblin-core/host/collection_state";
/**
 * The duration in milliseconds the shell gives an animation: `duration`, or 0 while the seat asks for reduced motion
 * (`appearance.reduced-motion`, the seat's `reduced_motion`). A 0 makes a slide, a fade or a transition an
 * instant write. The compositor lands a timed write at once under reduced motion too, so a shell that forgets this
 * still obeys.
 */
export declare function motion(duration: number): number;
/** The window the first seat's keyboard focus is on, if it is one of the compositor's. */
export declare function focusedClient(): Client | null;
/**
 * Whether the first seat holds its keyboard focus on `client`. The stock policy reads the seat, not
 * `client.focused`: a shell gives focus by writing the seat's `focus`, and a real compositor was seen not to keep a
 * window's own `focused` current while a shell holds the tree.
 */
export declare function hasFocus(client: Client): boolean;
/**
 * The member of `list` that holds the keyboard focus. With a seat it is the one the seat's focus names; a tree with
 * no seat has no seat focus to read, so the member whose own `focused` is set is the answer.
 */
export declare function focusedAmong(list: readonly Member[]): Member | null;
/** The seat that is not the XR program's: the keyboard and pointer of the desktop. */
export declare function mainSeat(): number | undefined;

import { Box } from "gnoblin-std/ui/box";
import { Text } from "gnoblin-std/ui/text";
/** How long a toast takes to come and go. */
export interface ToastOptions {
    /** Milliseconds to fade in. Default 120. */
    fadeIn?: number;
    /** Milliseconds to stay at full strength. Default 4000. */
    hold?: number;
    /** Milliseconds to fade out. Default 300. */
    fadeOut?: number;
    /** The clock, in milliseconds: the toolkit's by default. A test passes its own. */
    now?: () => number;
}
/** A card with one message that fades in, holds, fades out and destroys itself. */
export declare class Toast extends Box {
    #private;
    /** The text displayed in this toast. */
    readonly message: Text;
    /** Milliseconds to fade in. */
    fadeIn: number;
    /** Milliseconds at full strength. */
    hold: number;
    /** Milliseconds to fade out. */
    fadeOut: number;
    /** The clock, in milliseconds. */
    now: () => number;
    constructor(text: string, options?: ToastOptions);
    /** Updates the fade from the toast's age and destroys it after fade-out. */
    refresh(): void;
}
/** Shows `text` in a toast, over the windows. It removes itself after 4420 ms. */
export declare function toast(text: string, options?: ToastOptions): Toast;

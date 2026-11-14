// The stock panel's message: `toast(text)` shows a short message in the top right corner of the screen, fades it in,
// holds it, fades it out and removes it.
//
// It owns the toast card. It does not own the bar: that is `Bar`, in `gnoblin/widgets`. Read `gnoblin/ui` first for
// `Box`, `Text` and the frame loop that calls `refresh()`.

import { Box, Text, uiNow, uiScreen } from "gnoblin/ui";

/** The card's width in pixels. */
const TOAST_WIDTH = 320;
/** The card's height in pixels. */
const TOAST_HEIGHT = 64;
/** The gap in pixels between the card and the screen's right edge, plus the card's width. */
const TOAST_RIGHT_INSET = 340;
/** The distance in pixels from the top of the screen, below the bar. */
const TOAST_TOP = 44;
/** The card's corner radius in pixels. */
const TOAST_RADIUS = 10;
/** The space in pixels between the card's edge and its text. */
const TOAST_PADDING = 14;
/** The card's background colour. */
const TOAST_FILL = "#45475a";

/** How long a toast takes to come and go. */
export interface ToastOptions {
    /** Seconds to fade in. Default 0.12. */
    fadeIn?: number;
    /** Seconds to stay at full strength. Default 4. */
    hold?: number;
    /** Seconds to fade out. Default 0.3. */
    fadeOut?: number;
    /** The clock, in milliseconds: the toolkit's by default. A test passes its own. */
    now?: () => number;
}

/** A card with one message that fades in, holds, fades out and destroys itself. */
export class Toast extends Box {
    readonly message: Text;
    /** Seconds to fade in. */
    fadeIn = 0.12;
    /** Seconds at full strength. */
    hold = 4;
    /** Seconds to fade out. */
    fadeOut = 0.3;
    /** The clock, in milliseconds. */
    now: () => number = () => uiNow();
    readonly #start: number;

    constructor(text: string, options: ToastOptions = {}) {
        // 340 is the width the card takes when the tree has no screen yet; the card then sits at x = 0.
        const screen = uiScreen() ?? { width: TOAST_RIGHT_INSET, height: 0 };
        super({
            x: screen.width - TOAST_RIGHT_INSET,
            y: TOAST_TOP,
            width: TOAST_WIDTH,
            height: TOAST_HEIGHT,
            radius: TOAST_RADIUS,
            padding: TOAST_PADDING,
            direction: "row",
            fill: TOAST_FILL,
            opacity: 0,
        });
        if (options.fadeIn !== undefined) this.fadeIn = options.fadeIn;
        if (options.hold !== undefined) this.hold = options.hold;
        if (options.fadeOut !== undefined) this.fadeOut = options.fadeOut;
        if (options.now !== undefined) this.now = options.now;
        this.#start = this.now();
        this.message = new Text({ text, grow: 1 });
        this.add(this.message);
        this.mountTop();
    }

    // The opacity follows the age of the card, so the frame loop drives the fade and no timer is needed.
    override refresh(): void {
        const seconds = (this.now() - this.#start) / 1000;
        const { fadeIn, hold, fadeOut } = this;
        if (seconds < fadeIn) {
            this.opacity = seconds / fadeIn;
        } else if (seconds < fadeIn + hold) {
            this.opacity = 1;
        } else if (seconds < fadeIn + hold + fadeOut) {
            this.opacity = 1 - (seconds - fadeIn - hold) / fadeOut;
        } else {
            this.destroy();
        }
    }
}

/** Shows `text` in a toast, over the windows. It removes itself after about four seconds. */
export function toast(text: string, options: ToastOptions = {}): Toast {
    return new Toast(text, options);
}

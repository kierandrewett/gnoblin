// The stock panel's message: `toast(text)` shows a short message in the top right corner of the screen, fades it in,
// holds it, fades it out and removes it.
//
// It owns the toast card. It does not own the bar: that is `Bar`, in `gnoblin/widgets`. Read `gnoblin/ui` first for
// `Box`, `Text` and the frame loop that calls `refresh()`.

import { Box, Text, uiNow, uiScreen } from "gnoblin/ui";

import { NEVER, NOW, onWake } from "gnoblin/wake";

/** The toasts that have not ended: a card's fade follows its age, so the frame loop runs while one is up. */
const toasts = new Set<Toast>();
onWake("toasts", () => {
    for (const toast of toasts) {
        if (toast.alive) {
            return NOW;
        }
        toasts.delete(toast);
    }
    return NEVER;
});

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
export class Toast extends Box {
    readonly message: Text;
    /** Milliseconds to fade in. */
    fadeIn = 120;
    /** Milliseconds at full strength. */
    hold = 4000;
    /** Milliseconds to fade out. */
    fadeOut = 300;
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
        toasts.add(this);
    }

    // The opacity follows the age of the card, so the frame loop drives the fade and no timer is needed.
    override refresh(): void {
        const age = this.now() - this.#start;
        const { fadeIn, hold, fadeOut } = this;
        if (age < fadeIn) {
            this.opacity = age / fadeIn;
        } else if (age < fadeIn + hold) {
            this.opacity = 1;
        } else if (age < fadeIn + hold + fadeOut) {
            this.opacity = 1 - (age - fadeIn - hold) / fadeOut;
        } else {
            this.destroy();
        }
    }
}

/** Shows `text` in a toast, over the windows. It removes itself after 4420 ms. */
export function toast(text: string, options: ToastOptions = {}): Toast {
    return new Toast(text, options);
}

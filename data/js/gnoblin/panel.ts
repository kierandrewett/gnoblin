// The stock panel's message: `toast(text)` shows a short message in the top right corner of the screen, fades it in, holds
// it, fades it out and removes it (data/retired shell/gnoblin/shell/panel.retired shell). The bar the retired panel module shared a file with is
// `Bar`, in `gnoblin/widgets`.

import { Box, Text, uiNow, uiScreen } from "gnoblin/ui";

export interface ToastOptions {
    /** Seconds to fade in. Default 0.12. */
    fadeIn?: number;
    /** Seconds to stay at full strength. Default 4. */
    hold?: number;
    /** Seconds to fade out. Default 0.3. */
    fadeOut?: number;
    /** The clock, in milliseconds: the toolkit's by default. */
    now?: () => number;
}

/** A card with one message that fades in, holds, fades out and destroys itself. */
export class Toast extends Box {
    readonly message: Text;
    fadeIn = 0.12;
    hold = 4;
    fadeOut = 0.3;
    now: () => number = () => uiNow();
    readonly #start: number;

    constructor(text: string, options: ToastOptions = {}) {
        const screen = uiScreen() ?? { width: 340, height: 0 };
        super({
            x: screen.width - 340,
            y: 44,
            width: 320,
            height: 64,
            radius: 10,
            padding: 14,
            direction: "row",
            fill: "#45475a",
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

/** Shows `text` for a few seconds. */
export function toast(text: string, options: ToastOptions = {}): Toast {
    return new Toast(text, options);
}

// `WindowButtons`: the window's buttons in a row, in the order and with the icons the options name.

import type { FrameContext } from "gnoblin/frame/client";
import { WindowButton } from "gnoblin/frame/parts/window_button";
import type { WindowButtonName, WindowButtonOptions } from "gnoblin/frame/parts/window_button";
import { Box } from "gnoblin/ui";
import type { BoxOptions } from "gnoblin/ui";

/** Options of `WindowButtons`. `buttons` are the buttons left to right. */
export interface WindowButtonsOptions extends BoxOptions {
    buttons?: readonly WindowButtonName[];
    /** What each button is made with: its size, colour and icon, and the maximise button's `restoreIcon`. */
    button?: WindowButtonOptions;
    /** The icon of a button by name, over the `button` options. */
    icons?: Partial<Record<WindowButtonName, string>>;
}

/**
 * The buttons of a window as a row. It is as wide as its buttons and gap make it. Place it in any outlet, left or
 * right or both, and give it a `gap`.
 */
export class WindowButtons extends Box {
    readonly buttons = new Map<WindowButtonName, WindowButton>();

    constructor(context: FrameContext, options: WindowButtonsOptions = {}) {
        const { buttons = ["minimise", "maximise", "close"], button = {}, icons = {}, ...box } = options;
        super({ direction: "row", ...box });
        for (const name of buttons) {
            const icon = icons[name];
            const made = new WindowButton(context, name, icon === undefined ? button : { ...button, icon });
            this.buttons.set(name, made);
            this.add(made);
        }
    }

    protected override naturalWidth(): number {
        const widths = [...this.buttons.values()].map((button) => button.width);
        const gaps = this.gap * Math.max(widths.length - 1, 0);
        return widths.reduce((sum, width) => sum + width, 0) + gaps + 2 * this.padding;
    }

    protected override naturalHeight(): number {
        return Math.max(0, ...[...this.buttons.values()].map((button) => button.height)) + 2 * this.padding;
    }
}

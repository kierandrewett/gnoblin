import type { FrameContext } from "gnoblin-std/frame/client";
import { WindowButton } from "gnoblin-std/frame/parts/window_button";
import type { WindowButtonName, WindowButtonOptions } from "gnoblin-std/frame/parts/window_button";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
/** Options of `WindowButtons`. `buttons` are the buttons left to right. */
export interface WindowButtonsOptions extends BoxOptions {
    /** Buttons to create, in left-to-right order. */
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
export declare class WindowButtons extends Box {
    /** The created buttons, keyed by their action name. */
    readonly buttons: Map<WindowButtonName, WindowButton>;
    constructor(context: FrameContext, options?: WindowButtonsOptions);
    protected naturalWidth(): number;
    protected naturalHeight(): number;
}

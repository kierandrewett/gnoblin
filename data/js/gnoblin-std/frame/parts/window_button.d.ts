import type { FrameContext } from "gnoblin-std/frame/client";
import { Icon } from "gnoblin-std/ui/icon";
import type { IconOptions } from "gnoblin-std/ui/icon";
/** The three window buttons. */
export type WindowButtonName = "minimise" | "maximise" | "close";
/** The icon of each button, by name from the icon theme. */
export declare const defaultButtonIcons: Readonly<Record<WindowButtonName, string>>;
/** Options of a `WindowButton`. The icon, size and colour are those of `Icon`. */
export interface WindowButtonOptions extends IconOptions {
    /** The icon the maximise button shows while the window is maximised. Left out, it keeps its one icon. */
    restoreIcon?: string;
}
/**
 * A window button. A left press does what the button says through the window's actions (`frame.actions`): close fades
 * the window out and closes it, maximise toggles it as the shell does, minimise hides it. The press does not reach the
 * node under the button, so a button in a drag area is not a drag.
 */
export declare class WindowButton extends Icon {
    #private;
    /** Window operations and action hooks shared by the frame. */
    readonly context: FrameContext;
    /** Which window action this button invokes. */
    readonly name: WindowButtonName;
    /** The icon shown while the window is maximised, for the maximise button. */
    restoreIcon: string | null;
    constructor(context: FrameContext, name: WindowButtonName, options?: WindowButtonOptions);
    /** Updates the maximise icon when the window's maximised state changes. */
    refresh(): void;
}

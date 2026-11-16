// `WindowButton`: one of the window's three buttons as an icon: minimise, maximise or close.

import type { FrameContext } from "gnoblin-std/frame/client";
import { Icon } from "gnoblin-std/ui/icon";
import type { UiPointerEvent } from "gnoblin-std/ui/events";
import type { IconOptions } from "gnoblin-std/ui/icon";

/** The three window buttons. */
export type WindowButtonName = "minimise" | "maximise" | "close";

/** The icon of each button, by name from the icon theme. */
export const defaultButtonIcons: Readonly<Record<WindowButtonName, string>> = {
    minimise: "window-minimize-symbolic",
    maximise: "window-maximize-symbolic",
    close: "window-close-symbolic",
};

/** The buttons of the pointer, as the compositor numbers them (Linux `BTN_LEFT`). */
const LEFT = 272;

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
export class WindowButton extends Icon {
    /** Window operations and action hooks shared by the frame. */
    readonly context: FrameContext;
    /** Which window action this button invokes. */
    readonly name: WindowButtonName;
    /** The icon shown while the window is maximised, for the maximise button. */
    restoreIcon: string | null;
    readonly #icon: string;

    constructor(context: FrameContext, name: WindowButtonName, options: WindowButtonOptions = {}) {
        const { restoreIcon, ...icon } = options;
        super({ icon: defaultButtonIcons[name], size: 16, color: "#cdd6f4", ...icon });
        this.context = context;
        this.name = name;
        this.restoreIcon = restoreIcon ?? null;
        this.#icon = this.icon;
        this.onPress = (event) => this.#press(event);
    }

    /** Updates the maximise icon when the window's maximised state changes. */
    override refresh(): void {
        if (this.name === "maximise" && this.restoreIcon !== null) {
            this.icon = this.context.client.maximised ? this.restoreIcon : this.#icon;
        }
    }

    #press(event: UiPointerEvent): void {
        event.stop();
        if (event.button !== LEFT) {
            return;
        }
        const { actions } = this.context;
        if (this.name === "close") {
            actions.close();
        } else if (this.name === "maximise") {
            actions.toggleMaximise();
        } else {
            actions.minimise();
        }
    }
}

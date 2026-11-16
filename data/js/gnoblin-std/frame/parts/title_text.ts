// `TitleText`: the window's title as one line of text that follows the window. Put it in any outlet.

import type { FrameContext } from "gnoblin-std/frame/client";
import { Text } from "gnoblin-std/ui/text";
import type { TextOptions } from "gnoblin-std/ui/text";

/**
 * The window's title, ellipsized to the room it has. It reads the title once a frame, so a title that changes is
 * shown without the script running again. It takes the room a row leaves (`grow` 1) and has the stock titlebar's
 * size, colour and font unless the options say otherwise.
 */
export class TitleText extends Text {
    /** Window context that supplies the title. */
    readonly context: FrameContext;

    constructor(context: FrameContext, options: TextOptions = {}) {
        super({ grow: 1, size: 13, color: "#c6d0f5", font: "sans", ellipsize: true, ...options });
        this.context = context;
    }

    /** Updates the text from the window's current title. */
    override refresh(): void {
        this.text = this.context.client.title;
    }
}

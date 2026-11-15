// A box that paints one icon from the installed icon theme.

import { Box } from "gnoblin/ui/box";
import type { BoxOptions } from "gnoblin/ui/box";
import { DEFAULT_ICON_SIZE, DEFAULT_TEXT_COLOUR } from "gnoblin/ui/paint";
import type { Color, PaintOp } from "gnoblin/ui/paint";

/** Options for an `Icon`. `size` is in logical pixels. */
export interface IconOptions extends BoxOptions {
    /** Icon name from the installed icon theme. */
    icon?: string;
    /** Icon size in logical pixels; defaults to `DEFAULT_ICON_SIZE`. */
    size?: number;
    /** Icon colour; defaults to `DEFAULT_TEXT_COLOUR`. */
    color?: Color;
}

/** Paints one icon from the installed theme, at `size` logical pixels square unless its node has another size. */
export class Icon extends Box {
    /** Icon name from the installed icon theme. */
    icon = "";
    /** Icon size in logical pixels. */
    size = DEFAULT_ICON_SIZE;
    /** Icon colour. */
    color: Color = DEFAULT_TEXT_COLOUR;

    constructor(options: IconOptions = {}) {
        super(options);
        if (options.icon !== undefined) this.icon = options.icon;
        if (options.size !== undefined) this.size = options.size;
        if (options.color !== undefined) this.color = options.color;
    }

    protected override naturalWidth(): number {
        return this.size;
    }

    protected override naturalHeight(): number {
        return this.size;
    }

    /** Paints the selected theme icon at the configured size and colour. */
    override draw(): PaintOp[] {
        return [{ op: "icon", name: this.icon, x: 0, y: 0, size: this.size, color: this.color }];
    }
}

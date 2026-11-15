// `Icon`, one icon of the icon theme.

import { Box } from "gnoblin/ui/box";
import type { BoxOptions } from "gnoblin/ui/box";
import { DEFAULT_ICON_SIZE, DEFAULT_TEXT_COLOUR } from "gnoblin/ui/paint";
import type { Color, PaintOp } from "gnoblin/ui/paint";

/** Options for `Icon`. */
export interface IconOptions extends BoxOptions {
    icon?: string;
    size?: number;
    color?: Color;
}

/** One icon of the icon theme, `size` pixels square unless the node is given another size. */
export class Icon extends Box {
    icon = "";
    size = DEFAULT_ICON_SIZE;
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

    override draw(): PaintOp[] {
        return [{ op: "icon", name: this.icon, x: 0, y: 0, size: this.size, color: this.color }];
    }
}

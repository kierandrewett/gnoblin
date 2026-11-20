import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
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
export declare class Icon extends Box {
    /** Icon name from the installed icon theme. */
    icon: string;
    /** Icon size in logical pixels. */
    size: number;
    /** Icon colour. */
    color: Color;
    constructor(options?: IconOptions);
    protected naturalWidth(): number;
    protected naturalHeight(): number;
    /** Paints the selected theme icon at the configured size and colour. */
    draw(): PaintOp[];
}

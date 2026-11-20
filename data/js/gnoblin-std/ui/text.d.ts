import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { UiAnswer } from "gnoblin-core/host/ui_backend";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
/** The measured width and height of text, and the distance from the top to its first baseline, in logical pixels. */
export interface TextMetrics {
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
    /** Baseline distance from the top of the first line, in logical pixels. */
    ascent: number;
}
/**
 * Estimates text metrics until the compositor returns a measurement. The estimate uses half the font size per
 * character, a line height of one and a half font sizes and a baseline at 1.1 times the font size.
 */
export declare function estimateText(text: string, size: number): TextMetrics;
/** Options for a `Text`. Sizes are in logical pixels. */
export interface TextOptions extends BoxOptions {
    /** Text content; defaults to an empty string. */
    text?: string;
    /** Font size in logical pixels; defaults to `DEFAULT_TEXT_SIZE`. */
    size?: number;
    /** Text colour; defaults to `DEFAULT_TEXT_COLOUR`. */
    color?: Color;
    /** Font name; defaults to `DEFAULT_FONT`. */
    font?: string;
    /** Whether to shorten text that exceeds the available width; defaults to `false`. */
    ellipsize?: boolean;
}
/**
 * A text canvas uses the compositor's measurement for its font, size and text. Until the answer arrives, it uses
 * `estimateText`. When `ellipsize` is true, text wider than the available width is shortened and ends with an ellipsis.
 */
export declare class Text extends Box {
    #private;
    /** Text content. */
    text: string;
    /** Font size in logical pixels. */
    size: number;
    /** Text colour. */
    color: Color;
    /** Font name. */
    font: string;
    /** Whether to shorten text that exceeds the available width. */
    ellipsize: boolean;
    constructor(options?: TextOptions);
    /** The baseline's distance from the top of the first line, in logical pixels. */
    get ascent(): number;
    /** Returns the measured metrics for the current font, size and text, or an estimate until the answer arrives. */
    metrics(): TextMetrics;
    /** Applies a measurement only if it matches the current text, font and size. */
    answered(key: string, answer: UiAnswer): void;
    protected naturalWidth(): number;
    protected naturalHeight(): number;
    /** Returns the complete text, or the width-limited text with an ellipsis when `ellipsize` is true. */
    shown(): string;
    /**
     * Centres the text block vertically. If the node is shorter than the block, it is clipped equally at the top and
     * bottom.
     */
    draw(): PaintOp[];
}

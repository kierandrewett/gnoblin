// Text canvases use compositor measurements for natural size and an estimate until each answer arrives.

import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { UiAnswer } from "gnoblin-std/ui/backend";
import { DEFAULT_FONT, DEFAULT_TEXT_COLOUR, DEFAULT_TEXT_SIZE } from "gnoblin-std/ui/paint";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
import { host, measuring, reportError } from "gnoblin-std/ui/state";

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
export function estimateText(text: string, size: number): TextMetrics {
    let widest = 0;
    let lines = 0;
    for (const line of text.split("\n")) {
        widest = Math.max(widest, Math.floor(([...line].length * size) / 2));
        lines += 1;
    }
    return { width: widest, height: lines * Math.floor((size * 3) / 2), ascent: Math.floor((size * 11) / 10) };
}

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
export class Text extends Box {
    /** Text content. */
    text = "";
    /** Font size in logical pixels. */
    size = DEFAULT_TEXT_SIZE;
    /** Text colour. */
    color: Color = DEFAULT_TEXT_COLOUR;
    /** Font name. */
    font = DEFAULT_FONT;
    /** Whether to shorten text that exceeds the available width. */
    ellipsize = false;
    // The answer in use, and the key it was measured under. It stops applying once the key changes.
    #measured: (TextMetrics & { key: string }) | null = null;
    // The key last asked. A text asks once per key: the compositor is not asked again for a key it was already asked
    // about. A changed font, size or text is a new key and asks again.
    #asked: string | null = null;

    constructor(options: TextOptions = {}) {
        super(options);
        if (options.text !== undefined) this.text = options.text;
        if (options.size !== undefined) this.size = options.size;
        if (options.color !== undefined) this.color = options.color;
        if (options.font !== undefined) this.font = options.font;
        if (options.ellipsize !== undefined) this.ellipsize = options.ellipsize;
    }

    /** The baseline's distance from the top of the first line, in logical pixels. */
    get ascent(): number {
        return this.metrics().ascent;
    }

    /** Returns the measured metrics for the current font, size and text, or an estimate until the answer arrives. */
    metrics(): TextMetrics {
        // The separator is a control character that does not occur in text, so two different triples do not
        // share a key.
        const key = `${this.font}\u0001${this.size}\u0001${this.text}`;
        if (this.#measured !== null && this.#measured.key === key) {
            return this.#measured;
        }
        if (this.#asked !== key && this.alive) {
            this.#asked = key;
            const token = host().measure(this.node, this.font, this.size, this.text);
            if (token !== undefined) {
                measuring.set(token, { text: this, key });
            }
        }
        return estimateText(this.text, this.size);
    }

    /** Applies a measurement only if it matches the current text, font and size. */
    answered(key: string, answer: UiAnswer): void {
        if (this.#asked !== key) {
            return;
        }
        const parsed = /^(\d+) (\d+) (\d+)$/.exec(answer.result);
        // An error keeps the estimate for this text: asking again would get the same refusal.
        const taken =
            parsed === null
                ? estimateText(this.text, this.size)
                : { width: Number(parsed[1]), height: Number(parsed[2]), ascent: Number(parsed[3]) };
        this.#measured = { ...taken, key };
        if (answer.error !== "") {
            reportError(
                "Text.measure",
                new Error(`Text ${JSON.stringify(this.text)}: measure failed: ${answer.error}`),
            );
        }
    }

    protected override naturalWidth(): number {
        return this.metrics().width;
    }

    protected override naturalHeight(): number {
        return this.metrics().height;
    }

    /** Returns the complete text, or the width-limited text with an ellipsis when `ellipsize` is true. */
    shown(): string {
        const measured = this.metrics();
        const chars = [...this.text];
        if (!this.ellipsize || this.width >= measured.width || measured.width === 0) {
            return this.text;
        }
        // One character of room is left for the ellipsis.
        const keep = Math.max(0, Math.floor((chars.length * this.width) / measured.width) - 1);
        return `${chars.slice(0, keep).join("")}\u2026`;
    }

    /**
     * Centres the text block vertically. If the node is shorter than the block, it is clipped equally at the top and
     * bottom.
     */
    override draw(): PaintOp[] {
        const y = Math.floor((this.height - this.metrics().height) / 2);
        return [{ op: "text", text: this.shown(), x: 0, y, size: this.size, color: this.color, font: this.font }];
    }
}

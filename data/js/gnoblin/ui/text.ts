// `Text`, one string whose natural size is what the compositor measures, and the estimate used until it answers.

import { Box } from "gnoblin/ui/box";
import type { BoxOptions } from "gnoblin/ui/box";
import type { UiAnswer } from "gnoblin/ui/backend";
import { DEFAULT_FONT, DEFAULT_TEXT_COLOUR, DEFAULT_TEXT_SIZE } from "gnoblin/ui/paint";
import type { Color, PaintOp } from "gnoblin/ui/paint";
import { host, measuring, reportError } from "gnoblin/ui/state";

/** How much room a string takes: the width and height of its lines, and the distance down to the first baseline. */
export interface TextMetrics {
    width: number;
    height: number;
    ascent: number;
}

/**
 * Until the compositor answers a measurement, a text is sized by a guess: a sans glyph averages half the font size, a
 * line box is one and a half sizes tall and the baseline sits a little over one size below its top. The fractions are
 * rough on purpose: the answer replaces them within a frame, so they only need to keep the layout from jumping.
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

/** Options for `Text`. Sizes are pixels. */
export interface TextOptions extends BoxOptions {
    text?: string;
    size?: number;
    color?: Color;
    font?: string;
    ellipsize?: boolean;
}

/**
 * One string. Its natural size is what the compositor measures for its font, size and text: the answer arrives a
 * frame later, and until then the size is `estimateText`'s. With `ellipsize` a text wider than the room its row gave
 * it is cut to the share that fits and ends in an ellipsis, so a row never has to know how wide a title is.
 */
export class Text extends Box {
    text = "";
    size = DEFAULT_TEXT_SIZE;
    color: Color = DEFAULT_TEXT_COLOUR;
    font = DEFAULT_FONT;
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

    /** Pixels from the top of the first line to its baseline: what texts of different sizes are lined up by. */
    get ascent(): number {
        return this.metrics().ascent;
    }

    /** What the compositor measured for the current font, size and text, or the estimate while it has not yet. */
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

    /** Takes a measurement's answer. An answer to a question the text has since moved on from is ignored. */
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

    /** The text as drawn: all of it, or with `ellipsize` the part that fits the node's width and an ellipsis. */
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
     * The text block is centred in the node, so a row that squeezes the node shorter than the block (a titlebar)
     * trims both ends evenly and not just the bottom.
     */
    override draw(): PaintOp[] {
        const y = Math.floor((this.height - this.metrics().height) / 2);
        return [{ op: "text", text: this.shown(), x: 0, y, size: this.size, color: this.color, font: this.font }];
    }
}

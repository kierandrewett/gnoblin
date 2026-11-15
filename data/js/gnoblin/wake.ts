// When the shell needs a frame without anything arriving.
//
// The shell runs a frame when the compositor sent something, a link beside the tree has something, or script ran
// outside a frame (`crates/gnoblin-js/src/shell/gate.rs`). What is left is the script's own clock and the parts that
// draw every frame: a timer that comes due, a delay that runs out, a lock screen. Each such part registers one source
// where it keeps its state, and the shell asks `nextWake()` at the end of every frame.

/** The reading of the shell's clock, in milliseconds, at which a source next needs a frame; `Infinity` for never. */
export type WakeSource = () => number;

/** A source that wants a frame on every turn while it holds: a screen that is up, a session that is running. */
export const NOW = 0;

/** A source with nothing to wait for. */
export const NEVER = Number.POSITIVE_INFINITY;

const sources: WakeSource[] = [];

/**
 * Registers a source for the life of the generation. A source is called once per frame that ran, so it may look at
 * what it owns, but it must not look at every window: the cost of a frame that ran is what gating exists to avoid on
 * the frames that did not.
 */
export function onWake(source: WakeSource): void {
    sources.push(source);
}

/** The earliest reading any source asks for: `Infinity` when none does, a reading in the past for "now". */
export function nextWake(): number {
    let at = NEVER;
    for (const source of sources) {
        const wanted = source();
        if (wanted < at) {
            at = wanted;
        }
    }
    return at;
}

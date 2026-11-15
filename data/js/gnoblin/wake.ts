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

const sources: { readonly owner: string; readonly source: WakeSource }[] = [];

/**
 * Registers a source for the life of the generation, under the name of the part that owns it (`"focus rests"`, `"lock
 * screen"`). A source is called once per frame that ran, so it may look at what it owns, but it must not look at every
 * window: the cost of a frame that ran is what gating exists to avoid on the frames that did not.
 *
 * The name is for the test that lists what is registered (`crates/gnoblin-js/tests/wake_sources.rs`), which fails
 * when a part that keeps a clock does not register.
 */
export function onWake(owner: string, source: WakeSource): void {
    sources.push({ owner, source });
}

/** The owner of each registered source, in the order they registered. */
export function wakeOwners(): string[] {
    return sources.map(({ owner }) => owner);
}

/**
 * The earliest reading any source asks for: `Infinity` when none does, a reading in the past for "now". The shell asks
 * at the end of every frame, so a source that throws is handed to `failed` and asks for nothing: one part's failure
 * must not end the shell.
 */
export function nextWake(failed: (owner: string, error: unknown) => void): number {
    let at = NEVER;
    for (const { owner, source } of sources) {
        let wanted: number;
        try {
            wanted = source();
        } catch (error) {
            failed(owner, error);
            continue;
        }
        if (wanted < at) {
            at = wanted;
        }
    }
    return at;
}

// Frame wake sources let timers, animations and other time-based features ask the shell for their next frame.
// Each source registers its state and the shell calls `nextWake()` at the end of every frame.

/** The reading of the shell's clock, in milliseconds, at which a source next needs a frame; `Infinity` for never. */
export type WakeSource = () => number;

/** A source that wants a frame on every turn while it holds: a screen that is up, a session that is running. */
export const NOW = 0;

/** A source with nothing to wait for. */
export const NEVER = Number.POSITIVE_INFINITY;

const sources: { readonly owner: string; readonly source: WakeSource }[] = [];

/**
 * Registers a named source for the life of the generation. The shell calls it once per frame to check when its owner
 * next needs a frame; it should inspect only the state it owns, not every window.
 *
 * The name appears in the test that checks that frame clocks are registered
 * (`crates/gnoblin-js/tests/wake_sources.rs`).
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

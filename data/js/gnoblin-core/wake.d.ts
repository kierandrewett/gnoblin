/** The reading of the shell's clock, in milliseconds, at which a source next needs a frame; `Infinity` for never. */
export type WakeSource = () => number;
/** A source that wants a frame on every turn while it holds: a screen that is up, a session that is running. */
export declare const NOW = 0;
/** A source with nothing to wait for. */
export declare const NEVER: number;
/**
 * Registers a named source for the life of the generation. The shell calls it once per frame to check when its owner
 * next needs a frame; it should inspect only the state it owns, not every window.
 *
 * The name appears in the test that checks that frame clocks are registered
 * (`crates/gnoblin-js/tests/wake_sources.rs`).
 */
export declare function onWake(owner: string, source: WakeSource): void;
/** The owner of each registered source, in the order they registered. */
export declare function wakeOwners(): string[];
/**
 * The earliest reading any source asks for: `Infinity` when none does, a reading in the past for "now". The shell asks
 * at the end of every frame, so a source that throws is handed to `failed` and asks for nothing: one part's failure
 * must not end the shell.
 */
export declare function nextWake(failed: (owner: string, error: unknown) => void): number;

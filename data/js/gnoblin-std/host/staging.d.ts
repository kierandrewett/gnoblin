/**
 * Installs transition staging around host reads, writes and calls. Called once, before anything uses `host`. It wraps
 * the functions as the shell handed them over (`native`) and replaces the read and write tracking core installed with
 * its own, which does the same and stages in between: while a transition callback runs, the engine's writes are the
 * library's staged writes (`transition_set`), so the library's own passes at the commit stage with them.
 */
export declare function installStaging(): void;

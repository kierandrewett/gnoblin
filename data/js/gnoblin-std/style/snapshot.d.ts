import type { Style } from "gnoblin-std/style/style";
/**
 * Takes the state of the styles `current()` yields (the scalars, the lists and each list element's fields) and returns
 * what puts it back. A style that `current()` yields only later (made after the snapshot) is emptied. A failed
 * `requestTransition` uses this to discard style writes made by a failed callback. A list handed out earlier is not
 * restored; the script must read it again.
 */
export declare function snapshotStyles(current: () => Iterable<Style>): () => void;

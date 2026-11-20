import type { Output } from "gnoblin-core/host/output";
import type { OutputInfo } from "gnoblin-std/emitted/records";
import type { Workspace } from "gnoblin-std/host/workspace";
/** What the library knows of an output, or null while it has not heard of it: the engine hears of an output a turn before. */
export declare function outputInfo(output: Output): OutputInfo | null;
/** The outputs that are connected, in the order the compositor made them. */
export declare function liveOutputs(): Output[];
/** Whether the output has a desk of its own: it is on and not a mirror. */
export declare function hasDesk(output: Output): boolean;
/** The outputs that have a desk: every connected output but a mirror and one that is off. */
export declare function deskOutputs(): Output[];
/** The output flagged primary, or the first one; null while the tree has none. */
export declare function primaryOutput(): Output | null;
/** The workspace an output shows (the one its span shows, for a spanned output), or null. */
export declare function shownOn(output: Output): Workspace | null;
/** The workspace a new window, a key and a plain `show` act on: the one the output under the pointer shows, else the primary one's. */
export declare function currentWorkspace(): Workspace | null;

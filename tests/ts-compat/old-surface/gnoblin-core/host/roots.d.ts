import { MutableCollection } from "gnoblin-core/host/collections";
import type { Output } from "gnoblin-core/host/output";
/** `gnoblin.outputs`: the connected outputs. */
export declare class OutputCollection extends MutableCollection<Output> {
}
/** Creates the root collections for the current generation. */
export declare function installRoots(): void;

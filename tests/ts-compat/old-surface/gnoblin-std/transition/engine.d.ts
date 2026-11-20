import { Transition } from "gnoblin-std/transition/transition";
import type { TransitionHost, TransitionOptions } from "gnoblin-std/transition/types";
/** Runs callbacks as transitions over a host. */
export declare class TransitionEngine {
    #private;
    constructor(host: TransitionHost);
    /** Runs `callback` once and stages its property writes as a transition. */
    request(callback: () => void, options?: TransitionOptions): Transition;
}
/**
 * The shorthand of one object: `object.set(props, options)` is one `requestTransition` that assigns each property.
 * `label` names the object in an error path. A key that is not a property with a setter is refused before anything is
 * written. Without `options` the change is atomic and immediate.
 */
export declare function setProperties(engine: TransitionEngine, object: object, label: string, props: Record<string, unknown>, options: TransitionOptions | undefined): Transition;

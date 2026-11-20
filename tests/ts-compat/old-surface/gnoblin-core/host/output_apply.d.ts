import type { OutputConfig } from "gnoblin-core/output_config";
import type { ErrorCode } from "gnoblin-core/errors";
import type { Output } from "gnoblin-core/host/output";
import type { Operation } from "gnoblin-core/operation";
/** An output request awaiting its compositor reply and optional config notice path. */
export interface PendingApplication {
    /** Output receiving the request. */
    output: Output;
    /** The operation the caller holds; the compositor's answer settles it. */
    operation: Operation<Output>;
    /** The config path (`outputs.DP-1`) to name in a notice when the compositor refuses; null for a script's own
     * call. */
    path: string | null;
}
/**
 * The code of an `apply` the compositor refused, from the words it gave. The message of the failure stays those words,
 * which name the key (`transform: this compositor draws only normal`), so only the code is worked out here.
 */
export declare function applyFailureCode(refusal: string): ErrorCode;
/**
 * Sends one request for `output` to the compositor and returns the operation its answer settles: it succeeds with the
 * output, or fails with the compositor's own words, and the code `applyFailureCode` gives them. A request that cannot
 * be sent (a bad value, an output that is gone) throws instead.
 */
export declare function applyOutput(output: Output, options: Omit<OutputConfig, "mirrorOf"> & {
    mirrorOf?: Output | string | null;
}, target: string, path?: string | null): Operation<Output>;
/** Settles each answer's operation, tells the output's listeners, and reports a refused config entry. */
export declare function settleApplications(): void;

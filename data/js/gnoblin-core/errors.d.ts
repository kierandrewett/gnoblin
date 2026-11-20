/** Why a call failed. `docs/shell-api/errors.md` says what each code means. */
export type ErrorCode = "invalid_argument" | "not_found" | "already_exists" | "invalid_state" | "read_only" | "unsupported" | "denied" | "busy" | "cancelled" | "timed_out" | "unavailable" | "internal" | "config_rejected";
/**
 * What a failed operation on a native object throws. `code` says why and `target` names the object and member, as in
 * `Client#7.x`; the message begins with the target.
 */
export declare class GnoblinError extends Error {
    /** Stable reason code for this failure. */
    readonly code: ErrorCode;
    /** Object and member named by the failed call. */
    readonly target: string;
    constructor(code: ErrorCode, target: string, message: string);
}
/**
 * The `ErrorCode` for a word the host's tree gives when it refuses a write or a call: `unknown-node` is a node that is
 * gone, `read-only` a property no script may write, and so on. A word it does not know is a refusal, `denied`.
 */
export declare function errorCodeOfRefusal(refusal: string): ErrorCode;

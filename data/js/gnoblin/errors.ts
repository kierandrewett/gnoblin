// The one set of error codes of the script API, and the error a native failure throws.
//
// A call that fails later settles its `Operation` with `{ code, message }`. A call that cannot be made throws a
// `GnoblinError` with the same `code` and a `target` naming the object and member. A bad argument throws a plain
// `TypeError` or `RangeError` instead, because it is the script's mistake and no state is involved. The codes are
// snake_case and are the ones `docs/shell-api/errors.md` lists; the host's own words for a refused write
// (`unknown-node`, `read-only`) are translated here and nowhere else, so no script ever sees them as a `code`.

/** Why a call failed. `docs/shell-api/errors.md` says what each code means. */
export type ErrorCode =
    | "invalid_argument"
    | "not_found"
    | "already_exists"
    | "invalid_state"
    | "read_only"
    | "unsupported"
    | "denied"
    | "busy"
    | "cancelled"
    | "timed_out"
    | "unavailable"
    | "internal"
    | "config_rejected";

/**
 * What a failed operation on a native object throws. `code` says why and `target` names the object and member, as in
 * `Client#7.x`; the message begins with the target.
 */
export class GnoblinError extends Error {
    /** Stable reason code for this failure. */
    readonly code: ErrorCode;
    /** Object and member named by the failed call. */
    readonly target: string;

    constructor(code: ErrorCode, target: string, message: string) {
        super(`${target}: ${message}`);
        this.name = "GnoblinError";
        this.code = code;
        this.target = target;
    }
}

/**
 * The `ErrorCode` for a word the host's tree gives when it refuses a write or a call: `unknown-node` is a node that is
 * gone, `read-only` a property no script may write, and so on. A word it does not know is a refusal, `denied`.
 */
export function errorCodeOfRefusal(refusal: string): ErrorCode {
    switch (refusal) {
        case "unknown-node":
        case "unknown-property":
            return "not_found";
        case "unknown-method":
        case "not-animated":
            return "unsupported";
        case "read-only":
            return "read_only";
        case "invalid-value":
            return "invalid_argument";
        case "no-seat":
            return "unavailable";
        default:
            return "denied";
    }
}

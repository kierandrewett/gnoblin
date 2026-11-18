// What a failure of `libgnoblin-std` is called in the script API.
//
// A table function that fails throws an `Error` whose text starts with the symbol of the ABI code (`E_TYPE: ...`) and whose `code`
// is that code. The library writes the text the script API has always shown (`Client#3.opacity: must be a finite number`), so this
// is only the spelling: a wrong argument is a `TypeError`, anything else a `GnoblinError` of the matching code, whose target is the
// path the text starts with.

import { GnoblinError } from "gnoblin-core/errors";
import type { ErrorCode } from "gnoblin-core/errors";
import { ErrorKind } from "gnoblin-core/generated/abi";

/** A failure of a table function: an `Error` with the ABI code. */
type Failure = Error & { code: number };

/** Whether `error` is what a table function throws. */
function isFailure(error: unknown): error is Failure {
    return error instanceof Error && "code" in error && typeof error.code === "number";
}

/** Whether the library refused a value for being of the wrong kind for what it was written to. */
export function wrongKind(error: unknown): boolean {
    return isFailure(error) && error.code === ErrorKind.TYPE;
}

/** Whether the library cannot hold writes of the node: it is not in the tree the library reads, or its class is not in the core's catalog. */
export function unknownNode(error: unknown): boolean {
    return isFailure(error) && (error.code === ErrorKind.NO_SUCH_NODE || error.code === ErrorKind.NO_SUCH_MEMBER);
}

/** The script API's code for an ABI failure code. */
function codeOf(abi: number): ErrorCode {
    switch (abi) {
        case ErrorKind.NO_SUCH_NODE:
        case ErrorKind.NO_SUCH_MEMBER:
        case ErrorKind.STALE:
            return "not_found";
        case ErrorKind.BUSY:
            return "busy";
        case ErrorKind.RETIRED:
            return "invalid_state";
        case ErrorKind.UNSUPPORTED:
            return "unsupported";
        case ErrorKind.TIMEOUT:
            return "timed_out";
        case ErrorKind.ERR:
        case ErrorKind.PANIC:
            return "internal";
        default:
            return "denied";
    }
}

/**
 * The refusal word of the host (`unknown-node`, `invalid-value`) for a library failure: what a write through `host.set` answers, which
 * `errorCodeOfRefusal` turns into a code at the one place that does.
 */
export function refusalOf(error: unknown): string {
    if (!isFailure(error)) {
        throw error;
    }
    switch (error.code) {
        case ErrorKind.NO_SUCH_NODE:
            return "unknown-node";
        case ErrorKind.NO_SUCH_MEMBER:
            return "unknown-property";
        case ErrorKind.TYPE:
            return "invalid-value";
        default:
            return "denied";
    }
}

/**
 * The error a script sees for `error`, which a table function threw: a `TypeError` for a value of the wrong kind, a `GnoblinError`
 * for the rest, with `fallback` as its target when the text names none. Anything that is not a library failure is returned as it is.
 */
export function stdFailure(error: unknown, fallback: string): unknown {
    if (!isFailure(error)) {
        return error;
    }
    const text = error.message.replace(/^E_[A-Z_]+: /, "");
    if (error.code === ErrorKind.TYPE) {
        return new TypeError(text);
    }
    const named = /^([^\s:]+): ([\s\S]*)$/.exec(text);
    return named === null
        ? new GnoblinError(codeOf(error.code), fallback, text)
        : new GnoblinError(codeOf(error.code), named[1], named[2]);
}

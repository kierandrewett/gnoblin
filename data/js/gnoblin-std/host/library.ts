// How `gnoblin-std/host` reaches `libgnoblin-std`: the library is opened once for the generation, a node is named by its number,
// and a failure the library reports is thrown as the error a script has always been given for that call.

import { GnoblinError } from "gnoblin-core/errors";
import type { ErrorCode } from "gnoblin-core/errors";
import type { Client } from "gnoblin-core/host/client";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { Client as NativeClient, Output as NativeOutput, open } from "gnoblin-std/generated/classes";

/** The library's failure codes (`gnoblin-core-abi`'s `E_*`) a call here tells apart. */
export const FAILED = { type: -10, refused: -11, busy: -13, noSuchNode: -8, noSuchMember: -9, stale: -6 } as const;

/** Opens the library for this generation: the desktop it holds (workspaces, desks, the layout pass) starts here. */
export function openLibrary(): void {
    open();
}

/** The library's handle on a window. */
export function nativeClient(client: Client): NativeClient {
    return new NativeClient(stateOf(client).node);
}

/** The library's handle on an output. */
export function nativeOutput(output: Output): NativeOutput {
    return new NativeOutput(stateOf(output).node);
}

/** The code a failure of the library carries, or zero for any other error. */
export function failureCode(error: unknown): number {
    const code = typeof error === "object" && error !== null ? (error as { code?: unknown }).code : undefined;
    return typeof code === "number" ? code : 0;
}

/**
 * Runs a call of the library. A failure with a code in `known` is thrown as the `GnoblinError` that names `target`, with the
 * library's own words; any other failure goes on as it is.
 */
export function throughLibrary<T>(target: string, known: Readonly<Record<number, ErrorCode>>, call: () => T): T {
    try {
        return call();
    } catch (error) {
        const code = known[failureCode(error)];
        if (code === undefined) {
            throw error;
        }
        throw new GnoblinError(code, target, error instanceof Error ? error.message : String(error));
    }
}

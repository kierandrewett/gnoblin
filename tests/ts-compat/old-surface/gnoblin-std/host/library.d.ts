import type { ErrorCode } from "gnoblin-core/errors";
import type { Client } from "gnoblin-core/host/client";
import type { Output } from "gnoblin-core/host/output";
import { Client as NativeClient, Output as NativeOutput } from "gnoblin-std/emitted/classes";
/** The library's failure codes (`gnoblin-core-abi`'s `E_*`) a call here tells apart. */
export declare const FAILED: {
    readonly type: -10;
    readonly refused: -11;
    readonly busy: -13;
    readonly noSuchNode: -8;
    readonly noSuchMember: -9;
    readonly stale: -6;
};
/** Opens the library for this generation: the desktop it holds (workspaces, desks, the layout pass) starts here. */
export declare function openLibrary(): void;
/** The library's handle on a window. */
export declare function nativeClient(client: Client): NativeClient;
/** The library's handle on an output. */
export declare function nativeOutput(output: Output): NativeOutput;
/** The code a failure of the library carries, or zero for any other error. */
export declare function failureCode(error: unknown): number;
/**
 * Runs a call of the library. A failure with a code in `known` is thrown as the `GnoblinError` that names `target`, with the
 * library's own words; any other failure goes on as it is.
 */
export declare function throughLibrary<T>(target: string, known: Readonly<Record<number, ErrorCode>>, call: () => T): T;

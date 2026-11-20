import type { Display } from "gnoblin-std/display/display";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import type { DisplayOutputCollection, OutputView } from "gnoblin-std/display/types";
import type { ErrorCode } from "gnoblin-core/errors";
/** A `Collection<Output>` the manager keeps equal to the outputs a display presents. */
export interface OutputSet {
    /** Live collection of outputs assigned to the display. */
    readonly collection: DisplayOutputCollection;
    /** Replaces the collection members with `outputs`. */
    set(outputs: readonly OutputView[]): void;
}
/** The codes of the library's failures a call is ready for, and the error of the shell's each one is. */
export interface KnownFailures {
    /** `E_BUSY`: a name is taken. */
    busy?: ErrorCode;
    /** `E_REFUSED`: the library refuses the request as it stands. */
    refused?: ErrorCode;
    /** `E_STALE`: the object is not what the call needs. */
    stale?: ErrorCode;
}
/** What the manager needs of the shell: the objects of the library by their handles, its failures as the shell's errors, and the events of the collections. */
export interface DisplayBackend {
    /** The connected output of the shell for a compositor node, or null. */
    outputOf(node: number): OutputView | null;
    /** The workspace of the shell the library's handle names, or null. */
    workspaceOf(handle: number): object | null;
    /** The library's handle of a workspace of the shell. */
    workspaceHandle(workspace: object): number;
    /** The compositor node of an output of the shell. */
    outputNode(output: object): number;
    /** Runs a call of the library. A failure with a code in `known` is thrown as the shell's error of that code, with the library's words. */
    through<T>(target: string, known: KnownFailures, call: () => T): T;
    /** Whether `error` is the library's word that the object it was asked about is gone. */
    isStale(error: unknown): boolean;
    /** A new collection of outputs for one display. */
    outputSet(): OutputSet;
    /** The collections' events: the member joined or left. */
    listed(member: Display | OutputProfile): void;
    /** Reports that a display or output profile left its collection. */
    unlisted(member: Display | OutputProfile): void;
    /** Reports `text` once per generation. */
    say(text: string): void;
}

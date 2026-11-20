import type { DisplayOutputCollection } from "gnoblin-std/display/types";
import type { Display } from "gnoblin-std/display/display";
import type { ErrorCode } from "gnoblin-core/errors";
/** The facts a camera or display hands the manager. */
export interface DisplayHooks {
    /** Whether `value` is a workspace of the shell. */
    isWorkspace(value: unknown): boolean;
    /** Whether `value` is a connected output of the shell. */
    isOutput(value: unknown): boolean;
    /** The error type the shell throws, named by `code`, `target` and `message`. */
    error(code: ErrorCode, target: string, message: string): Error;
    /** What the display's viewport shows now: the workspace on its output, or the one it last showed or was given while it has none. */
    shown(display: Display): object | null;
    /**
     * Shows `workspace` on the display, or keeps it until the display has an output. Throws when it cannot be shown.
     */
    show(display: Display, workspace: object): void;
    /** The outputs a display presents now, as a `Collection<Output>`. */
    outputs(display: Display): DisplayOutputCollection;
    /** The library's id of a workspace of the shell. */
    workspaceHandle(workspace: object): number;
    /** The compositor's node of a connected output of the shell. */
    outputNode(output: object): number;
}
/** Gives the module the shell's side. The host module calls it once per generation, before any user code runs. */
export declare function attachDisplays(next: DisplayHooks): void;
/** Returns the attached shell hooks, or throws if the host has not attached them. */
export declare function shell(): DisplayHooks;
/** The next number for a display made without an `id`: `display-N` in creation order. */
export declare function nextDisplayNumber(): number;

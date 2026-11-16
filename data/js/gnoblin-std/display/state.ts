// The display module's shared state: the shell's side (`DisplayHooks`), attached once per generation, the counter of
// displays made without an `id`, and the number check the camera shares.

import type { DisplayOutputCollection } from "gnoblin-std/display/types";
import type { Display } from "gnoblin-std/display/display";
import type { ErrorCode } from "gnoblin-core/errors";

/** The facts a camera or display hands the manager. */
export interface DisplayHooks {
    /** Something a profile, display or viewport holds changed: the manager must reconcile. */
    changed(): void;
    /** Whether `value` is a workspace of the shell. */
    isWorkspace(value: unknown): boolean;
    /** Whether `value` is a connected output of the shell. */
    isOutput(value: unknown): boolean;
    /** The error type the shell throws, named by `code`, `target` and `message`. */
    error(code: ErrorCode, target: string, message: string): Error;
    /** What the display's viewport shows now, or undefined when the display has no output to show it on. */
    shown(display: Display): object | null | undefined;
    /**
     * Shows `workspace` on the display, or keeps it until the display has an output. Throws when it cannot be shown.
     */
    show(display: Display, workspace: object): void;
    /** The outputs a display presents now, as a `Collection<Output>`. */
    outputs(display: Display): DisplayOutputCollection;
}

let hooks: DisplayHooks | null = null;

/** Gives the module the shell's side. The host module calls it once per generation, before any user code runs. */
export function attachDisplays(next: DisplayHooks): void {
    hooks = next;
    counter = 0;
}

/** Returns the attached shell hooks, or throws if the host has not attached them. */
export function shell(): DisplayHooks {
    if (hooks === null) {
        throw new Error("displays need the shell host and cannot run without it");
    }
    return hooks;
}

/** Counts the displays made without an `id`. */
let counter = 0;

/** The next number for a display made without an `id`: `display-N` in creation order. */
export function nextDisplayNumber(): number {
    return ++counter;
}

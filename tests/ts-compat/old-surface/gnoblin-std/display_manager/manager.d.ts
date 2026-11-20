import { Display } from "gnoblin-std/display/display";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import type { DisplayOutputCollection } from "gnoblin-std/display/types";
import type { DisplayBackend } from "gnoblin-std/display_manager/backend";
/** Coordinates the shell's profiles, displays and outputs. */
export declare class DisplayManager {
    #private;
    /** Creates a manager for the host's profile and display collections. */
    constructor(backend: DisplayBackend, profiles: OutputProfile[], displays: Display[]);
    /** @throws GnoblinError `already_exists` when a profile with the same id is in the collection */
    addProfile(profile: OutputProfile): void;
    /** Stops managing the profile. The compositor keeps the arrangement it applied last. */
    removeProfile(profile: OutputProfile): boolean;
    /**
     * Adds a display the script made.
     *
     * @throws GnoblinError `busy` naming both displays when one of its outputs is presented by another, or
     *   `already_exists` when a display has its id
     */
    addDisplay(display: Display): void;
    /**
     * Removes a script display: its mirror and span end and each of its outputs gets a display of the shell's.
     * Returns false for a display of the shell's own, which stays, and for one that is not a member.
     */
    removeDisplay(display: Display): boolean;
    /**
     * Replaces `old` with `next` as one validated change. Displays that share an output with `next` leave, and `next`
     * takes `old`'s place. Nothing changes if validation fails.
     *
     * @throws GnoblinError `invalid_argument` when `old` is not in the collection, or `already_exists` when `next` is
     *   present or has an id that remains in use
     */
    replaceDisplay(old: Display, next: Display): void;
    /** Removes every display of the script. */
    clearDisplays(): void;
    /**
     * The display under `point` in the compositor's space, else the primary output's; null when no display has an
     * output.
     */
    displayAt(point: {
        x: number;
        y: number;
    }): Display | null;
    /** What the display's viewport shows: the workspace on its output, else the one it last showed or was given. */
    shown(display: Display): object | null;
    /**
     * Shows `workspace` on the display, or keeps it for when the display has an output.
     *
     * @throws GnoblinError as `gnoblin.workspaces.show` does, when the workspace is not in `gnoblin.workspaces`
     */
    assignContent(display: Display, workspace: object): void;
    /** The `Collection<Output>` of the outputs the display presents. */
    outputsOf(display: Display): DisplayOutputCollection;
    /**
     * What a transition callback can change that is not a tree write, to put back when it fails: the displays' wanted
     * workspaces (the library keeps them), cameras and colours.
     */
    snapshot(): () => void;
    /** Called each layout pass: catches up with the library, then reports what the cameras ask of the renderer. */
    frame(): void;
    /** Has the library reconcile the profiles and displays now, and catches up with what it did. */
    reflow(): void;
    /** Makes `gnoblin.displays` and the outputs each display presents equal to the library's. */
    sync(): void;
}

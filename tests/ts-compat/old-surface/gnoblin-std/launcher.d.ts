import type { UiKeyEvent } from "gnoblin-std/ui/events";
import { Entry } from "gnoblin-std/widgets";
import type { EntryOptions } from "gnoblin-std/widgets";
/** What `openLauncher` takes: any `Entry` option, and the function that runs the command. */
export interface LauncherOptions extends EntryOptions {
    /** Starts the typed command. Default: `gnoblin.spawn`. */
    spawn?: (command: string) => void;
}
/** A command entry, 600 by 48 pixels. Submitting it runs the text and destroys it. A blank text runs nothing. */
export declare class Launcher extends Entry {
    #private;
    constructor(options?: LauncherOptions);
    /** Lets `Entry` handle typing and submission, then closes the launcher on Escape. */
    onKey(event: UiKeyEvent): void;
}
/**
 * Opens the launcher in the middle of the screen's width, over the windows, and gives it the keyboard. Only one
 * launcher lives at a time: a second call focuses the open one and returns it.
 *
 *     openLauncher();
 */
export declare function openLauncher(options?: LauncherOptions): Launcher;

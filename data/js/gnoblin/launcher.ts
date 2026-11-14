// The stock launcher: Super+Space opens a command entry on the screen. Return starts the command and closes the
// entry; Escape closes it. A config that gives `launcher` a command line runs that program on Super+Space instead and
// opens no entry.
//
// It owns the entry and what happens when it is submitted. It does not own the Super+Space binding (read
// `gnoblin/bindings`). Read `gnoblin/widgets` first for `Entry`, which does all the typing.

import { uiScreen } from "gnoblin/ui";
import type { UiKeyEvent } from "gnoblin/ui";
import { Entry } from "gnoblin/widgets";
import type { EntryOptions } from "gnoblin/widgets";

/** The host module's global, which is there before any stock module runs. */
declare const gnoblin: { spawn(command: string): void };

/** The entry's width in pixels. */
const LAUNCHER_WIDTH = 600;
/** The entry's height in pixels. */
const LAUNCHER_HEIGHT = 48;
/** The entry's corner radius in pixels. */
const LAUNCHER_RADIUS = 12;
/** The entry's background colour. */
const LAUNCHER_FILL = "#313244";
/** The entry's text size in pixels. */
const LAUNCHER_FONT_SIZE = 18;
/** The distance from the top of the screen to the entry, in pixels. */
const LAUNCHER_TOP = 200;
/** The width in pixels assumed for the screen while the tree has none. */
const FALLBACK_SCREEN_WIDTH = 1200;

/** What `openLauncher` takes: any `Entry` option, and the function that runs the command. */
export interface LauncherOptions extends EntryOptions {
    /** Starts the typed command. Default: `gnoblin.spawn`. */
    spawn?: (command: string) => void;
}

/** A command entry, 600 by 48 pixels. Submitting it runs the text and destroys it. A blank text runs nothing. */
export class Launcher extends Entry {
    readonly #spawn: (command: string) => void;

    constructor(options: LauncherOptions = {}) {
        super({
            width: LAUNCHER_WIDTH,
            height: LAUNCHER_HEIGHT,
            radius: LAUNCHER_RADIUS,
            fill: LAUNCHER_FILL,
            size: LAUNCHER_FONT_SIZE,
            ...options,
        });
        this.#spawn = options.spawn ?? ((command) => gnoblin.spawn(command));
        this.onSubmit = (text) => {
            if (text.trim() !== "") {
                this.#spawn(text);
            }
            this.destroy();
        };
    }

    // `Entry` types first and turns Return into its submit. Escape is the launcher's own key.
    override onKey(event: UiKeyEvent): void {
        super.onKey(event);
        if (event.pressed && event.key === "Escape") {
            this.destroy();
            event.stop();
        }
    }
}

let current: Launcher | null = null;

/**
 * Opens the launcher in the middle of the screen's width, over the windows, and gives it the keyboard. Only one
 * launcher lives at a time: a second call focuses the open one and returns it.
 */
export function openLauncher(options: LauncherOptions = {}): Launcher {
    if (current !== null && current.alive) {
        current.focus();
        return current;
    }
    const screen = uiScreen();
    const x = Math.floor((screen?.width ?? FALLBACK_SCREEN_WIDTH) / 2) - LAUNCHER_WIDTH / 2;
    const launcher = new Launcher({ x, y: LAUNCHER_TOP, ...options });
    launcher.mountTop();
    launcher.focus();
    current = launcher;
    return launcher;
}
